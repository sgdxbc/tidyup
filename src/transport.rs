use std::{
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    sync::mpsc,
    time::Instant,
};

pub struct Config {
    pub f: usize,
    pub replica: Box<[SocketAddr]>,
}
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
