use std::{
    cmp::Reverse,
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

pub use message::TransportConfig;

use crate::App;

pub trait State {
    fn poll(&mut self) -> bool;
}

pub trait SharedState {
    fn shared_poll(&self) -> bool;
}

impl<T: SharedState> State for T {
    fn poll(&mut self) -> bool {
        self.shared_poll()
    }
}

pub trait Deploy {
    fn deploy(&mut self, state: impl State + Send + 'static);
    fn deploy_shared(&mut self, shared_state: impl SharedState + Send + Sync + 'static) {
        self.deploy(shared_state)
    }
}

pub trait ClientState
where
    Self: State,
{
    fn invoke(&mut self, op: Box<[u8]>);
    fn take_result(&mut self) -> Option<Box<[u8]>>;
    fn poll_at(&self) -> OptionInstant;
}

pub type OptionInstant = Reverse<Option<Reverse<Instant>>>;

pub struct ReplicaCommon {
    pub id: usize,
    pub config: Arc<TransportConfig>,
    pub app: App,
    pub tx: TxChannel,
    pub rx: RxChannel,
    pub clock: Clock,
    pub n_effect: usize,
}

pub struct ClientCommon {
    pub id: u16,
    pub config: Arc<TransportConfig>,
    pub tx: TxChannel,
    pub rx: RxChannel,
    pub rx_addr: SocketAddr,
    pub clock: Clock,
}

pub enum TxChannel {
    Udp(UdpSocket),
    Simulated(()), //
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
    pub fn send_to(&self, payload: &[u8], dest: SocketAddr) -> usize {
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
    pub fn now(&self) -> OptionInstant {
        match self {
            Self::Real => Reverse(Some(Reverse(Instant::now()))),
            Self::Simulated(_) => todo!(),
        }
    }

    pub fn after(&self, duration: Duration) -> OptionInstant {
        match self {
            Self::Real => Reverse(Some(Reverse(Instant::now() + duration))),
            Self::Simulated(_) => todo!(),
        }
    }
}
