use std::{
    net::UdpSocket,
    sync::{mpsc, Arc, Mutex},
};

use nix::unistd::gettid;

use crate::{
    misc::bind_core,
    transport::{Config, RxChannel, TxChannel},
    unreplicated, EffectRunner, Receiver,
};

pub struct Driver<T> {
    replica: T,
}

impl Driver<unreplicated::Replica> {
    pub fn new(config: Arc<Config>, i: usize, n_runner_thread: usize) -> Self {
        let (tx, rx) = mpsc::sync_channel(64);
        let socket = UdpSocket::bind(config.replica[i]).unwrap();
        socket.set_nonblocking(true).unwrap();
        let rx_channel = Arc::new(Mutex::new(RxChannel::Udp(socket.try_clone().unwrap())));
        let new_context = || unreplicated::ReplicaEffect {
            tx: TxChannel::Udp(socket.try_clone().unwrap()),
            rx: rx_channel.clone(),
            buffer: [0; 1500],
            message_channel: tx.clone(),
        };
        let runner = if n_runner_thread != 0 {
            EffectRunner::new((0..n_runner_thread).map(|_| new_context()))
        } else {
            EffectRunner::Inline(new_context())
        };
        Self {
            replica: unreplicated::Replica::new(config, runner, rx),
        }
    }
}

impl<T> Driver<T> {
    pub fn run(&mut self)
    where
        T: Receiver,
    {
        let core_id = bind_core();
        println!("* Replica start: Core {core_id} Thread {}", gettid());
        loop {
            self.replica.poll();
        }
    }
}
