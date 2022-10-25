use bincode::Options;

pub mod unreplicated;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ClientId(std::net::SocketAddr, u8);
pub type RequestNumber = u32;

pub type OpNumber = u32;
pub type ViewNumber = u8;
pub type ReplicaId = u8;

impl ClientId {
    pub fn likely_unique(addr: std::net::SocketAddr) -> Self {
        Self(
            addr,
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .unwrap()
                .as_millis()
                .to_le_bytes()[0],
        )
    }
}

pub fn deserialize<M>(message: &[u8]) -> M
where
    M: serde::de::DeserializeOwned,
{
    bincode::options()
        .allow_trailing_bytes()
        .deserialize(message)
        .unwrap()
}
