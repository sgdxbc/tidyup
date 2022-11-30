use std::{net::SocketAddr, sync::Arc, time::Instant};

use crate::transport::{Clock, Config, RxChannel, TxChannel};

pub trait State {
    fn poll(&mut self) -> bool;
}

pub trait Deploy {
    fn deploy(&mut self, state: impl State + Send + 'static);
}

pub trait ClientState
where
    Self: State,
{
    fn invoke(&mut self, op: Box<[u8]>);
    fn take_result(&mut self) -> Option<Box<[u8]>>;
    fn poll_at(&self) -> Option<Instant>;
}

pub struct ReplicaCommon {
    pub tx: TxChannel,
    pub rx: RxChannel,
    pub n_effect: usize,
    pub id: usize,
    pub app: (), //
    pub clock: Clock,
}

pub struct ClientCommon {
    pub id: u16,
    pub config: Arc<Config>,
    pub tx: TxChannel,
    pub rx: RxChannel,
    pub rx_addr: SocketAddr,
    pub clock: Clock,
}
