use std::{
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    sync::{mpsc, Arc},
    time::Instant,
};

use crate::App;

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
    pub app: App,
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

pub struct Config {
    pub f: usize,
    pub replica: Box<[SocketAddr]>,
}
/// A serializable version of `Config` so we don't need to derive `Serialize`
/// for `Config` and add serde into dependencies.
pub type Config_ = (usize, Box<[SocketAddr]>);
impl From<Config> for Config_ {
    fn from(config: Config) -> Self {
        (config.f, config.replica)
    }
}
impl From<Config_> for Config {
    fn from(config: Config_) -> Self {
        Self {
            f: config.0,
            replica: config.1,
        }
    }
}

pub enum TxChannel {
    Udp(UdpSocket),
    Simulated(mpsc::Sender<()>), //
}

impl Clone for TxChannel {
    fn clone(&self) -> Self {
        match self {
            Self::Udp(socket) => Self::Udp(socket.try_clone().unwrap()),
            Self::Simulated(tx) => Self::Simulated(tx.clone()),
        }
    }
}

impl TxChannel {
    pub fn send_to(&mut self, payload: &[u8], dest: SocketAddr) -> usize {
        match self {
            Self::Udp(socket) => socket.send_to(payload, dest).unwrap(),
            Self::Simulated(_) => todo!(),
        }
    }
}

pub enum RxChannel {
    Udp(UdpSocket),
    Simulated(mpsc::Receiver<()>),
}

impl RxChannel {
    pub fn receive_from(&mut self, payload: &mut [u8]) -> Option<(usize, SocketAddr)> {
        match self {
            Self::Udp(socket) => socket.recv_from(payload).map_or_else(
                |err| {
                    assert!(err.kind() == ErrorKind::WouldBlock);
                    None
                },
                Some,
            ),
            Self::Simulated(_) => todo!(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Clock {
    Real,
    Simulated(()),
}

impl Clock {
    pub fn now(&self) -> Instant {
        match self {
            Self::Real => Instant::now(),
            Self::Simulated(_) => todo!(),
        }
    }
}
