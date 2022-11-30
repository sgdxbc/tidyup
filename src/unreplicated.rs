use std::{
    collections::HashMap,
    convert::identity,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use bincode::Options;
use message::{Reply, Request};

use crate::{
    misc::alloc_client_id,
    receiver::{Deploy, ReplicaArgs},
    transport::{Clock, Config, RxChannel, TxChannel},
    Receiver,
};

pub struct Client {
    id: u16,
    request_number: u32,
    op: Option<Box<[u8]>>,
    result: Option<Box<[u8]>>,

    config: Arc<Config>,
    effect: ClientEffect,
    resend_instant: Option<Instant>,
    buffer: [u8; 1500],
}

pub struct ClientEffect {
    pub tx: TxChannel,
    pub rx: RxChannel,
    pub rx_addr: SocketAddr,
    pub clock: Clock,
}

impl Client {
    pub fn new(config: Arc<Config>, effect: ClientEffect) -> Self {
        Self {
            id: alloc_client_id(),
            request_number: 0,
            op: None,
            result: None,
            config,
            effect,
            resend_instant: None,
            buffer: [0; 1500],
        }
    }
}

impl crate::Client for Client {
    fn invoke(&mut self, op: Box<[u8]>) {
        assert!(self.op.is_none());
        self.request_number += 1;
        self.op = Some(op);
        self.result = None;
        self.do_send();
    }

    fn take_result(&mut self) -> Option<Box<[u8]>> {
        self.result.take()
    }

    fn poll_at(&self) -> Option<Instant> {
        self.resend_instant
    }
}

impl Receiver for Client {
    fn poll(&mut self) -> bool {
        if self
            .resend_instant
            .filter(|&instant| instant <= self.effect.clock.now())
            .is_some()
        {
            println!(
                "! Resend: Client {} Request {}",
                self.id, self.request_number
            );
            self.do_send();
        }

        let Some((len, _)) = self.effect.rx.receive_from(&mut self.buffer) else {
            return false;
        };
        'rx: {
            if self.op.is_none() {
                break 'rx;
            }
            let message = bincode::options()
                .allow_trailing_bytes()
                .deserialize::<Reply>(&self.buffer[..len])
                .unwrap();
            if message.request_number != self.request_number {
                break 'rx;
            }
            self.op = None;
            self.result = Some(message.result);
            self.resend_instant = None;
        }
        true
    }
}

impl Client {
    fn do_send(&mut self) {
        let request = Request {
            client_id: self.id,
            request_number: self.request_number,
            op: self.op.clone().unwrap(),
            addr: self.effect.rx_addr,
        };
        self.effect.tx.send_to(
            &bincode::options().serialize(&request).unwrap(),
            self.config.replica[0],
        );
        self.resend_instant = Some(self.effect.clock.now() + Duration::from_millis(10));
    }
}

pub struct Replica {
    main: MainThread,
    listen: ListenThread,
    effect: Box<[EffectThread]>,
}

pub struct MainThread {
    // app
    client_table: HashMap<u16, Reply>,
    message_channel: crossbeam_channel::Receiver<Request>,
    effect_channel: crossbeam_channel::Sender<(SocketAddr, Reply)>,
}

pub struct ListenThread {
    rx: RxChannel,
    buffer: [u8; 1500],
    message_channel: crossbeam_channel::Sender<Request>,
}

pub struct EffectThread {
    tx: TxChannel,
    main_channel: crossbeam_channel::Receiver<(SocketAddr, Reply)>,
}

impl Replica {
    pub fn new(args: ReplicaArgs) -> Self {
        let message_channel = crossbeam_channel::bounded(1024);
        let effect_channel = crossbeam_channel::bounded(1024);
        Self {
            main: MainThread {
                client_table: Default::default(),
                message_channel: message_channel.1,
                effect_channel: effect_channel.0,
            },
            listen: ListenThread {
                rx: args.rx,
                buffer: [0; 1500],
                message_channel: message_channel.0,
            },
            effect: (0..args.n_effect)
                .map(|_| EffectThread {
                    tx: args.tx.clone(),
                    main_channel: effect_channel.1.clone(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    pub fn deploy(self, driver: &mut impl Deploy) {
        driver.deploy(self.main);
        driver.deploy(self.listen);
        for thread in Vec::from(self.effect) {
            driver.deploy(thread);
        }
    }
}

impl Receiver for Replica {
    fn poll(&mut self) -> bool {
        let mut poll_again = Vec::new();
        poll_again.push(self.listen.poll());
        poll_again.push(self.main.poll());
        poll_again.extend(self.effect.iter_mut().map(|thread| thread.poll()));
        poll_again.into_iter().any(identity)
    }
}

impl Receiver for ListenThread {
    fn poll(&mut self) -> bool {
        let Some((len, _)) = self.rx.receive_from(&mut self.buffer) else {
            return false;
        };
        let request = bincode::options()
            .allow_trailing_bytes()
            .deserialize(&self.buffer[..len])
            .unwrap();
        self.message_channel.send(request).unwrap();
        true
    }
}

impl Receiver for MainThread {
    fn poll(&mut self) -> bool {
        if let Ok(message) = self.message_channel.try_recv() {
            self.handle_request(message);
            true
        } else {
            false
        }
    }
}

impl MainThread {
    fn handle_request(&mut self, message: Request) {
        if let Some(reply) = self.client_table.get(&message.client_id) {
            if reply.request_number > message.request_number {
                println!("! ignore committed request");
                return;
            }
            if reply.request_number == message.request_number {
                println!("* resend replied request");
                self.effect_channel
                    .send((message.addr, reply.clone()))
                    .unwrap();
                return;
            }
        }
        // execute app
        let result = Box::new([]);
        let reply = Reply {
            request_number: message.request_number,
            result,
        };
        self.client_table.insert(message.client_id, reply.clone());
        self.effect_channel.send((message.addr, reply)).unwrap();
    }
}

impl Receiver for EffectThread {
    fn poll(&mut self) -> bool {
        if let Ok((dest, message)) = self.main_channel.try_recv() {
            self.tx
                .send_to(&bincode::options().serialize(&message).unwrap(), dest);
            true
        } else {
            false
        }
    }
}
