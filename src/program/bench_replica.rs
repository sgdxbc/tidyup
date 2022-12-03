use std::{
    mem::take,
    net::UdpSocket,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::{spawn, JoinHandle},
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, Sender};
use nix::sys::signal::{signal, SigHandler, Signal::SIGINT};

use crate::{
    core::{Clock, Deploy, ReplicaCommon, RxChannel, SharedState, TransportConfig, TxChannel},
    misc::bind_core,
    App, State,
};

pub struct Program {
    threads: Vec<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
    to_thread: (
        Sender<Box<dyn FnMut() + Send>>,
        Receiver<Box<dyn FnMut() + Send>>,
    ),
    shared_states: Vec<Arc<dyn SharedState + Send + Sync>>,
    n_thread: usize,
}

impl Program {
    pub fn new(n_thread: usize) -> Self {
        Self {
            threads: Default::default(),
            shared_states: Default::default(),
            shutdown: Arc::new(AtomicBool::new(false)),
            to_thread: crossbeam_channel::bounded(n_thread),
            n_thread,
        }
    }
}

impl Deploy for Program {
    fn deploy(&mut self, mut state: impl State + Send + 'static) {
        assert!(self.threads.len() < self.n_thread);
        let to_thread = self.to_thread.1.clone();
        let shutdown = self.shutdown.clone();
        self.threads.push(spawn(move || {
            bind_core();
            let mut on_idle = to_thread.recv().unwrap();
            let mut instant = Instant::now();
            while !shutdown.load(Ordering::SeqCst) {
                let now = Instant::now();
                if state.poll() {
                    instant = now;
                }
                // tune this parameter
                if now - instant >= Duration::from_millis(10) {
                    on_idle();
                }
            }
        }))
    }

    fn deploy_shared(&mut self, shared_state: impl SharedState + Send + Sync + 'static) {
        self.shared_states.push(Arc::new(shared_state))
    }
}

impl Drop for Program {
    fn drop(&mut self) {
        if !self.shutdown.swap(true, Ordering::SeqCst) {
            println!("! Implicitly shutdown replica threads on driver dropping");
        }
        for thread in Vec::from(take(&mut self.threads)) {
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
            config,
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

        for _ in 0..self.threads.len() {
            let shared_states = self.shared_states.clone();
            self.to_thread
                .0
                .send(Box::new(move || {
                    for shared_state in &shared_states {
                        shared_state.shared_poll();
                    }
                }))
                .unwrap()
        }
        while self.threads.len() < self.n_thread {
            let shared_states = self.shared_states.clone();
            let shutdown = self.shutdown.clone();
            self.threads.push(spawn(move || {
                bind_core();
                while !shutdown.load(Ordering::SeqCst) {
                    for shared_state in &shared_states {
                        shared_state.shared_poll();
                    }
                }
            }))
        }

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
        for thread in Vec::from(take(&mut self.threads)) {
            thread.join().unwrap();
        }
    }
}
