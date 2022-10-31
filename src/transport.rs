use std::{
    collections::HashMap,
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    os::unix::prelude::{AsRawFd, RawFd},
    panic::panic_any,
    sync::mpsc::{channel, Receiver, Sender},
    time::{Duration, Instant},
};

use messages::{
    crypto::{PublicKey, SecretKey},
    serialize, ClientId, ReplicaId,
};
use nix::sys::epoll::{epoll_create, epoll_ctl, epoll_wait, EpollEvent, EpollFlags, EpollOp};
use serde::{Deserialize, Serialize};

use crate::worker::{Work, WorkerPool};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub remotes: Vec<SocketAddr>,
    pub public_keys: Vec<PublicKey>,
    pub secret_keys: Vec<SecretKey>,
    pub f: usize,
}

pub struct Transport<R: ?Sized> {
    work: Work,
    reaction_id: u32,
    reactions: HashMap<u32, Box<dyn FnOnce(&mut R)>>,
    timeouts: HashMap<u32, TransportTimeout>,
    local_addr: SocketAddr,
    pub n: usize,
    pub f: usize,
}
struct TransportTimeout {
    delay: Duration,
    deadline: Instant,
}

impl<R> Transport<R> {
    pub fn create_timeout(
        &mut self,
        delay: Duration,
        reaction: impl FnOnce(&mut R) + 'static,
    ) -> u32
    where
        R: AsMut<Transport<R>>,
    {
        self.reaction_id += 1;
        let id = self.reaction_id;
        self.reactions.insert(
            id,
            Box::new(move |receiver| {
                receiver.as_mut().timeouts.remove(&id).unwrap();
                reaction(receiver);
            }),
        );
        self.timeouts.insert(
            id,
            TransportTimeout {
                delay,
                deadline: Instant::now() + delay,
            },
        );
        id
    }

    pub fn reset_timeout(&mut self, id: u32) {
        let timeout = self.timeouts.get_mut(&id).unwrap();
        timeout.deadline = Instant::now() + timeout.delay;
    }

    pub fn cancel_timeout(&mut self, id: u32) {
        drop(self.reactions.remove(&id).unwrap()); // explicit discard closure
        self.timeouts.remove(&id).unwrap();
    }
}

pub struct WorkTask<'a, R> {
    transport: &'a mut Transport<R>,
    task: Box<dyn FnOnce(&mut Worker) + Send>,
}

impl<R> WorkTask<'_, R> {
    pub fn detach(self) {
        match &mut self.transport.work {
            Work::Inline(worker) => (self.task)(worker),
            Work::Pool(pool) => pool.work(self.task),
        }
    }

    pub fn then(self, reaction: impl FnOnce(&mut R) + 'static) {
        self.transport.reaction_id += 1;
        let id = self.transport.reaction_id;
        self.transport.reactions.insert(id, Box::new(reaction));
        let task = move |worker: &mut Worker| {
            (self.task)(worker);
            worker.trigger_reaction(id);
        };
        match &mut self.transport.work {
            Work::Inline(worker) => task(worker),
            Work::Pool(pool) => pool.work(task),
        }
    }
}

impl<R> Transport<R> {
    #[must_use]
    pub fn work(&mut self, task: impl FnOnce(&mut Worker) + Send + 'static) -> WorkTask<'_, R> {
        WorkTask {
            transport: self,
            task: Box::new(task),
        }
    }

    fn earliest_timeout(&self) -> (Instant, u32) {
        self.timeouts
            .iter()
            .map(|(&id, timeout)| (timeout.deadline, id))
            .min()
            .unwrap_or((Instant::now() + Duration::from_secs(600), u32::MAX))
    }

    pub fn client_id(&self) -> ClientId {
        ClientId::likely_unique(self.local_addr)
    }
}

pub struct Worker {
    id: usize,
    socket: UdpSocket,
    local: usize, // remotes[local] is local socket address
    pub config: Config,
    pub secret_key: SecretKey,
    back_channel: Sender<(usize, u32)>,
}

impl Clone for Worker {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            socket: self.socket.try_clone().unwrap(),
            local: self.local,
            back_channel: self.back_channel.clone(),
            secret_key: self.secret_key.clone(),
            config: self.config.clone(),
        }
    }
}

