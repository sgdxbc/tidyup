use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use bincode::Options;
use message::{Reply, Request};

use crate::{
    effect_runner::EffectContext,
    misc::alloc_client_id,
    transport::{Clock, Config, RxChannel, TxChannel},
    EffectRunner, Receiver,
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
}

impl Receiver for Client {
    fn poll_at(&self) -> Option<Instant> {
        self.resend_instant
    }

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
    // app
    client_table: HashMap<u16, Reply>,
    runner: EffectRunner<ReplicaEffect>,
    message_channel: mpsc::Receiver<Request>,
}

pub struct ReplicaEffect {
    pub tx: TxChannel,
    pub rx: Arc<Mutex<RxChannel>>,
    pub buffer: [u8; 1500],
    pub message_channel: mpsc::SyncSender<Request>,
}

impl EffectContext for ReplicaEffect {
    fn idle_poll(&mut self) -> bool {
        let Some((len, _)) = self.rx.try_lock().ok().and_then(|mut rx| rx.receive_from(&mut self.buffer)) else {
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

impl Replica {
    pub fn new(
        config: Arc<Config>,
        runner: EffectRunner<ReplicaEffect>,
        message_channel: mpsc::Receiver<Request>,
    ) -> Self {
        assert_eq!(config.f, 0);
        Self {
            client_table: HashMap::new(),
            runner,
            message_channel,
        }
    }
}

impl Receiver for Replica {
    fn poll_at(&self) -> Option<Instant> {
        None
    }

    fn poll(&mut self) -> bool {
        if let Ok(message) = self.message_channel.try_recv() {
            self.handle_request(message);
            true
        } else {
            false
        }
    }
}

impl Replica {
    fn handle_request(&mut self, message: Request) {
        if let Some(reply) = self.client_table.get(&message.client_id) {
            if reply.request_number > message.request_number {
                println!("! ignore committed request");
                return;
            }
            if reply.request_number == message.request_number {
                let reply = reply.clone();
                self.runner.run(move |effect| {
                    effect
                        .tx
                        .send_to(&bincode::options().serialize(&reply).unwrap(), message.addr);
                });
                println!("* resend replied request");
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
        self.runner.run(move |effect| {
            effect
                .tx
                .send_to(&bincode::options().serialize(&reply).unwrap(), message.addr);
        });
    }
}
