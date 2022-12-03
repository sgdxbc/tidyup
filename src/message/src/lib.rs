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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Reply {
    // should not use default
    pub request_number: u32,
    // should not use default
    pub result: Box<[u8]>,
    pub replica_id: u8,
    pub view_number: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signed<M> {
    pub inner: M,
    pub signature: secp256k1::ecdsa::Signature,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    pub n: usize, // always == replica.len()
    pub f: usize,
    pub replica: Box<[SocketAddr]>,
    pub public_keys: Box<[secp256k1::PublicKey]>,
    pub secret_keys: Box<[secp256k1::SecretKey]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub config: TransportConfig,
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
    pub n_thread: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCommand {
    // total client = n_client * n_thread
    pub n_client: NonZeroUsize,
    pub n_thread: NonZeroUsize,
    pub ip: IpAddr,
    pub n_report: NonZeroUsize,
}

pub mod pbft;
