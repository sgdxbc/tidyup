use std::{collections::HashSet, time::Duration};

use messages::{
    deserialize,
    hotstuff::{Reply, Request},
    ClientId, ReplicaId, RequestNumber,
};

use crate::transport::{Transport, TransportReceiver};

pub struct Client {
    transport: Transport<Self>,
    id: ClientId,
    request_number: RequestNumber,
    op: Option<Box<[u8]>>,
    result: Option<Box<[u8]>>,
    replied_replicas: HashSet<ReplicaId>,
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
            replied_replicas: HashSet::new(),
            timeout: 0,
        }
    }
}

impl crate::client::Client for Client {
    fn take_result(&mut self) -> Option<Box<[u8]>> {
        if self.op.is_some() {
            None
        } else {
            self.result.take()
        }
    }

    fn invoke(&mut self, op: Box<[u8]>) {
        assert!(self.op.is_none());
        assert!(self.result.is_none());
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
            .work(move |worker| worker.send_message_to_all(message));
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
        if self.op.is_none() {
            return;
        }
        let message = deserialize::<Reply>(message);
        if message.request_number != self.request_number {
            return;
        }

        if let Some(result) = &self.result {
            if &message.result != result {
                return;
            }
        } else {
            self.result = Some(message.result);
        }
        self.replied_replicas.insert(message.replica_id);

        if self.replied_replicas.len() == self.transport.f + 1 {
            self.op.take().unwrap();
            self.replied_replicas.drain();
            self.transport.cancel_timeout(self.timeout);
        }
    }
}
