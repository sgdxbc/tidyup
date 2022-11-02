use std::net::SocketAddr;

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
    pub certified: Digest,
    pub certificate: Vec<(ReplicaId, Signature)>,
    pub requests: Vec<Request>,
    pub parent: Digest,
    pub replica_id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub digest: Digest,
    pub replica_id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    Generic(Signed<Generic>),
    Vote(Signed<Vote>),
}

impl Request {
    pub fn remote(&self) -> SocketAddr {
        self.client_id.0
    }
}

pub const GENESIS: Digest = [0; 32];

impl Generic {
    pub fn verify_certificate(self, f: usize, public_keys: &[PublicKey]) -> Option<Self> {
        if self.certified == GENESIS {
            return Some(self);
        }

        if self.certificate.len() < 2 * f + 1 {
            return None;
        }

        for (id, signature) in &self.certificate {
            (Signed {
                inner: Vote {
                    digest: self.certified,
                    replica_id: *id,
                },
                signature: signature.clone(),
            })
            .verify(&public_keys[*id as usize])?;
        }
        Some(self)
    }
}
