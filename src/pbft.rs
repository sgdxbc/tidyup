use std::{collections::HashMap, mem::take, net::SocketAddr, sync::Arc, time::Duration};

use bincode::Options;
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use message::{
    pbft::{Commit, PrePrepare, Prepare, ToReplica},
    Reply, Request, Signed, TransportConfig,
};
use secp256k1::{ecdsa::Signature, hashes::sha256, All, PublicKey, Secp256k1, SecretKey};

use crate::{
    core::{ClientCommon, Deploy, ReplicaCommon, RxChannel, SharedState, TxChannel},
    App, ClientState, OptionInstant, State,
};

pub struct Client {
    common: ClientCommon,
    request_number: u32,
    op: Option<Box<[u8]>>,
    results: HashMap<u8, Box<[u8]>>,
    committed_result: Option<Box<[u8]>>,
    resend_instant: OptionInstant,
    view_number: u8,
    buffer: [u8; 1500],
}

impl Client {
    pub fn new(common: ClientCommon) -> Self {
        Self {
            request_number: 0,
            op: None,
            results: Default::default(),
            committed_result: None,
            resend_instant: Default::default(),
            view_number: 0,
            buffer: [0; 1500],
            common,
        }
    }
}

impl ClientState for Client {
    fn invoke(&mut self, op: Box<[u8]>) {
        assert!(self.op.is_none());
        self.request_number += 1;
        self.op = Some(op);
        self.committed_result = None;
        self.do_send();
    }

    fn take_result(&mut self) -> Option<Box<[u8]>> {
        self.committed_result.take()
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
            self.results.insert(0, message.result);
            if self.results.len() > self.common.config.f {
                // TODO check matching results
                self.committed_result = Some(self.results.drain().next().unwrap().1);
                self.op = None;
                self.resend_instant = Default::default();
            }
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
        let request = bincode::options()
            .serialize(&ToReplica::Request(request))
            .unwrap();
        self.common.tx.send_to(
            &request,
            // TODO broadcast on resend
            self.common.config.replica[self.view_number as usize % self.common.config.n],
        );
        self.resend_instant = self.common.clock.after(Duration::from_millis(10));
    }
}

pub struct Replica {
    main: MainThread,
    listen: ListenThread,
    crypto: CryptoThread,
}

impl State for Replica {
    fn poll(&mut self) -> bool {
        let mut poll_again = false;
        poll_again |= self.listen.poll();
        poll_again |= self.crypto.poll();
        poll_again |= self.main.poll();
        poll_again
    }
}

enum ToCrypto {
    Verify(ToReplica),
    SendReply(SocketAddr, Reply), // send only, reply does not need to sign
    // sign and broadcast
    SendPrePrepare(PrePrepare, Box<[Request]>),
    SendPrepare(Prepare),
    SendCommit(Commit),
}

enum ToMain {
    Verified(ToReplica),
    Signed(ToReplica),
}

struct MainThread {
    id: u8,
    config: Arc<TransportConfig>,
    app: App,
    client_table: HashMap<u16, Reply>,
    view_number: u8,
    op_number: u32,
    execute_number: u32,
    log: Vec<LogEntry>,
    requests: Vec<Request>,
    reorder_pre_prepare: HashMap<u32, (Signed<PrePrepare>, Box<[Request]>)>,
    ingress: Receiver<ToMain>,
    to_crypto: Sender<ToCrypto>,
}

struct LogEntry {
    pre_prepare: Signed<PrePrepare>,
    requests: Box<[Request]>,
    prepare_quorum: HashMap<u8, Signature>,
    commit_quorum: HashMap<u8, Signature>,
}

struct ListenThread {
    rx: RxChannel,
    buffer: [u8; 1500],
    to_main: Sender<ToMain>,
    to_crypto: Sender<ToCrypto>,
}

struct CryptoThread {
    secp: Secp256k1<All>,
    public_keys: Box<[PublicKey]>,
    secret_key: SecretKey,
    tx: TxChannel,
    broadcast_addresses: Box<[SocketAddr]>,
    ingress: Receiver<ToCrypto>,
    to_main: Sender<ToMain>,
}

