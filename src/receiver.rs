use std::time::Instant;

use crate::transport::{RxChannel, TxChannel};

pub trait Receiver {
    fn poll(&mut self) -> bool;
}

pub trait Client
where
    Self: Receiver,
{
    fn invoke(&mut self, op: Box<[u8]>);
    fn take_result(&mut self) -> Option<Box<[u8]>>;
    fn poll_at(&self) -> Option<Instant>;
}

pub struct ReplicaArgs {
    pub tx: TxChannel,
    pub rx: RxChannel,
    pub n_effect: usize,
    pub id: usize,
    pub app: (), //
}

pub trait Deploy {
    fn deploy(&mut self, receiver: impl Receiver + Send + 'static);
}
