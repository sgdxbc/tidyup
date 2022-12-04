use std::{
    cmp::Reverse,
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    sync::Arc,
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
    #[cfg(test)]
    Simulated(
        crossbeam_channel::Sender<(SocketAddr, SocketAddr, Vec<u8>)>,
        SocketAddr,
    ),
}

impl Clone for TxChannel {
    fn clone(&self) -> Self {
        match self {
            Self::Udp(socket) => Self::Udp(socket.try_clone().unwrap()),
            #[cfg(test)]
            Self::Simulated(tx, addr) => Self::Simulated(tx.clone(), *addr),
        }
    }
}

impl TxChannel {
    pub fn send_to(&self, payload: &[u8], dest: SocketAddr) -> usize {
        match self {
            Self::Udp(socket) => socket.send_to(payload, dest).unwrap(),
            #[cfg(test)]
            Self::Simulated(sender, addr) => {
                sender.try_send((*addr, dest, payload.to_vec())).unwrap();
                payload.len()
            }
        }
    }
}

pub enum RxChannel {
    Udp(UdpSocket),
    #[cfg(test)]
    Simulated(crossbeam_channel::Receiver<(SocketAddr, Vec<u8>)>),
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
            #[cfg(test)]
            Self::Simulated(receiver) => receiver.try_recv().map_or_else(
                |err| {
                    assert!(err.is_empty());
                    None
                },
                |(remote, buffer)| {
                    payload[..buffer.len()].copy_from_slice(&buffer);
                    Some((buffer.len(), remote))
                },
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Clock {
    Real,
    #[cfg(test)]
    Simulated(Instant, Arc<std::sync::Mutex<Duration>>),
}

impl Clock {
    pub fn now(&self) -> OptionInstant {
        match self {
            Self::Real => Reverse(Some(Reverse(Instant::now()))),
            #[cfg(test)]
            Self::Simulated(instant, elapsed) => {
                Reverse(Some(Reverse(*instant + *elapsed.try_lock().unwrap())))
            }
        }
    }

    pub fn after(&self, duration: Duration) -> OptionInstant {
        let Reverse(Some(Reverse(now))) = self.now() else {
                unreachable!()
            };
        Reverse(Some(Reverse(now + duration)))
    }
}
