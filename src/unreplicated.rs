use std::{collections::HashMap, time::Duration};

use messages::{
    deserialize_from,
    unreplicated::{Reply, Request},
    ClientId, OpNumber, RequestNumber,
};

use crate::{
    app::App,
    transport::{Transport, TransportReceiver},
};

pub struct Client {
    transport: Transport<Self>,
    id: ClientId,
    request_number: RequestNumber,
    op: Option<Box<[u8]>>,
    result: Option<Box<[u8]>>,
    timeout: u32,
}

impl AsMut<Transport<Self>> for Client {
    fn as_mut(&mut self) -> &mut Transport<Self> {
        &mut self.transport
    }
}

impl Client {
    pub fn new(transport: Transport<Self>) -> Self {
        Self {
            id: transport.client_id(),
            transport,
            request_number: 0,
            op: None,
            result: None,
            timeout: 0,
        }
    }
}

impl crate::client::Client for Client {
    fn take_result(&mut self) -> Option<Box<[u8]>> {
        self.result.take()
    }

    fn invoke(&mut self, op: Box<[u8]>) {
        assert!(self.op.is_none());
        self.request_number += 1;
        self.op = Some(op);
        self.do_request();
    }
}

impl Client {
    fn do_request(&mut self) {
        assert!(self.result.is_none());
        let message = Request {
            client_id: self.id,
            request_number: self.request_number,
            op: self.op.clone().unwrap(),
        };
        self.transport
            .work(move |worker| worker.send_message_to_replica(0, message))
            .detach();
        self.timeout = self
            .transport
            .create_timeout(Duration::from_secs(1), |self_| {
                //
                self_.do_request();
            });
    }
}

impl TransportReceiver for Client {
    fn receive_message(&mut self, message: &[u8]) {
        if self.result.is_some() {
            return;
        }
        let message = deserialize_from::<Reply>(message);
        if message.request_number != self.request_number {
            return;
        }
        self.result = Some(message.result);
        self.op.take().unwrap();
        self.transport.cancel_timeout(self.timeout);
    }
}

pub struct Replica {
    transport: Transport<Self>,
    // id fixed to 0
    app: Box<dyn App>,
    op_number: OpNumber,
    cache: HashMap<ClientId, Reply>,
    pub log: Vec<Request>,
}

impl AsMut<Transport<Self>> for Replica {
    fn as_mut(&mut self) -> &mut Transport<Self> {
        &mut self.transport
    }
}

impl Replica {
    pub fn new(transport: Transport<Self>, app: impl App + 'static) -> Self {
        Self {
            transport,
            app: Box::new(app),
            op_number: 0,
            cache: HashMap::new(),
            log: Vec::new(),
        }
    }
}

impl TransportReceiver for Replica {
    fn receive_message(&mut self, message: &[u8]) {
        let message = deserialize_from::<Request>(message);
        let dest = message.remote();
        match self.cache.get(&message.client_id) {
            Some(reply) if reply.request_number > message.request_number => return,
            Some(reply) if reply.request_number == message.request_number => {
                let reply = reply.clone();
                self.transport
                    .work(move |worker| worker.send_message(dest, reply))
                    .detach();
            }
            _ => {}
        }

        self.log.push(message.clone());
        self.op_number += 1;
        assert_eq!(self.log.len() as OpNumber, self.op_number);
        let result = self.app.execute(self.op_number, &message.op);
        let reply = Reply {
            request_number: message.request_number,
            result,
        };
        self.cache.insert(message.client_id, reply.clone());
        self.transport
            .work(move |worker| worker.send_message(dest, reply))
            .detach();
    }
}
