use serde::{Deserialize, Serialize};

use crate::{Request, Signed};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToReplica {
    Request(Request),
    PrePrepare(Signed<PrePrepare>, Box<[Request]>),
    Prepare(Signed<Prepare>),
    Commit(Signed<Commit>),
    // checkpoint, view change, ...
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrePrepare {
    pub view_number: u8,
    pub op_number: u32,
    pub request_digest: [u8; 32],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prepare {
    pub view_number: u8,
    pub op_number: u32,
    pub request_digest: [u8; 32],
    pub replica_id: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub view_number: u8,
    pub op_number: u32,
    pub request_digest: [u8; 32],
    pub replica_id: u8,
}
