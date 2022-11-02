use blsttc::serde_impl::SerdeSecret;
use rand::thread_rng;
use secp256k1::{Message, Secp256k1, SignOnly, VerifyOnly};
use serde::{Deserialize, Serialize};

use crate::{digest, serialize, ReplicaId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Signature {
    None,
    Secp256k1(secp256k1::ecdsa::Signature),
    Blsttc(blsttc::SignatureShare),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuorumSignature {
    Vec(Vec<(ReplicaId, Signature)>),
    Blsttc(blsttc::Signature),
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
    Blsttc(blsttc::PublicKeyShare),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QuorumKey {
    Vec(Vec<PublicKey>),
    Blsttc(blsttc::PublicKeySet),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SecretKey {
    Disabled,
    Secp256k1(secp256k1::SecretKey),
    Blsttc(SerdeSecret<blsttc::SecretKeyShare>),
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

    pub fn new_blsttc(secret_key: &SecretKey) -> Self {
        if let SecretKey::Blsttc(secret_key) = secret_key {
            Self::Blsttc(secret_key.public_key_share())
        } else {
            panic!()
        }
    }
}

impl SecretKey {
    pub fn generate_secp256k1() -> Self {
        Self::Secp256k1(Secp256k1::new().generate_keypair(&mut thread_rng()).0)
    }

    pub fn generate_blsttc(f: usize) -> (Vec<SecretKey>, QuorumKey) {
        let key_set = blsttc::SecretKeySet::random(2 * f, &mut thread_rng());
        (
            (0..3 * f + 1)
                .map(|i| SecretKey::Blsttc(SerdeSecret(key_set.secret_key_share(i))))
                .collect(),
            QuorumKey::Blsttc(key_set.public_keys()),
        )
    }
}

impl<M> Signed<M> {
    pub fn sign(inner: M, secret_key: &SecretKey) -> Self
    where
        M: Serialize,
    {
        thread_local!(static SECP256K1: Secp256k1<SignOnly> = Secp256k1::signing_only());

        match secret_key {
            SecretKey::Disabled => panic!(),
            SecretKey::Secp256k1(secret_key) => {
                let message = digest(&inner);
                let signature = SECP256K1.with(|secp| {
                    secp.sign_ecdsa(&Message::from_slice(&message).unwrap(), secret_key)
                });
                Self {
                    inner,
                    signature: Signature::Secp256k1(signature),
                }
            }
            SecretKey::Blsttc(secret_key) => Self {
                signature: Signature::Blsttc(secret_key.sign(serialize(&inner))),
                inner,
            },
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
            // blsttc
            _ => None,
        }
    }
}

impl<M> QuorumSigned<M> {
    pub fn new(
        inner: M,
        signatures: impl Iterator<Item = (ReplicaId, Signature)>,
        quorum_key: &QuorumKey,
    ) -> Self {
        match quorum_key {
            QuorumKey::Vec(_) => Self {
                inner,
                signature: QuorumSignature::Vec(signatures.collect()),
            },
            QuorumKey::Blsttc(quorum_key) => {
                let signatures = signatures.map(|(i, signature)| {
                    if let Signature::Blsttc(signature) = signature {
                        (i as u64, signature)
                    } else {
                        panic!()
                    }
                });
                Self {
                    inner,
                    signature: QuorumSignature::Blsttc(
                        quorum_key.combine_signatures(signatures).unwrap(),
                    ),
                }
            }
        }
    }

    pub fn verify(self, f: usize, quorum_key: &QuorumKey) -> Option<Self>
    where
        M: Serialize,
    {
        match (self.signature, quorum_key) {
            (QuorumSignature::Vec(quorum), QuorumKey::Vec(public_keys)) => {
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
            (QuorumSignature::Blsttc(signature), QuorumKey::Blsttc(public_key)) => {
                if public_key
                    .public_key()
                    .verify(&signature, serialize(&self.inner))
                {
                    Some(Self {
                        signature: QuorumSignature::Blsttc(signature),
                        ..self
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}
