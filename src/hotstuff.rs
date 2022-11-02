use std::{
    collections::{HashMap, HashSet},
    mem::take,
    sync::{Arc, Mutex},
    time::Duration,
};

use messages::{
    crypto::{Signature, Signed},
    deserialize_from, digest,
    hotstuff::{Generic, Reply, Request, ToReplica, Vote, GENESIS},
    ClientId, Digest, OpNumber, ReplicaId, RequestNumber, ViewNumber,
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
            .work(move |worker| worker.send_message_to_all(ToReplica::Request(message)))
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
        if self.op.is_none() {
            return;
        }
        let message = deserialize_from::<Reply>(message);
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

pub struct Replica {
    transport: Transport<Self>,
    id: ReplicaId,
    app: Box<dyn App>,
    // common
    view_number: ViewNumber,
    cache: HashMap<ClientId, (RequestNumber, Option<Reply>)>,
    storage: HashMap<Digest, Block>,
    block_lock: Digest,
    block_execute: Digest,
    // primary
    requests: Vec<Request>,
    certified: Digest,
    certificates: HashMap<Digest, HashMap<ReplicaId, Signature>>,
    parent: Digest,
    // backup
    vote_height: OpNumber,
    // reactions
    waiting_delivered: HashMap<Digest, Vec<Box<dyn FnOnce(&mut Self)>>>,
    waiting_certificate: HashMap<Digest, Vec<Box<dyn FnOnce(&mut Self)>>>,
    waiting_request: Vec<Box<dyn FnOnce(&mut Self)>>,
}

#[derive(Debug, Default, Clone)]
struct Block {
    height: OpNumber,
    requests: Vec<Request>,
    parent: Digest,
    certified: Digest,
    status: BlockStatus,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum BlockStatus {
    #[default]
    Delivering,
    Voting,
    Committed,
}

impl AsMut<Transport<Self>> for Replica {
    fn as_mut(&mut self) -> &mut Transport<Self> {
        &mut self.transport
    }
}

impl Replica {
    pub fn new(transport: Transport<Self>, id: ReplicaId, app: impl App + 'static) -> Self {
        let mut storage = HashMap::new();
        storage.insert(
            GENESIS,
            Block {
                status: BlockStatus::Committed,
                ..Default::default()
            },
        );
        let mut certificates = HashMap::new();
        certificates.insert(GENESIS, HashMap::new());
        let mut self_ = Self {
            transport,
            id,
            app: Box::new(app),
            view_number: 0,
            block_lock: GENESIS,
            block_execute: GENESIS,
            cache: HashMap::new(),
            requests: Vec::new(),
            storage,
            certified: GENESIS,
            certificates,
            parent: GENESIS,
            vote_height: 0,
            waiting_delivered: HashMap::new(),
            waiting_certificate: HashMap::new(),
            waiting_request: Vec::new(),
        };
        if self_.id == self_.primary_id() {
            self_.register_propose();
        }
        self_
    }
}

impl TransportReceiver for Replica {
    fn receive_message(&mut self, message: &[u8]) {
        match deserialize_from::<ToReplica>(message) {
            ToReplica::Request(message) => self.handle_request(message),
            ToReplica::Generic(message) => {
                self.wait_verified_generic(message, |self_, message| self_.handle_generic(message))
            }
            ToReplica::Vote(message) => self.wait_verified_vote(message, Self::handle_vote),
        }
    }
}

impl Replica {
    fn register_propose(&mut self) {
        self.wait_certificate(self.parent, |self_| {
            self_.wait_request(|self_| {
                if self_.id == self_.primary_id() {
                    self_.do_propose();
                    self_.register_propose();
                }
            })
        });
    }

    fn handle_request(&mut self, message: Request) {
        match self.cache.get(&message.client_id) {
            Some((request_number, _)) if request_number > &message.request_number => return,
            Some((request_number, reply)) if request_number == &message.request_number => {
                if let Some(reply) = reply {
                    let reply = reply.clone();
                    let dest = message.remote();
                    self.transport
                        .work(move |worker| worker.send_message(dest, reply))
                        .detach();
                }
                return;
            }
            _ => {}
        }
        self.cache
            .insert(message.client_id, (message.request_number, None));

        if self.id == self.primary_id() {
            self.requests.push(message);
            self.on_request();
        }
    }

    fn handle_generic(&mut self, message: Generic) {
        let replica_id = message.replica_id;
        let digest = digest(&(&message.requests, &message.parent));
        if replica_id != self.id {
            for request in &message.requests {
                let cache = self
                    .cache
                    .entry(request.client_id)
                    .or_insert((request.request_number, None));
                if cache.0 < request.request_number {
                    *cache = (request.request_number, None);
                }
            }
        }
        self.storage.entry(digest).or_insert(Block {
            height: OpNumber::MAX,
            requests: message.requests,
            parent: message.parent,
            certified: message.certified,
            ..Default::default()
        });
        if message.replica_id != self.id {
            self.certificates
                .insert(message.certified, message.certificate.into_iter().collect());
            self.on_certificate(&message.certified);
        }

        self.wait_delivered_block(
            digest,
            Box::new(move |self_| {
                self_.do_update(digest);

                self_.parent = digest;
                if replica_id != self_.id {
                    // cancel timeout for certified block
                }

                if self_.vote_opinion(digest) {
                    self_.vote_height = self_.storage[&digest].height;
                    self_.do_vote(replica_id, digest);
                }
            }),
        );
    }

    fn vote_opinion(&self, digest: Digest) -> bool {
        if self.storage[&digest].height <= self.vote_height {
            return false;
        }
        if self.storage[&self.storage[&digest].certified].height
            > self.storage[&self.block_lock].height
        {
            return true;
        }
        let mut block = digest;
        while self.storage[&block].height > self.storage[&self.block_lock].height {
            block = self.storage[&block].parent;
        }
        block == self.block_lock
    }

    fn handle_vote(&mut self, message: Signed<Vote>) {
        let certified = message.inner.digest;
        self.wait_delivered_block(
            certified,
            Box::new(move |self_| {
                let certificate = self_.certificates.entry(certified).or_insert_with(|| {
                    // not proposed by self
                    HashMap::new()
                });
                if certificate.len() >= self_.transport.n - self_.transport.f {
                    return;
                }
                certificate.insert(message.inner.replica_id, message.signature);
                if certificate.len() == self_.transport.n - self_.transport.f {
                    if self_.storage[&certified].height > self_.storage[&self_.certified].height {
                        self_.certified = certified;
                    }
                    self_.on_certificate(&certified);
                }
            }),
        )
    }

    fn do_propose(&mut self) {
        assert_eq!(self.primary_id(), self.id);
        assert_eq!(self.parent, self.certified);
        let generic = Generic {
            certified: self.certified,
            certificate: self.certificates[&self.certified]
                .iter()
                .map(|(&id, signature)| (id, signature.clone()))
                .collect(),
            parent: self.parent,
            requests: self.requests.drain(..).collect(), //
            replica_id: self.id,
        };
        self.transport
            .work({
                let generic = generic.clone();
                move |worker| {
                    worker.send_message_to_all(ToReplica::Generic(Signed::sign(
                        generic,
                        &worker.secret_key,
                    )))
                }
            })
            .detach();

        self.handle_generic(generic); // loopback
    }

    fn do_update(&mut self, digest: Digest) {
        assert_eq!(self.storage[&digest].status, BlockStatus::Voting);
        let block2_digest = self.storage[&digest].certified;
        if self.storage[&block2_digest].status == BlockStatus::Committed {
            return;
        }
        if self.storage[&block2_digest].height > self.storage[&self.certified].height {
            self.certified = block2_digest;
        }
        let block1_digest = self.storage[&block2_digest].certified;
        if self.storage[&block1_digest].status == BlockStatus::Committed {
            return;
        }
        if self.storage[&block1_digest].height > self.storage[&self.block_lock].height {
            self.block_lock = block1_digest;
        }
        let block_digest = self.storage[&block1_digest].certified;
        if self.storage[&block1_digest].status == BlockStatus::Committed {
            return;
        }
        if self.storage[&block2_digest].parent != block1_digest
            || self.storage[&block1_digest].parent != block_digest
        {
            return;
        }

        let mut commit_blocks = Vec::new();
        let mut block = block_digest;
        while self.storage[&block].height > self.storage[&self.block_execute].height {
            commit_blocks.push(block);
            block = self.storage[&block].parent;
        }
        assert_eq!(block, self.block_execute);
        for digest in commit_blocks.into_iter().rev() {
            self.do_execute(digest);
        }
        self.block_execute = block_digest;
    }

    fn do_execute(&mut self, digest: Digest) {
        let block = self.storage.get_mut(&digest).unwrap();
        assert_eq!(block.status, BlockStatus::Voting);
        block.status = BlockStatus::Committed;
        let op_number = block.height; //
        for request in &self.storage[&digest].requests {
            match self.cache.get(&request.client_id) {
                Some((request_number, _)) if request_number > &request.request_number => continue,
                Some((request_number, Some(_))) if request_number == &request.request_number => {
                    continue
                }
                _ => {}
            }
            if let Some((request_number, reply)) = self.cache.get(&request.client_id) {
                if request_number > &request.request_number
                    || request_number == &request.request_number && reply.is_some()
                {
                    continue;
                }
            }
            let result = self.app.execute(op_number, &request.op);
            let reply = Reply {
                request_number: request.request_number,
                result,
                replica_id: self.id,
            };
            self.cache.insert(
                request.client_id,
                (request.request_number, Some(reply.clone())),
            );
            let dest = request.remote();
            self.transport
                .work(move |worker| worker.send_message(dest, reply))
                .detach();
        }
    }

    fn do_vote(&mut self, id: ReplicaId, digest: Digest) {
        let vote = Vote {
            digest,
            replica_id: self.id,
        };
        if id != self.id {
            self.transport
                .work(move |worker| {
                    worker.send_message_to_replica(
                        id,
                        ToReplica::Vote(Signed::sign(vote, &worker.secret_key)),
                    )
                })
                .detach()
        } else {
            let signed_vote = Arc::new(Mutex::new(None));
            self.transport
                .work({
                    let signed_vote = signed_vote.clone();
                    move |worker| {
                        *signed_vote.try_lock().unwrap() =
                            Some(Signed::sign(vote, &worker.secret_key))
                    }
                })
                .then(move |self_| {
                    self_.handle_vote(
                        Arc::try_unwrap(signed_vote)
                            .unwrap()
                            .into_inner()
                            .unwrap()
                            .unwrap(),
                    )
                })
        }
    }

    fn wait_verified_generic(
        &mut self,
        message: Signed<Generic>,
        then: impl FnOnce(&mut Self, Generic) + 'static,
    ) {
        let verified_message = Arc::new(Mutex::new(None));
        self.transport
            .work({
                let verified_message = verified_message.clone();
                move |worker| {
                    let id = message.inner.replica_id;
                    *verified_message.try_lock().unwrap() = message
                        .verify(&worker.config.public_keys[id as usize])
                        .and_then(|message| {
                            message
                                .inner
                                .verify_certificate(worker.config.f, &worker.config.public_keys)
                        });
                }
            })
            .then(move |self_| {
                if let Some(message) = Arc::try_unwrap(verified_message)
                    .unwrap()
                    .into_inner()
                    .unwrap()
                {
                    then(self_, message);
                }
            });
    }

    fn wait_verified_vote(
        &mut self,
        message: Signed<Vote>,
        then: impl FnOnce(&mut Self, Signed<Vote>) + 'static,
    ) {
        let verified_message = Arc::new(Mutex::new(Some(message)));
        let message = verified_message.clone();
        self.transport
            .work({
                move |worker| {
                    let mut verified_message = message.try_lock().unwrap();
                    let message = verified_message.take().unwrap();
                    let id = message.inner.replica_id;
                    if let Some(message) = message.verify(&worker.config.public_keys[id as usize]) {
                        *verified_message = Some(message);
                    }
                }
            })
            .then(move |self_| {
                if let Some(message) = Arc::try_unwrap(verified_message)
                    .unwrap()
                    .into_inner()
                    .unwrap()
                {
                    then(self_, message)
                }
            })
    }

    fn wait_delivered_block(&mut self, digest: Digest, then: Box<dyn FnOnce(&mut Self)>) {
        if let Some(block) = self.storage.get(&digest) {
            if block.status != BlockStatus::Delivering {
                then(self);
                return;
            }
        } else {
            self.waiting_delivered.entry(digest).or_default().push(then);
            return;
        }

        let parent = self.storage[&digest].parent;
        let certified = self.storage[&digest].certified;
        self.wait_delivered_block(
            parent,
            Box::new(move |self_| {
                let parent_height = self_.storage[&parent].height;
                self_.wait_delivered_block(
                    certified,
                    Box::new(move |self_| {
                        let block = self_.storage.get_mut(&digest).unwrap();
                        assert_eq!(block.status, BlockStatus::Delivering);
                        block.status = BlockStatus::Voting;
                        block.height = parent_height + 1;
                        then(self_);
                        self_.on_delivered_block(&digest);
                    }),
                );
            }),
        );
    }

    fn on_delivered_block(&mut self, digest: &Digest) {
        for then in self.waiting_delivered.remove(digest).into_iter().flatten() {
            then(self);
        }
    }

    fn wait_certificate(&mut self, digest: Digest, then: impl FnOnce(&mut Self) + 'static) {
        if digest == GENESIS {
            then(self);
            return;
        }
        if let Some(certificate) = self.certificates.get(&digest) {
            if certificate.len() >= self.transport.n - self.transport.f {
                then(self);
                return;
            }
        }
        self.waiting_certificate
            .entry(digest)
            .or_default()
            .push(Box::new(then));
    }

    fn on_certificate(&mut self, digest: &Digest) {
        for then in self
            .waiting_certificate
            .remove(digest)
            .into_iter()
            .flatten()
        {
            then(self);
        }
    }

    fn wait_request(&mut self, then: impl FnOnce(&mut Self) + 'static) {
        if self.cache.values().any(|(_, reply)| reply.is_none()) {
            then(self);
            return;
        }
        self.waiting_request.push(Box::new(then));
    }

    fn on_request(&mut self) {
        for then in take(&mut self.waiting_request) {
            then(self)
        }
    }

    fn primary_id(&self) -> ReplicaId {
        (self.view_number as usize % self.transport.n) as _
    }
}

impl Drop for Replica {
    fn drop(&mut self) {
        if self.id == self.primary_id() {
            let mut n_proposal = 0;
            let mut n_request = 0;
            for block in self.storage.values() {
                if block.status != BlockStatus::Committed {
                    continue;
                }
                n_proposal += 1;
                n_request += block.requests.len();
            }
            println!(
                "Average batch size {:.2}",
                n_request as f32 / n_proposal as f32
            );
        }
    }
}
