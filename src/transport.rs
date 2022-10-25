use std::{
    collections::HashMap,
    io::ErrorKind,
    net::{SocketAddr, UdpSocket},
    panic::panic_any,
    sync::mpsc::{Receiver, Sender},
    time::{Duration, Instant},
};

use bincode::Options;
use mio::{Events, Interest, Poll, Token};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub remotes: Vec<SocketAddr>,
    pub public_keys: Vec<()>,
    pub secret_keys: Vec<()>,
    pub f: usize,
}

pub struct Transport<T: ?Sized> {
    pool: WorkerPool,
    reaction_id: u32,
    reactions: HashMap<u32, Box<dyn FnOnce(&mut T)>>,
    timeouts: HashMap<u32, TransportTimeout>,
}
struct TransportTimeout {
    delay: Duration,
    deadline: Instant,
}

impl<T> Transport<T> {
    pub fn create_timeout(
        &mut self,
        delay: Duration,
        reaction: impl FnOnce(&mut T) + 'static,
    ) -> u32
    where
        T: TransportReceiver,
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

    pub fn work(&mut self, task: impl FnOnce(&mut Worker) + 'static) {
        match &mut self.pool {
            WorkerPool::Inline(worker) => task(worker),
            //
        }
    }

    fn earliest_timeout(&self) -> (Instant, u32) {
        self.timeouts
            .iter()
            .map(|(&id, timeout)| (timeout.deadline, id))
            .min()
            .unwrap_or((Instant::now() + Duration::from_secs(600), u32::MAX))
    }
}

enum WorkerPool {
    Inline(Worker),
}

pub struct Worker {
    id: usize,
    socket: UdpSocket,
    local: usize, // remotes[local] is local socket address
    remotes: Vec<SocketAddr>,
    public_keys: Vec<()>,
    secret_key: (),
    back_channel: Sender<(usize, u32)>,
}

impl Worker {
    pub fn send_message(&self, dest: SocketAddr, message: impl Serialize) {
        self.socket
            .send_to(&bincode::options().serialize(&message).unwrap(), dest)
            .unwrap();
    }

    pub fn send_message_to_replica(&self, id: u8, message: impl Serialize) {
        self.send_message(self.remotes[id as usize], message);
    }

    pub fn send_message_to_all(&self, message: impl Serialize) {
        let message = bincode::options().serialize(&message).unwrap();
        for (i, &dest) in self.remotes.iter().enumerate() {
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

    config: Config,
    poll: Poll,

    back_channel: Receiver<(usize, u32)>,
    back_sender: Sender<(usize, u32)>,
}
struct ReceiverData<M> {
    receive_message: Box<dyn Fn(&mut M, &[u8]) -> (Instant, u32)>,
    execute_reaction: Box<dyn Fn(&mut M, u32) -> (Instant, u32)>,
    socket: UdpSocket,
    // have to keep a copy here because we need to access every receiver's
    // earliest timeout even when it is sleeping on slow path
    wake_deadline: Instant,
    wake_reaction: u32,
}

pub trait TransportReceiver
where
    Self: AsMut<Transport<Self>>,
{
    fn receive_message(&mut self, message: &[u8]);
}

impl<M> TransportRuntime<M> {
    pub fn create_transport<T>(
        &mut self,
        addr: SocketAddr,
        replica_id: u8,
        receiver_mut: impl Fn(&mut M) -> &mut T + 'static + Clone,
    ) -> Transport<T>
    where
        T: TransportReceiver,
    {
        let id = self.receivers.len();
        let socket = UdpSocket::bind(addr).unwrap();
        socket.set_nonblocking(true).unwrap();

        self.poll
            .registry()
            .register(
                &mut mio::net::UdpSocket::from_std(socket.try_clone().unwrap()),
                Token(id),
                Interest::READABLE,
            )
            .unwrap();

        let worker = Worker {
            id,
            local: replica_id as _,
            remotes: self.config.remotes.clone(),
            public_keys: self.config.public_keys.clone(),
            secret_key: self.config.secret_keys[replica_id as usize], // TODO
            socket: socket.try_clone().unwrap(),
            back_channel: self.back_sender.clone(),
        };

        self.receivers.push(ReceiverData {
            receive_message: Box::new({
                let receiver_mut = receiver_mut.clone();
                move |context, message| {
                    receiver_mut(context).receive_message(message);
                    receiver_mut(context).as_mut().earliest_timeout()
                }
            }),
            execute_reaction: Box::new(move |context, reaction_id| {
                let reaction = receiver_mut(context)
                    .as_mut()
                    .reactions
                    .remove(&reaction_id)
                    .unwrap();
                reaction(receiver_mut(context));
                receiver_mut(context).as_mut().earliest_timeout()
            }),
            wake_deadline: Instant::now() + Duration::from_secs(600),
            wake_reaction: u32::MAX,
            socket,
        });

        Transport {
            reaction_id: 0,
            reactions: HashMap::new(),
            timeouts: HashMap::new(),
            pool: WorkerPool::Inline(worker),
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

        if let Some((deadline, id, reaction)) = self
            .receivers
            .iter()
            .enumerate()
            .map(|(id, peer)| (peer.wake_deadline, id, peer.wake_reaction))
            .min()
        {
            assert!(deadline <= self.wake_deadline);
            self.wake_deadline = deadline;
            self.wake_transport = id;
            self.wake_reaction = reaction;
        }
    }

    pub fn run(&mut self, context: &mut M) {
        let mut buffer = [0; (64 << 20) - 20 - 8];
        let mut events = Events::with_capacity(64);
        loop {
            while Instant::now() >= self.wake_deadline {
                assert_ne!(self.wake_transport, usize::MAX);
                assert_ne!(self.receivers[self.wake_transport].wake_reaction, u32::MAX);
                let earliest_timeout = (self.receivers[self.wake_transport].execute_reaction)(
                    context,
                    self.wake_reaction,
                );
                self.update_wake(self.wake_transport, earliest_timeout);
            }

            while let Ok((id, reaction_id)) = self.back_channel.try_recv() {
                let earliest_timeout = (self.receivers[id].execute_reaction)(context, reaction_id);
                self.update_wake(id, earliest_timeout);
            }

            // TODO prioritize timeout and triggered action
            self.poll.poll(&mut events, Some(Duration::ZERO)).unwrap();
            for event in events.iter() {
                let Token(id) = event.token();
                assert!(event.is_readable());
                loop {
                    let receive_message = &self.receivers[id].receive_message;
                    match self.receivers[id].socket.recv_from(&mut buffer) {
                        Ok((len, _remote)) => {
                            let earliest_timeout = receive_message(context, &buffer[..len]);
                            self.update_wake(id, earliest_timeout);
                        }
                        Err(err) if err.kind() == ErrorKind::WouldBlock => break,
                        err => panic_any(err),
                    }
                }
            }
        }
    }
}
