use serde::{Deserialize, Serialize};

use crate::{
    crypto::{PublicKey, Signature, Signed},
    ClientId, Digest, ReplicaId, RequestNumber,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub client_id: ClientId,
    pub request_number: RequestNumber,
    pub op: Box<[u8]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub request_number: RequestNumber,
    pub result: Box<[u8]>,
    pub replica_id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Generic {
    certified: Digest,
    certificate: Vec<(ReplicaId, Signature)>,
    requests: Vec<Request>,
    parent: Digest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    digest: Digest,
    replica_id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    Generic(Signed<Generic>),
    Vote(Signed<Vote>),
}

impl Generic {
    pub fn verify_certificate(self, f: usize, public_keys: &[PublicKey]) -> Option<Self> {
        if self.certificate.len() < 2 * f + 1 {
            return None;
        }

        for (id, signature) in &self.certificate {
            if (Signed {
                inner: Vote {
                    digest: self.certified,
                    replica_id: *id,
                },
                signature: signature.clone(),
            })
            .verify(&public_keys[*id as usize])
            .is_none()
            {
                return None;
            }
        }
        Some(self)
    }
}
