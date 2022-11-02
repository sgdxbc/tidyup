use secp256k1::{Message, Secp256k1, SignOnly, VerifyOnly};
use serde::{Deserialize, Serialize};

use crate::{digest, ReplicaId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Signature {
    None,
    Secp256k1(secp256k1::ecdsa::Signature),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuorumSignature {
    Vec(Vec<(ReplicaId, Signature)>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signed<M> {
    pub inner: M,
    pub signature: Signature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuorumSigned<M> {
    pub inner: M,
    pub signature: QuorumSignature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PublicKey {
    Secp256k1(secp256k1::PublicKey),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SecretKey {
    Disabled,
    Secp256k1(secp256k1::SecretKey),
}

impl PublicKey {
    pub fn new_secp256k1(secret_key: &SecretKey) -> Self {
        if let SecretKey::Secp256k1(secret_key) = secret_key {
            Self::Secp256k1(secp256k1::PublicKey::from_secret_key(
                &Secp256k1::new(),
                secret_key,
            ))
        } else {
            panic!()
        }
    }
}

impl SecretKey {
    pub fn new_secp256k1(id: ReplicaId) -> Self {
        Self::Secp256k1(
            secp256k1::SecretKey::from_slice(&[&[0xf0; 31][..], &[id]].concat()).unwrap(),
        )
    }
}

impl<M> Signed<M> {
    pub fn sign(inner: M, secret_key: &SecretKey) -> Self
    where
        M: Serialize,
    {
        thread_local!(static SECP256K1: Secp256k1<SignOnly> = Secp256k1::signing_only());

        let message = digest(&inner);
        match secret_key {
            SecretKey::Disabled => panic!(),

            SecretKey::Secp256k1(secret_key) => {
                let signature = SECP256K1.with(|secp| {
                    secp.sign_ecdsa(&Message::from_slice(&message).unwrap(), secret_key)
                });
                Self {
                    inner,
                    signature: Signature::Secp256k1(signature),
                }
            }
        }
    }

    pub fn verify(self, public_key: &PublicKey) -> Option<Self>
    where
        M: Serialize,
    {
        thread_local!(static SECP256K1: Secp256k1<VerifyOnly> = Secp256k1::verification_only());

        let message = digest(&self.inner);
        match (public_key, &self.signature) {
            (PublicKey::Secp256k1(public_key), Signature::Secp256k1(signature)) => SECP256K1
                .with(|secp| {
                    secp.verify_ecdsa(
                        &Message::from_slice(&message).unwrap(),
                        signature,
                        public_key,
                    )
                })
                .ok()
                .map(|()| self),
            _ => None,
        }
    }
}

impl<M> QuorumSigned<M> {
    pub fn verify(self, f: usize, public_keys: &[PublicKey]) -> Option<Self>
    where
        M: Serialize,
    {
        match self.signature {
            QuorumSignature::Vec(quorum) => {
                if quorum.len() < 2 * f + 1 {
                    return None;
                }
                let mut verified_quorum = Vec::new();
                let mut inner = self.inner;
                for (id, signature) in quorum {
                    let verified =
                        (Signed { inner, signature }).verify(&public_keys[id as usize])?;
                    inner = verified.inner;
                    verified_quorum.push((id, verified.signature));
                }
                Some(QuorumSigned {
                    inner,
                    signature: QuorumSignature::Vec(verified_quorum),
                })
            }
        }
    }
}
