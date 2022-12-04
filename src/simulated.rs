use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender};
use message::TransportConfig;
use secp256k1::{Secp256k1, SecretKey};

use crate::{
    core::{ClientCommon, Clock, ReplicaCommon, RxChannel, TxChannel},
    App,
};

pub struct Network {
    instant: Instant,
    elapsed: Arc<Mutex<Duration>>,
    routes: HashMap<SocketAddr, Sender<(SocketAddr, Vec<u8>)>>,
    replica_rx: Box<[Receiver<(SocketAddr, Vec<u8>)>]>,
    config: Arc<TransportConfig>,
    n_client: usize,
    tx: Channel<(SocketAddr, SocketAddr, Vec<u8>)>,
}
type Channel<T> = (Sender<T>, Receiver<T>);

impl Network {
    pub fn new(n: usize, f: usize) -> Self {
        let secret_keys = (0..n)
            .map(|i| SecretKey::from_slice(&[&[0x7f; 31][..], &[i as _][..]].concat()).unwrap())
            .collect::<Box<_>>();
        let secp = Secp256k1::new();
        let public_keys = secret_keys.iter().map(|k| k.public_key(&secp)).collect();

        let mut replica = Vec::new();
        let mut replica_rx = Vec::new();
        let mut routes = HashMap::new();
        for i in 0..n {
            let addr = SocketAddr::from(([10, 0, 0, i as _], 8000));
            replica.push(addr);
            let rx = crossbeam_channel::unbounded();
            routes.insert(addr, rx.0);
            replica_rx.push(rx.1);
        }

        Self {
            instant: Instant::now(),
            elapsed: Default::default(),
            routes,
            replica_rx: replica_rx.into_boxed_slice(),
            config: Arc::new(TransportConfig {
                n,
                f,
                replica: replica.into_boxed_slice(),
                public_keys,
                secret_keys,
            }),
            n_client: 0,
            tx: crossbeam_channel::unbounded(),
        }
    }

    pub fn elapse(&mut self, duration: Duration) {
        *self.elapsed.try_lock().unwrap() += duration;
    }

    pub fn replica(&self, i: usize, app: App) -> ReplicaCommon {
        ReplicaCommon {
            id: i,
            config: self.config.clone(),
            app,
            tx: TxChannel::Simulated(self.tx.0.clone(), self.config.replica[i]),
            rx: RxChannel::Simulated(self.replica_rx[i].clone()),
            clock: Clock::Simulated(self.instant, self.elapsed.clone()),
        }
    }

    pub fn insert_client(&mut self) -> ClientCommon {
        self.n_client += 1;
        let rx_addr = SocketAddr::from(([10, 0, 0, 101], self.n_client as u16));
        let rx = crossbeam_channel::unbounded();
        self.routes.insert(rx_addr, rx.0);
        ClientCommon {
            id: self.n_client as _,
            config: self.config.clone(),
            tx: TxChannel::Simulated(self.tx.0.clone(), rx_addr),
            rx: RxChannel::Simulated(rx.1),
            rx_addr,
            clock: Clock::Simulated(self.instant, self.elapsed.clone()),
        }
    }

    pub fn poll(&self, mut filter: impl FnMut(SocketAddr, SocketAddr, &[u8]) -> bool) -> bool {
        let Ok((source, dest, buffer)) = self.tx.1.try_recv() else {
            return false;
        };
        if filter(source, dest, &buffer) {
            self.routes[&dest].try_send((source, buffer)).unwrap();
        }
        true
    }
}
