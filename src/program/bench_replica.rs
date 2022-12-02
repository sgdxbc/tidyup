use std::{
    mem::take,
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{spawn, JoinHandle},
};

use nix::sys::signal::{signal, SigHandler, Signal::SIGINT};

use crate::{
    core::{Clock, Deploy, ReplicaCommon, RxChannel, TransportConfig, TxChannel},
    misc::bind_core,
    App, State,
};

#[derive(Default)]
pub struct Program {
    threads: Vec<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl Deploy for Program {
    fn deploy(&mut self, mut state: impl State + Send + 'static) {
        let shutdown = self.shutdown.clone();
        self.threads.push(spawn(move || {
            bind_core();
            while !shutdown.load(Ordering::SeqCst) {
                state.poll();
            }
        }))
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        if !self.shutdown.swap(true, Ordering::SeqCst) {
            println!("! Implicitly shutdown replica threads on driver dropping");
        }
        for thread in take(&mut self.threads) {
            thread.join().unwrap()
        }
    }
}

impl Program {
    pub fn args(
        config: Arc<TransportConfig>,
        i: usize,
        app: App,
        n_effect: usize,
    ) -> ReplicaCommon {
        let socket = UdpSocket::bind(config.replica[i]).unwrap();
        socket.set_nonblocking(true).unwrap();
        ReplicaCommon {
            tx: TxChannel::Udp(socket.try_clone().unwrap()),
            rx: RxChannel::Udp(socket),
            n_effect,
            id: i,
            app,
            clock: Clock::Real,
        }
    }

    pub fn run_until_interrupt(&mut self) {
        assert!(!self.shutdown.load(Ordering::SeqCst));
        static FLAG: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
        extern "C" fn on_interrupt(_: i32) {
            *FLAG.0.lock().unwrap() = true;
            FLAG.1.notify_one();
        }
        unsafe { signal(SIGINT, SigHandler::Handler(on_interrupt)) }.unwrap();
        let _unused = FLAG
            .1
            .wait_while(FLAG.0.lock().unwrap(), |&mut interrupted| !interrupted)
            .unwrap();
        unsafe { signal(SIGINT, SigHandler::SigDfl) }.unwrap();
        self.shutdown.store(true, Ordering::SeqCst);
        for thread in take(&mut self.threads) {
            thread.join().unwrap();
        }
    }
}
