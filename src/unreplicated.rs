use std::{collections::HashMap, convert::identity, net::SocketAddr, time::Duration};

use bincode::Options;
use message::{Reply, Request};

use crate::{
    core::{ClientCommon, RxChannel, TxChannel},
    App, ClientState, Deploy, OptionInstant, ReplicaCommon, State,
};

pub struct Client {
    common: ClientCommon,
    request_number: u32,
    op: Option<Box<[u8]>>,
    result: Option<Box<[u8]>>,
    resend_instant: OptionInstant,
    buffer: [u8; 1500],
}

impl Client {
    pub fn new(common: ClientCommon) -> Self {
        Self {
            request_number: 0,
            op: None,
            result: None,
            common,
            resend_instant: Default::default(),
            buffer: [0; 1500],
        }
    }
}

impl ClientState for Client {
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

    fn poll_at(&self) -> OptionInstant {
        self.resend_instant
    }
}

impl State for Client {
    fn poll(&mut self) -> bool {
        if self.resend_instant <= self.common.clock.now() {
            println!(
                "! Resend: Client {} Request {}",
                self.common.id, self.request_number
            );
            self.do_send();
        }

        let Some((len, _)) = self.common.rx.receive_from(&mut self.buffer) else {
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
            self.resend_instant = Default::default();
        }
        true
    }
}

impl Client {
    fn do_send(&mut self) {
        let request = Request {
            client_id: self.common.id,
            request_number: self.request_number,
            op: self.op.clone().unwrap(),
            addr: self.common.rx_addr,
        };
        self.common.tx.send_to(
            &bincode::options().serialize(&request).unwrap(),
            self.common.config.replica[0],
        );
        self.resend_instant = self.common.clock.after(Duration::from_millis(10));
    }
}

pub struct Replica {
    main: MainThread,
    listen: ListenThread,
    effect: Box<[EffectThread]>,
}

struct MainThread {
    app: App,
    client_table: HashMap<u16, Reply>,
    op_number: u32,
    log: Vec<Request>,
    ingress: crossbeam_channel::Receiver<Request>,
    to_effect: crossbeam_channel::Sender<(SocketAddr, Reply)>,
}

struct ListenThread {
    rx: RxChannel,
    buffer: [u8; 1500],
    to_main: crossbeam_channel::Sender<Request>,
}

struct EffectThread {
    tx: TxChannel,
    ingress: crossbeam_channel::Receiver<(SocketAddr, Reply)>,
}

impl Replica {
    pub fn new(common: ReplicaCommon) -> Self {
        let message_channel = crossbeam_channel::bounded(1024);
        let effect_channel = crossbeam_channel::bounded(1024);
        Self {
            main: MainThread {
                app: common.app,
                client_table: Default::default(),
                op_number: 0,
                log: Default::default(),
                ingress: message_channel.1,
                to_effect: effect_channel.0,
            },
            listen: ListenThread {
                rx: common.rx,
                buffer: [0; 1500],
                to_main: message_channel.0,
            },
            effect: (0..common.n_effect)
                .map(|_| EffectThread {
                    tx: common.tx.clone(),
                    ingress: effect_channel.1.clone(),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    pub fn deploy(self, program: &mut impl Deploy) {
        program.deploy(self.main);
        program.deploy(self.listen);
        for thread in Vec::from(self.effect) {
            program.deploy(thread);
        }
    }
}

impl State for Replica {
    fn poll(&mut self) -> bool {
        let mut poll_again = Vec::new();
        poll_again.push(self.listen.poll());
        poll_again.push(self.main.poll());
        poll_again.extend(self.effect.iter_mut().map(|thread| thread.poll()));
        poll_again.into_iter().any(identity)
    }
}

impl State for ListenThread {
    fn poll(&mut self) -> bool {
        let Some((len, _)) = self.rx.receive_from(&mut self.buffer) else {
            return false;
        };
        let request = bincode::options()
            .allow_trailing_bytes()
            .deserialize(&self.buffer[..len])
            .unwrap();
        self.to_main.send(request).unwrap();
        true
    }
}

impl State for MainThread {
    fn poll(&mut self) -> bool {
        if let Ok(message) = self.ingress.try_recv() {
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
                self.to_effect.send((message.addr, reply.clone())).unwrap();
                return;
            }
        }
        self.log.push(message.clone());
        self.op_number += 1;
        let result = self.app.execute(self.op_number, &message.op);
        let reply = Reply {
            request_number: message.request_number,
            result,
            ..Default::default()
        };
        self.client_table.insert(message.client_id, reply.clone());
        self.to_effect.send((message.addr, reply)).unwrap();
    }
}

impl State for EffectThread {
    fn poll(&mut self) -> bool {
        if let Ok((dest, message)) = self.ingress.try_recv() {
            self.tx
                .send_to(&bincode::options().serialize(&message).unwrap(), dest);
            true
        } else {
            false
        }
    }
}
