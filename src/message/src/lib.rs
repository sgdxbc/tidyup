use std::{
    net::{IpAddr, SocketAddr},
    num::NonZeroUsize,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub client_id: u16,
    pub request_number: u32,
    pub op: Box<[u8]>,
    pub addr: SocketAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub request_number: u32,
    pub result: Box<[u8]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command<C> {
    pub config: C,
    pub app: AppMode,
    pub protocol: ProtocolMode,
    pub replica: Option<ReplicaCommand>,
    pub client: Option<ClientCommand>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum AppMode {
    Null,
    //
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProtocolMode {
    Unreplicated,
    // HotStuff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicaCommand {
    pub id: usize,
    pub n_runner_thread: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCommand {
    // total client = n_client * n_thread
    pub n_client: NonZeroUsize,
    pub n_thread: NonZeroUsize,
    pub ip: IpAddr,
    pub n_report: NonZeroUsize,
}
