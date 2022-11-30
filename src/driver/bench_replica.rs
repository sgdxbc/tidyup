use std::{
    mem::take,
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{spawn, JoinHandle},
};

use crate::{
    misc::bind_core,
    receiver::{Deploy, ReplicaArgs},
    transport::{Config, RxChannel, TxChannel},
    Receiver,
};

#[derive(Default)]
pub struct Driver {
    threads: Vec<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl Deploy for Driver {
    fn deploy(&mut self, mut receiver: impl Receiver + Send + 'static) {
        let shutdown = self.shutdown.clone();
        self.threads.push(spawn(move || {
            bind_core();
            while !shutdown.load(Ordering::SeqCst) {
                receiver.poll();
            }
        }))
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        if !self.shutdown.swap(true, Ordering::SeqCst) {
            println!("! Implicitly shutdown replica threads on driver dropping");
        }
        for thread in take(&mut self.threads) {
            thread.join().unwrap()
        }
    }
}

impl Driver {
    pub fn args(config: Arc<Config>, i: usize, app: (), n_effect: usize) -> ReplicaArgs {
        let socket = UdpSocket::bind(config.replica[i]).unwrap();
        socket.set_nonblocking(true).unwrap();
        ReplicaArgs {
            tx: TxChannel::Udp(socket.try_clone().unwrap()),
            rx: RxChannel::Udp(socket),
            n_effect,
            id: i,
            app,
        }
    }
}
