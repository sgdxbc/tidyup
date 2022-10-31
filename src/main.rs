use std::{
    env::args,
    io::Write,
    net::{IpAddr, TcpListener, TcpStream},
    process::id,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc, Barrier,
    },
    thread::{sleep, spawn},
    time::Duration,
};

use messages::{
    crypto::{PublicKey, SecretKey},
    deserialize_from, serialize, ReplicaId,
};
use nix::{
    sched::{sched_setaffinity, CpuSet},
    unistd::Pid,
};
use serde::{Deserialize, Serialize};
use tidyup::{
    app, client, hotstuff,
    transport::{Config, ReactorMut, TransportReceiver, TransportRuntime},
    unreplicated,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Command {
    config: Config,
    app: AppMode,
    protocol: ProtocolMode,
    replica: Option<ReplicaCommand>,
    client: Option<ClientCommand>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AppMode {
    Null,
    //
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ProtocolMode {
    Unreplicated,
    HotStuff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplicaCommand {
    id: ReplicaId,
    n_worker: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientCommand {
    n: usize,
    n_transport: usize,
    ip: IpAddr,
}

fn main() {
    if args().nth(1).unwrap_or_default() == "command" {
        //
        let mut command = Command {
            app: AppMode::Null,
            protocol: ProtocolMode::HotStuff,
            config: Config {
                remotes: vec![
                    ([10, 0, 0, 1], 7001).into(),
                    ([10, 0, 0, 2], 7001).into(),
                    ([10, 0, 0, 3], 7001).into(),
                    ([10, 0, 0, 4], 7001).into(),
                ],
                f: 1,
                public_keys: Vec::new(),
                secret_keys: Vec::new(),
            },
            client: None,
            replica: None,
        };
        for i in 0..command.config.remotes.len() {
            let secret_key = SecretKey::new_secp256k1(i as ReplicaId);
            let public_key = PublicKey::new_secp256k1(&secret_key);
            command.config.public_keys.push(public_key);
            command.config.secret_keys.push(secret_key);
        }

        command.replica = Some(ReplicaCommand {
            id: 0,
            n_worker: 14,
        });
        command.client = None;
        for host in [
            "nsl-node1.d1.comp.nus.edu.sg",
            "nsl-node2.d1.comp.nus.edu.sg",
            "nsl-node3.d1.comp.nus.edu.sg",
        ] {
            TcpStream::connect((host, 7000))
                .unwrap()
                .write_all(&serialize(&command))
                .unwrap();
            command.replica.as_mut().unwrap().id += 1;
        }

        sleep(Duration::from_secs(1));
        command.replica = None;
        command.client = Some(ClientCommand {
            n: 20,
            n_transport: 16,
            ip: [10, 0, 0, 4].into(),
        });
        TcpStream::connect(("nsl-node4.d1.comp.nus.edu.sg", 7000))
            .unwrap()
            .write_all(&serialize(&command))
            .unwrap();
        return;
    }

    println!("Execution ready {}", id());
    let socket = TcpListener::bind(("0.0.0.0", 7000)).unwrap();
    let command = deserialize_from::<Command>(socket.accept().unwrap().0);
    match (command.replica, command.client) {
        (Some(replica), None) => {
            let mut cpu_set = CpuSet::new();
            cpu_set.set(0).unwrap();
            sched_setaffinity(Pid::from_raw(0), &cpu_set).unwrap();

            run_replica(command.protocol, replica, command.config, command.app)
        }
        (None, Some(client)) => {
            let n_complete = Arc::new(AtomicU32::new(0));
            let barrier = Arc::new(Barrier::new(client.n_transport));
            let handles = (0..client.n_transport)
                .map(|i| {
                    let protocol = command.protocol.clone();
                    let client = client.clone();
                    let config = command.config.clone();
                    let n_complete = n_complete.clone();
                    let barrier = barrier.clone();
                    spawn(move || {
                        let mut cpu_set = CpuSet::new();
                        cpu_set.set(i).unwrap();
                        sched_setaffinity(Pid::from_raw(0), &cpu_set).unwrap();

                        run_client(protocol, client, config, command.app, n_complete, barrier);
                    })
                })
                .collect::<Vec<_>>();
            for handle in handles {
                handle.join().unwrap();
            }
        }
        _ => unreachable!(),
    }
}

enum Replica {
    Unreplicated(unreplicated::Replica),
    HotStuff(hotstuff::Replica),
}

impl Replica {
    fn new(
        protocol: &ProtocolMode,
        command: ReplicaCommand,
        config: &Config,
        app: App,
        runtime: &mut TransportRuntime<Self>,
    ) -> Self {
        match protocol {
            ProtocolMode::Unreplicated => {
                let transport = runtime.create_transport(
                    config.remotes[command.id as usize],
                    command.id,
                    command.n_worker,
                    |self_| self_,
                );
                Self::Unreplicated(unreplicated::Replica::new(transport, app))
            }
            ProtocolMode::HotStuff => {
                let transport = runtime.create_transport(
                    config.remotes[command.id as usize],
                    command.id,
                    command.n_worker,
                    |self_| self_,
                );
                Self::HotStuff(hotstuff::Replica::new(transport, command.id, app))
            }
        }
    }
}

enum App {
    Null(app::Null),
}

fn run_replica(protocol: ProtocolMode, command: ReplicaCommand, config: Config, app: AppMode) {
    let app = match app {
        AppMode::Null => App::Null(app::Null),
    };
    let mut runtime = TransportRuntime::new(config.clone());
    let mut replica = Replica::new(&protocol, command, &config, app, &mut runtime);
    runtime.run(&mut replica);
}

enum Client {
    Unreplicated(unreplicated::Client),
    HotStuff(hotstuff::Client),
}

impl Client {
    fn new<M>(
        protocol: &ProtocolMode,
        command: &ClientCommand,
        runtime: &mut TransportRuntime<M>,
        loop_mut: impl Fn(&mut M) -> &mut LoopClient + 'static + Clone,
    ) -> Self {
        match protocol {
            ProtocolMode::Unreplicated => {
                let transport =
                    runtime.create_transport((command.ip, 0).into(), ReplicaId::MAX, 0, loop_mut);
                Self::Unreplicated(unreplicated::Client::new(transport))
            }
            ProtocolMode::HotStuff => {
                let transport =
                    runtime.create_transport((command.ip, 0).into(), ReplicaId::MAX, 0, loop_mut);
                Self::HotStuff(hotstuff::Client::new(transport))
            }
        }
    }
}

enum LoopClient {
    Null(client::Null<Client>),
}

impl LoopClient {
    fn new(app: &AppMode, client: Client, n_complete: Arc<AtomicU32>) -> Self {
        match app {
            AppMode::Null => Self::Null(client::Null::new(client, n_complete)),
        }
    }

    fn initiate(&mut self) {
        match self {
            Self::Null(client) => client.initiate(),
        }
    }
}

fn run_client(
    protocol: ProtocolMode,
    command: ClientCommand,
    config: Config,
    app: AppMode,
    n_complete: Arc<AtomicU32>,
    barrier: Arc<Barrier>,
) {
    struct Context {
        clients: Vec<LoopClient>,
        n_complete: Arc<AtomicU32>,
        n_reported: u32,
    }

    let mut runtime = TransportRuntime::new(config);
    let mut clients = Vec::new();
    for i in 0..command.n {
        let client = Client::new(
            &protocol,
            &command,
            &mut runtime,
            move |context: &mut Context| &mut context.clients[i],
        );
        clients.push(LoopClient::new(&app, client, n_complete.clone()));
    }

    runtime.create_timeout(Duration::ZERO, |context, _| {
        for client in &mut context.clients {
            client.initiate();
        }
    });

    let mut context = Context {
        clients,
        n_complete,
        n_reported: 0,
    };

    fn on_report(context: &mut Context, runtime: &mut TransportRuntime<Context>) {
        println!(
            "Interval throughput {} ops/sec",
            context.n_complete.swap(0, Ordering::SeqCst)
        );
        context.n_reported += 1;
        if context.n_reported == 30 {
            todo!()
        } else {
            runtime.create_timeout(Duration::from_secs(1), on_report);
        }
    }
    if barrier.wait().is_leader() {
        runtime.create_timeout(Duration::from_secs(1), on_report);
    }
    runtime.run(&mut context);
}

// main part end, below is boilerplate impl

impl TransportReceiver for Replica {
    fn receive_message(&mut self, message: &[u8]) {
        match self {
            Self::Unreplicated(receiver) => receiver.receive_message(message),
            Self::HotStuff(receiver) => receiver.receive_message(message),
        }
    }
}

impl tidyup::app::App for App {
    fn execute(&mut self, op_number: messages::OpNumber, op: &[u8]) -> Box<[u8]> {
        match self {
            Self::Null(app) => app.execute(op_number, op),
        }
    }
}

impl tidyup::client::Client for Client {
    fn invoke(&mut self, op: Box<[u8]>) {
        match self {
            Self::Unreplicated(client) => client.invoke(op),
            Self::HotStuff(client) => client.invoke(op),
        }
    }

    fn take_result(&mut self) -> Option<Box<[u8]>> {
        match self {
            Self::Unreplicated(client) => client.take_result(),
            Self::HotStuff(client) => client.take_result(),
        }
    }
}

impl TransportReceiver for Client {
    fn receive_message(&mut self, message: &[u8]) {
        match self {
            Self::Unreplicated(client) => client.receive_message(message),
            Self::HotStuff(client) => client.receive_message(message),
        }
    }
}

impl TransportReceiver for LoopClient {
    fn receive_message(&mut self, message: &[u8]) {
        match self {
            Self::Null(client) => client.receive_message(message),
        }
    }
}

impl ReactorMut<unreplicated::Replica> for Replica {
    fn reactor_mut(&mut self) -> &mut unreplicated::Replica {
        if let Self::Unreplicated(replica) = self {
            replica
        } else {
            unreachable!()
        }
    }
}

impl ReactorMut<hotstuff::Replica> for Replica {
    fn reactor_mut(&mut self) -> &mut hotstuff::Replica {
        if let Self::HotStuff(replica) = self {
            replica
        } else {
            unreachable!()
        }
    }
}

impl ReactorMut<unreplicated::Client> for Client {
    fn reactor_mut(&mut self) -> &mut unreplicated::Client {
        if let Self::Unreplicated(client) = self {
            client
        } else {
            unreachable!()
        }
    }
}

impl ReactorMut<hotstuff::Client> for Client {
    fn reactor_mut(&mut self) -> &mut hotstuff::Client {
        if let Self::HotStuff(client) = self {
            client
        } else {
            unreachable!()
        }
    }
}

impl<T> ReactorMut<T> for LoopClient
where
    Client: ReactorMut<T>,
{
    fn reactor_mut(&mut self) -> &mut T {
        match self {
            Self::Null(client) => client.reactor_mut(),
        }
    }
}