impl Replica {
    pub fn new(common: ReplicaCommon) -> Self {
        let to_main = crossbeam_channel::bounded(1024);
        let to_crypto = crossbeam_channel::bounded(1024);
        let public_keys = common.config.public_keys.clone();
        let secret_key = common.config.secret_keys[common.id];
        let broadcast_addresses = common
            .config
            .replica
            .iter()
            .enumerate()
            .filter_map(|(i, &address)| if i != common.id { Some(address) } else { None })
            .collect();
        Self {
            main: MainThread {
                id: common.id as _,
                config: common.config,
                app: common.app,
                client_table: Default::default(),
                view_number: 0,
                op_number: 0,
                execute_number: 0,
                log: Default::default(),
                requests: Default::default(),
                reorder_pre_prepare: Default::default(),
                ingress: to_main.1,
                to_crypto: to_crypto.0.clone(),
            },
            listen: ListenThread {
                rx: common.rx,
                buffer: [0; 1500],
                to_main: to_main.0.clone(),
                to_crypto: to_crypto.0,
            },
            crypto: CryptoThread {
                secp: Secp256k1::new(),
                public_keys,
                secret_key,
                tx: common.tx,
                broadcast_addresses,
                ingress: to_crypto.1,
                to_main: to_main.0,
            },
        }
    }

    pub fn deploy(self, program: &mut impl Deploy) {
        program.deploy(self.main);
        program.deploy(self.listen);
        program.deploy_shared(self.crypto);
    }
}

impl State for MainThread {
    fn poll(&mut self) -> bool {
        match self.ingress.try_recv() {
            Err(TryRecvError::Empty) => return false,
            Err(TryRecvError::Disconnected) => unreachable!(),
            Ok(ToMain::Verified(ToReplica::Request(message))) => self.handle_request(message),
            Ok(
                ToMain::Verified(ToReplica::PrePrepare(message, requests))
                | ToMain::Signed(ToReplica::PrePrepare(message, requests)),
            ) => {
                // should we check view number before reordering?

                if message.inner.op_number != self.next_entry_number() {
                    self.reorder_pre_prepare
                        .insert(message.inner.op_number, (message, requests));
                } else {
                    self.handle_pre_prepare(message, requests);
                    while let Some((message, requests)) =
                        self.reorder_pre_prepare.remove(&self.next_entry_number())
                    {
                        self.handle_pre_prepare(message, requests)
                    }
                }
            }
            // we don't handle the prepare-before-pre-prepare or even
            // commit-before-pre-prepare reordering
            Ok(
                ToMain::Verified(ToReplica::Prepare(message))
                | ToMain::Signed(ToReplica::Prepare(message)),
            ) => self.handle_prepare(message),
            Ok(
                ToMain::Verified(ToReplica::Commit(message))
                | ToMain::Signed(ToReplica::Commit(message)),
            ) => self.handle_commit(message),
            _ => todo!(),
        }
        true
    }
}

impl MainThread {
    fn handle_request(&mut self, message: Request) {
        if self.id != self.view_number % self.config.n as u8 {
            todo!()
        }
        self.requests.push(message);
        // TODO if too many concurrent proposals postpone pre-prepare
        self.op_number += 1;
        let pre_prepare = PrePrepare {
            view_number: self.view_number,
            op_number: self.op_number,
            request_digest: Default::default(), // will be filled in crypto
        };
        let requests = take(&mut self.requests).into_boxed_slice();
        self.to_crypto
            .send(ToCrypto::SendPrePrepare(pre_prepare, requests))
            .unwrap();
    }

    fn handle_pre_prepare(&mut self, message: Signed<PrePrepare>, requests: Box<[Request]>) {
        if message.inner.view_number != self.view_number {
            return;
        }
        assert_eq!(message.inner.op_number, self.next_entry_number());
        let op_number = message.inner.op_number;
        let request_digest = message.inner.request_digest;
        self.log.push(LogEntry {
            pre_prepare: message,
            requests,
            prepare_quorum: Default::default(),
            commit_quorum: Default::default(),
        });
        if self.id != self.view_number % self.config.n as u8 {
            let prepare = Prepare {
                view_number: self.view_number,
                op_number,
                request_digest,
                replica_id: self.id,
            };
            self.to_crypto.send(ToCrypto::SendPrepare(prepare)).unwrap()
        }
    }

