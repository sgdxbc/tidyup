use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

use crate::{ClientId, RequestNumber};

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
}

impl Request {
    pub fn remote(&self) -> SocketAddr {
        self.client_id.0
    }
}