impl Worker {
    pub fn send_message(&self, dest: SocketAddr, message: impl Serialize) {
        self.socket.send_to(&serialize(&message), dest).unwrap();
    }

    pub fn send_message_to_replica(&self, id: u8, message: impl Serialize) {
        self.send_message(self.config.remotes[id as usize], message);
    }

    pub fn send_message_to_all(&self, message: impl Serialize) {
        let message = serialize(&message);
        for (i, &dest) in self.config.remotes.iter().enumerate() {
            if i == self.local {
                continue;
            }
            self.socket.send_to(&message, dest).unwrap();
        }
    }

    pub fn trigger_reaction(&self, id: u32) {
        self.back_channel.send((self.id, id)).unwrap();
    }
}

pub struct TransportRuntime<M> {
    receivers: Vec<ReceiverData<M>>,
    wake_deadline: Instant,
    wake_transport: usize,
    wake_reaction: u32,
    context_timeouts: Vec<(Instant, Box<dyn FnOnce(&mut M, &mut Self)>)>,

    config: Config,
    poll: RawFd,

    is_running: bool,
    back_channel: Receiver<(usize, u32)>,
    back_sender: Sender<(usize, u32)>,
}
struct ReceiverData<M> {
    receive_message: Box<dyn Fn(&mut M, &[u8]) -> (Instant, u32)>,
    execute_reaction: Box<dyn Fn(&mut M, u32) -> (Instant, u32)>,
    socket: UdpSocket,
    // have to keep a copy here because on slow path we need to access every
    // receiver's earliest timeout even when it is sleeping
    wake_deadline: Instant,
    wake_reaction: u32,
}

impl<C> TransportRuntime<C> {
    pub fn new(config: Config) -> Self {
        let (back_sender, back_channel) = channel();
        Self {
            receivers: Vec::new(),
            wake_deadline: Instant::now() + Duration::from_secs(600),
            wake_transport: usize::MAX,
            wake_reaction: u32::MAX,
            context_timeouts: Vec::new(),
            config,
            poll: epoll_create().unwrap(),
            is_running: false,
            back_channel,
            back_sender,
        }
    }

    pub fn stop(&mut self) {
        assert!(self.is_running);
        self.is_running = false;
    }
}

pub trait ReactorMut<Reactor = Self> {
    fn reactor_mut(&mut self) -> &mut Reactor;
}

pub trait TransportReceiver {
    fn receive_message(&mut self, message: &[u8]);
}

impl<R> ReactorMut<Self> for R
where
    R: AsMut<Transport<Self>>,
{
    fn reactor_mut(&mut self) -> &mut Self {
        self
    }
}

impl<C> TransportRuntime<C> {
    pub fn create_transport<T, R>(
        &mut self,
        addr: SocketAddr,
        replica_id: u8,
        n_worker: usize,
        receiver_mut: impl Fn(&mut C) -> &mut T + 'static + Clone,
    ) -> Transport<R>
    where
        T: TransportReceiver + ReactorMut<R>,
        R: AsMut<Transport<R>>,
    {
        let id = self.receivers.len();
        let socket = UdpSocket::bind(addr).unwrap();
        socket.set_nonblocking(true).unwrap();

        epoll_ctl(
            self.poll,
            EpollOp::EpollCtlAdd,
            socket.as_raw_fd(),
            &mut EpollEvent::new(EpollFlags::EPOLLET | EpollFlags::EPOLLIN, id as _),
        )
        .unwrap();

        let worker = Worker {
            id,
            local: replica_id as _,
            socket: socket.try_clone().unwrap(),
            back_channel: self.back_sender.clone(),
            secret_key: if replica_id != ReplicaId::MAX {
                self.config.secret_keys[replica_id as usize].clone()
            } else {
                SecretKey::Disabled
            },
            config: self.config.clone(),
        };

        let local_addr = socket.local_addr().unwrap();

        self.receivers.push(ReceiverData {
            receive_message: Box::new({
                let receiver_mut = receiver_mut.clone();
                move |context, message| {
                    receiver_mut(context).receive_message(message);
                    receiver_mut(context)
                        .reactor_mut()
                        .as_mut()
                        .earliest_timeout()
                }
            }),
            execute_reaction: Box::new(move |context, reaction_id| {
                let reaction = receiver_mut(context)
                    .reactor_mut()
                    .as_mut()
                    .reactions
                    .remove(&reaction_id)
                    .unwrap();
                reaction(receiver_mut(context).reactor_mut());
                receiver_mut(context)
                    .reactor_mut()
                    .as_mut()
                    .earliest_timeout()
            }),
            wake_deadline: Instant::now() + Duration::from_secs(600),
            wake_reaction: u32::MAX,
            socket,
        });

        Transport {
            reaction_id: 0,
            reactions: HashMap::new(),
            timeouts: HashMap::new(),
            work: if n_worker == 0 {
                Work::Inline(worker)
            } else {
                assert_ne!(replica_id, ReplicaId::MAX);
                Work::Pool(WorkerPool::new(n_worker, worker))
            },
            local_addr,
            n: self.config.remotes.len(),
            f: self.config.f,
        }
    }