    fn handle_prepare(&mut self, message: Signed<Prepare>) {
        if message.inner.view_number != self.view_number {
            return;
        }
        let Some(entry) = Self::entry_mut(&mut self.log, message.inner.op_number) else {
            //
            return;
        };
        if message.inner.request_digest != entry.pre_prepare.inner.request_digest {
            //
            return;
        }
        if entry.prepare_quorum.len() > 2 * self.config.f {
            return;
        }
        let request_digest = message.inner.request_digest;
        entry
            .prepare_quorum
            .insert(message.inner.replica_id, message.signature);
        if entry.prepare_quorum.len() > 2 * self.config.f {
            let commit = Commit {
                view_number: self.view_number,
                op_number: self.op_number,
                request_digest,
                replica_id: self.id,
            };
            self.to_crypto.send(ToCrypto::SendCommit(commit)).unwrap();
        }
    }

    fn handle_commit(&mut self, message: Signed<Commit>) {
        if message.inner.view_number != self.view_number {
            return;
        }
        let Some(entry) = Self::entry_mut(&mut self.log, message.inner.op_number) else {
            //
            return;
        };
        if message.inner.request_digest != entry.pre_prepare.inner.request_digest {
            //
            return;
        }
        if entry.commit_quorum.len() > 2 * self.config.f {
            return;
        }
        entry
            .commit_quorum
            .insert(message.inner.replica_id, message.signature);
        if entry.commit_quorum.len() > 2 * self.config.f {
            self.execute();
        }
    }

    fn execute(&mut self) {
        let Some(entry) = Self::entry(&self.log, self.execute_number + 1) else {
            return;
        };
        if entry.prepare_quorum.len() > 2 * self.config.f
            && entry.commit_quorum.len() > 2 * self.config.f
        {
            self.execute_number += 1;
            for request in &entry.requests[..] {
                // not sure necessary or not but let's play safe and filter again
                if let Some(reply) = self.client_table.get(&request.client_id) {
                    if reply.request_number >= request.request_number {
                        continue;
                    }
                }
                let result = self.app.execute(self.execute_number, &request.op);
                let reply = Reply {
                    request_number: request.request_number,
                    result,
                    replica_id: self.id,
                    view_number: self.view_number,
                };
                self.client_table.insert(request.client_id, reply.clone());
                self.to_crypto
                    .send(ToCrypto::SendReply(request.addr, reply))
                    .unwrap();
            }
        }
    }

    fn next_entry_number(&self) -> u32 {
        (self.log.len() + 1) as _
    }

    fn entry(log: &[LogEntry], op_number: u32) -> Option<&LogEntry> {
        log.get((op_number - 1) as usize)
    }

    fn entry_mut(log: &mut [LogEntry], op_number: u32) -> Option<&mut LogEntry> {
        log.get_mut((op_number - 1) as usize)
    }
}

impl State for ListenThread {
    fn poll(&mut self) -> bool {
        let Some((len, _)) = self.rx.receive_from(&mut self.buffer) else {
            return false;
        };
        match bincode::options()
            .allow_trailing_bytes()
            .deserialize(&self.buffer[..len])
        {
            Ok(message @ ToReplica::Request(_)) => {
                // request is trusted, so send to main directly as "verified"
                self.to_main.send(ToMain::Verified(message)).unwrap()
            }
            Ok(
                message @ (ToReplica::PrePrepare(_, _)
                | ToReplica::Prepare(_)
                | ToReplica::Commit(_)),
            ) => self.to_crypto.send(ToCrypto::Verify(message)).unwrap(),
            message => {
                println!("! Rejected: {message:?}");
            }
        }
        true
    }
}

