use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::{
    crypto::{PublicKey, QuorumSigned, Signed},
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
    pub certified: QuorumSigned<Digest>,
    pub requests: Vec<Request>,
    pub parent: Digest,
    pub replica_id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub digest: Signed<Digest>,
    pub replica_id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    Generic(Signed<Generic>),
    Vote(Vote),
}

impl Request {
    pub fn remote(&self) -> SocketAddr {
        self.client_id.0
    }
}

pub const GENESIS: Digest = [0; 32];

impl Generic {
    pub fn verify_certificate(self, f: usize, public_keys: &[PublicKey]) -> Option<Self> {
        if self.certified.inner == GENESIS {
            Some(self)
        } else {
            self.certified
                .verify(f, public_keys)
                .map(|certified| Self { certified, ..self })
        }
    }
}