    fn update_wake(&mut self, id: usize, earliest_timeout: (Instant, u32)) {
        let (deadline, reaction) = earliest_timeout;
        self.receivers[id].wake_deadline = deadline;
        self.receivers[id].wake_reaction = reaction;
        // shortcut: this receiver wakes on earliest globally
        if deadline < self.wake_deadline {
            self.wake_deadline = deadline;
            self.wake_reaction = reaction;
            self.wake_transport = id;
            return;
        }
        // shortcut: this receiver did not wake up earliest, or it did not
        // modify its earliest timeout
        if !(self.wake_transport == id && self.wake_deadline < deadline) {
            return;
        }

        self.update_wake_internal();
    }

    fn update_wake_internal(&mut self) {
        if let Some((deadline, id, reaction)) = self
            .receivers
            .iter()
            .enumerate()
            .map(|(id, peer)| (peer.wake_deadline, id, peer.wake_reaction))
            .chain(
                self.context_timeouts
                    .iter()
                    .enumerate()
                    .map(|(i, &(deadline, _))| (deadline, usize::MAX, i as _)),
            )
            .min()
        {
            self.wake_deadline = deadline;
            self.wake_transport = id;
            self.wake_reaction = reaction;
        }
    }

    pub fn create_timeout(
        &mut self,
        delay: Duration,
        reaction: impl FnOnce(&mut C, &mut Self) + 'static,
    ) {
        self.context_timeouts
            .push((Instant::now() + delay, Box::new(reaction)));
        self.update_wake_internal();
    }

    pub fn run(&mut self, context: &mut C) {
        self.is_running = true;
        let mut buffer = [0; (u16::MAX - 20 - 8) as _];
        let mut event_buffer = [EpollEvent::empty(); 64];
        let mut events: &[EpollEvent] = &[];
        while self.is_running {
            while Instant::now() >= self.wake_deadline {
                if self.wake_transport == usize::MAX {
                    self.context_timeouts.swap_remove(self.wake_reaction as _).1(context, self);
                    self.update_wake_internal();
                } else {
                    assert_ne!(self.receivers[self.wake_transport].wake_reaction, u32::MAX);
                    let earliest_timeout = (self.receivers[self.wake_transport].execute_reaction)(
                        context,
                        self.wake_reaction,
                    );
                    self.update_wake(self.wake_transport, earliest_timeout);
                }
            }

            while let Ok((id, reaction_id)) = self.back_channel.try_recv() {
                let earliest_timeout = (self.receivers[id].execute_reaction)(context, reaction_id);
                self.update_wake(id, earliest_timeout);
            }

            if events.is_empty() {
                let len = epoll_wait(self.poll, &mut event_buffer, 0).unwrap();
                events = &event_buffer[..len];
            }
            let event = if let Some(event) = events.first() {
                event
            } else {
                continue;
            };
            assert!(event.events().contains(EpollFlags::EPOLLIN));
            let id = event.data() as usize;
            match self.receivers[id].socket.recv_from(&mut buffer) {
                Ok((len, _remote)) => {
                    let earliest_timeout =
                        (self.receivers[id].receive_message)(context, &buffer[..len]);
                    self.update_wake(id, earliest_timeout);
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    events = &events[1..];
                }
                err => panic_any(err),
            }
        }
    }
}
