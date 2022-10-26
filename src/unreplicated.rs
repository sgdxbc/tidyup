use std::{collections::HashMap, time::Duration};

use messages::{
    unreplicated::{Reply, Request},
    ClientId, OpNumber, RequestNumber,
};

use crate::{
    app::App,
    transport::{deserialize, Transport, TransportReceiver},
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
            .work(move |worker| worker.send_message_to_replica(0, message));
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
        let message = deserialize::<Reply>(message);
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
}

impl AsMut<Transport<Self>> for Replica {
    fn as_mut(&mut self) -> &mut Transport<Self> {
        &mut self.transport
    }
}

impl Replica {
    pub fn new(transport: Transport<Self>, app: Box<dyn App>) -> Self {
        Self {
            transport,
            app,
            op_number: 0,
            cache: HashMap::new(),
        }
    }
}

impl TransportReceiver for Replica {
    fn receive_message(&mut self, message: &[u8]) {
        let message = deserialize::<Request>(message);
        self.wait_reply(&message, |self_, reply| {
            let remote = message.remote();
            self_
                .transport
                .work(move |worker| worker.send_message(remote, reply))
        });
    }
}

impl Replica {
    fn wait_reply(&mut self, request: &Request, then: impl FnOnce(&mut Self, Reply)) {
        if let Some(reply) = self.cache.get(&request.client_id) {
            if reply.request_number > request.request_number {
                return;
            }
            if reply.request_number == request.request_number {
                then(self, reply.clone());
                return;
            }
        }

        self.op_number += 1;
        let result = self.app.execute(self.op_number, &request.op);
        let message = Reply {
            request_number: request.request_number,
            result,
        };
        self.cache.insert(request.client_id, message.clone());
        then(self, message);
    }
}
