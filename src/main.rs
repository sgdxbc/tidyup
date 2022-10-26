use std::{
    env::args,
    io::Write,
    net::{IpAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread::sleep,
    time::Duration,
};

use bincode::Options;
use messages::ReplicaId;
use serde::{Deserialize, Serialize};
use tidyup::{
    app::{self, App},
    client::{self, LoopClient as _},
    transport::{Config, ReactorMut, Transport, TransportReceiver, TransportRuntime},
    unreplicated,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Command {
    config: Config,
    app: AppMode,
    replica: Option<ReplicaCommand>,
    client: Option<ClientCommand>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum AppMode {
    Null,
    //
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReplicaCommand {
    mode: ReplicaMode,
    id: ReplicaId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ReplicaMode {
    Unreplicated,
    //
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientCommand {
    mode: ClientMode,
    n: usize,
    ip: IpAddr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ClientMode {
    Unreplicated,
    //
}

fn main() {
    if args().nth(1).unwrap_or_default() == "config" {
        //
        let mut command = Command {
            app: AppMode::Null,
            config: Config {
                remotes: vec![([10, 0, 0, 1], 7001).into()],
                f: 0,
                public_keys: vec![()],
                secret_keys: vec![()],
            },
            client: None,
            replica: None,
        };

        command.replica = Some(ReplicaCommand {
            mode: ReplicaMode::Unreplicated,
            id: 0,
        });
        command.client = None;
        TcpStream::connect(("nsl-node1.d1.comp.nus.edu.sg", 7000))
            .unwrap()
            .write_all(&bincode::options().serialize(&command).unwrap())
            .unwrap();

        sleep(Duration::from_secs(1));
        command.replica = None;
        command.client = Some(ClientCommand {
            mode: ClientMode::Unreplicated,
            n: 50,
            ip: [10, 0, 0, 2].into(),
        });
        TcpStream::connect(("nsl-node2.d1.comp.nus.edu.sg", 7000))
            .unwrap()
            .write_all(&bincode::options().serialize(&command).unwrap())
            .unwrap();
        return;
    }

    let socket = TcpListener::bind(("0.0.0.0", 7000)).unwrap();
    println!("Listen on {:?}", socket.local_addr().unwrap());
    let command = bincode::options()
        .deserialize_from::<_, Command>(socket.accept().unwrap().0)
        .unwrap();
    match (command.replica, command.client) {
        (Some(replica), None) => run_replica(replica, command.config, command.app),
        (None, Some(client)) => run_client(client, command.config, command.app),
        _ => unreachable!(),
    }
}

enum Replica {
    Unreplicated(unreplicated::Replica),
    Pbft(()),
}

impl Replica {
    fn new(
        command: ReplicaCommand,
        config: &Config,
        app: Box<dyn App>,
        runtime: &mut TransportRuntime<Self>,
    ) -> Self {
        match command.mode {
            ReplicaMode::Unreplicated => {
                let transport = runtime.create_transport(
                    config.remotes[command.id as usize],
                    command.id,
                    |self_| self_,
                );
                Self::Unreplicated(unreplicated::Replica::new(transport, app))
            }
        }
    }
}

fn run_replica(command: ReplicaCommand, config: Config, app: AppMode) {
    let app = match app {
        AppMode::Null => Box::new(app::Null),
    };
    let mut runtime = TransportRuntime::new(config.clone());
    let mut replica = Replica::new(command, &config, app, &mut runtime);
    runtime.run(&mut replica);
}

enum Client {
    Unreplicated(unreplicated::Client),
    Pbft(()),
}

impl Client {
    fn new<M>(
        command: &ClientCommand,
        runtime: &mut TransportRuntime<M>,
        loop_mut: impl Fn(&mut M) -> &mut LoopClient + 'static + Clone,
    ) -> Self {
        match command.mode {
            ClientMode::Unreplicated => {
                let transport = LoopClient::create_transport(runtime, command.ip, loop_mut);
                Self::Unreplicated(unreplicated::Client::new(transport))
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
}

struct Monitor {
    transport: Transport<Self>,
    n_complete: Arc<AtomicU32>,
    n_reported: u32,
}

impl AsMut<Transport<Self>> for Monitor {
    fn as_mut(&mut self) -> &mut Transport<Self> {
        &mut self.transport
    }
}

impl TransportReceiver for Monitor {
    fn receive_message(&mut self, _message: &[u8]) {
        unreachable!()
    }
}

fn run_client(command: ClientCommand, config: Config, app: AppMode) {
    struct Context {
        clients: Vec<LoopClient>,
        monitor: Monitor,
    }

    let mut runtime = TransportRuntime::new(config);
    let mut transport = runtime.create_transport(
        (command.ip, 0).into(),
        ReplicaId::MAX,
        |context: &mut Context| &mut context.monitor,
    );
    fn on_report(monitor: &mut Monitor) {
        println!(
            "Interval throughput {} ops/sec",
            monitor.n_complete.swap(0, Ordering::SeqCst)
        );
        monitor.n_reported += 1;
        if monitor.n_reported == 10 {
            todo!()
        } else {
            monitor
                .transport
                .create_timeout(Duration::from_secs(1), on_report);
        }
    }
    runtime.create_timeout(&mut transport, Duration::from_secs(1), on_report);
    let n_complete = Arc::new(AtomicU32::new(0));
    let monitor = Monitor {
        transport,
        n_complete: n_complete.clone(),
        n_reported: 0,
    };

    let mut clients = Vec::new();
    for i in 0..command.n {
        let client = Client::new(&command, &mut runtime, move |context: &mut Context| {
            &mut context.clients[i]
        });
        clients.push(LoopClient::new(&app, client, n_complete.clone()));
    }

    let mut context = Context { clients, monitor };
    runtime.run(&mut context);
}

// main part end, below is boilerplate impl

impl TransportReceiver for Replica {
    fn receive_message(&mut self, message: &[u8]) {
        match self {
            Self::Unreplicated(receiver) => receiver.receive_message(message),
            Self::Pbft(_) => todo!(),
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

impl ReactorMut<unreplicated::Client> for Client {
    fn reactor_mut(&mut self) -> &mut unreplicated::Client {
        if let Self::Unreplicated(client) = self {
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

impl tidyup::client::Client for Client {
    fn invoke(&mut self, op: Box<[u8]>) {
        match self {
            Self::Unreplicated(client) => client.invoke(op),
            Self::Pbft(_) => todo!(),
        }
    }

    fn take_result(&mut self) -> Option<Box<[u8]>> {
        match self {
            Self::Unreplicated(client) => client.take_result(),
            Self::Pbft(_) => todo!(),
        }
    }
}

impl TransportReceiver for Client {
    fn receive_message(&mut self, message: &[u8]) {
        match self {
            Self::Unreplicated(client) => client.receive_message(message),
            Self::Pbft(()) => todo!(),
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

impl tidyup::client::LoopClient for LoopClient {}
