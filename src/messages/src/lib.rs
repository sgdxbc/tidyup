pub mod crypto;
pub mod hotstuff;
pub mod unreplicated;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ClientId(std::net::SocketAddr, u8);
pub type RequestNumber = u32;

pub type OpNumber = u32;
pub type ViewNumber = u8;
pub type ReplicaId = u8;

pub type Digest = [u8; 32];

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

pub fn serialize(message: &impl serde::Serialize) -> Vec<u8> {
    use bincode::Options;
    bincode::options().serialize(&message).unwrap()
}

pub fn deserialize<M>(message: &[u8]) -> M
where
    M: serde::de::DeserializeOwned,
{
    use bincode::Options;
    bincode::options()
        .allow_trailing_bytes()
        .deserialize(message)
        .unwrap()
}

pub fn deserialize_from<M>(reader: impl std::io::Read) -> M
where
    M: serde::de::DeserializeOwned,
{
    use bincode::Options;
    bincode::options()
        .allow_trailing_bytes()
        .deserialize_from(reader)
        .unwrap()
}

pub fn digest(message: &impl serde::Serialize) -> Digest {
    *secp256k1::Message::from_hashed_data::<secp256k1::hashes::sha256::Hash>(&serialize(message))
        .as_ref()
}