impl SharedState for CryptoThread {
    fn shared_poll(&self) -> bool {
        match self.ingress.try_recv() {
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) | Ok(ToCrypto::Verify(ToReplica::Request(_))) => {
                unreachable!()
            }
            Ok(ToCrypto::Verify(ToReplica::PrePrepare(message, requests))) => {
                let request_digest = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                    &bincode::options().serialize(&requests).unwrap(),
                );
                if request_digest.as_ref() != &message.inner.request_digest {
                    //
                    return true;
                }
                if {
                    let signature = &message.signature;
                    let public_key = &self.public_keys
                        [message.inner.view_number as usize % self.public_keys.len()];
                    let message = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                        &bincode::options().serialize(&message).unwrap(),
                    );
                    self.secp.verify_ecdsa(&message, signature, public_key)
                }
                .is_ok()
                {
                    self.to_main
                        .send(ToMain::Verified(ToReplica::PrePrepare(message, requests)))
                        .unwrap()
                } else {
                    //
                }
                true
            }
            Ok(ToCrypto::Verify(ToReplica::Prepare(message))) => {
                if {
                    let signature = &message.signature;
                    let public_key = &self.public_keys[message.inner.replica_id as usize];
                    let message = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                        &bincode::options().serialize(&message).unwrap(),
                    );
                    self.secp.verify_ecdsa(&message, signature, public_key)
                }
                .is_ok()
                {
                    self.to_main
                        .send(ToMain::Verified(ToReplica::Prepare(message)))
                        .unwrap()
                } else {
                    //
                }
                true
            }
            Ok(ToCrypto::Verify(ToReplica::Commit(message))) => {
                if {
                    let signature = &message.signature;
                    let public_key = &self.public_keys[message.inner.replica_id as usize];
                    let message = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                        &bincode::options().serialize(&message).unwrap(),
                    );
                    self.secp.verify_ecdsa(&message, signature, public_key)
                }
                .is_ok()
                {
                    self.to_main
                        .send(ToMain::Verified(ToReplica::Commit(message)))
                        .unwrap()
                } else {
                    //
                }
                true
            }
            Ok(ToCrypto::SendReply(dest, message)) => {
                self.tx
                    .send_to(&bincode::options().serialize(&message).unwrap(), dest);
                true
            }
            Ok(ToCrypto::SendPrePrepare(mut message, requests)) => {
                message.request_digest = *secp256k1::Message::from_hashed_data::<sha256::Hash>(
                    &bincode::options().serialize(&requests).unwrap(),
                )
                .as_ref();
                let signature = {
                    let message = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                        &bincode::options().serialize(&message).unwrap(),
                    );
                    self.secp.sign_ecdsa(&message, &self.secret_key)
                };
                let message = ToReplica::PrePrepare(
                    Signed {
                        inner: message,
                        signature,
                    },
                    requests,
                );
                {
                    let message = bincode::options().serialize(&message).unwrap();
                    for &dest in &self.broadcast_addresses[..] {
                        self.tx.send_to(&message, dest);
                    }
                }
                self.to_main.send(ToMain::Signed(message)).unwrap();
                true
            }
            Ok(ToCrypto::SendPrepare(message)) => {
                let signature = {
                    let message = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                        &bincode::options().serialize(&message).unwrap(),
                    );
                    self.secp.sign_ecdsa(&message, &self.secret_key)
                };
                let message = ToReplica::Prepare(Signed {
                    inner: message,
                    signature,
                });
                {
                    let message = bincode::options().serialize(&message).unwrap();
                    for &dest in &self.broadcast_addresses[..] {
                        self.tx.send_to(&message, dest);
                    }
                }
                self.to_main.send(ToMain::Signed(message)).unwrap();
                true
            }
            Ok(ToCrypto::SendCommit(message)) => {
                let signature = {
                    let message = secp256k1::Message::from_hashed_data::<sha256::Hash>(
                        &bincode::options().serialize(&message).unwrap(),
                    );
                    self.secp.sign_ecdsa(&message, &self.secret_key)
                };
                let message = ToReplica::Commit(Signed {
                    inner: message,
                    signature,
                });
                {
                    let message = bincode::options().serialize(&message).unwrap();
                    for &dest in &self.broadcast_addresses[..] {
                        self.tx.send_to(&message, dest);
                    }
                }
                self.to_main.send(ToMain::Signed(message)).unwrap();
                true
            }
        }
    }
}
