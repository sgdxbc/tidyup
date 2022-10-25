use std::{
    borrow::{Borrow, BorrowMut},
    net::{IpAddr, TcpListener},
};

use bincode::Options;
use messages::ReplicaId;
use serde::{Deserialize, Serialize};
use tidyup::{
    app::{self, App},
    client::{self, LoopClient},
    transport::{Config, TransportReceiver, TransportRuntime},
    unreplicated,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Command {
    config: Config,
    app: AppMode,
    replica: Option<ReplicaCommand>,
    client: Option<ClientCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    let socket = TcpListener::bind(("0.0.0.0", 7000)).unwrap();
    let command = bincode::options()
        .deserialize_from::<_, Command>(socket.accept().unwrap().0)
        .unwrap();
    match (command.replica, command.client) {
        (Some(replica), None) => run_replica(replica, command.config, command.app),
        (None, Some(client)) => match command.app {
            AppMode::Null => run_client(client, command.config, |client| client::Null::new(client)),
        },
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
                    BorrowMut::<unreplicated::Replica>::borrow_mut,
                );
                Self::Unreplicated(unreplicated::Replica::new(transport, app))
            }
        }
    }
}

impl TransportReceiver for Replica {
    fn receive_message(&mut self, message: &[u8]) {
        match self {
            Self::Unreplicated(receiver) => receiver.receive_message(message),
            Self::Pbft(_) => todo!(),
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

impl Client {
    fn new<L, M>(
        command: &ClientCommand,
        runtime: &mut TransportRuntime<M>,
        loop_mut: impl Fn(&mut M) -> &mut L + 'static + Clone,
    ) -> Self
    where
        L: LoopClient<Self>,
    {
        match command.mode {
            ClientMode::Unreplicated => {
                let transport = L::create_transport(runtime, command.ip, loop_mut);
                Self::Unreplicated(unreplicated::Client::new(transport))
            }
        }
    }
}

fn run_client<L>(command: ClientCommand, config: Config, mut new_loop: impl FnMut(Client) -> L)
where
    L: LoopClient<Client>,
{
    let mut clients = Vec::new();
    let mut runtime = TransportRuntime::new(config);
    for i in 0..command.n {
        let client = Client::new(&command, &mut runtime, move |clients: &mut Vec<_>| {
            &mut clients[i]
        });
        clients.push(new_loop(client));
    }
    runtime.run(&mut clients);
}

impl Borrow<unreplicated::Replica> for Replica {
    fn borrow(&self) -> &unreplicated::Replica {
        if let Self::Unreplicated(replica) = self {
            replica
        } else {
            unreachable!()
        }
    }
}

impl BorrowMut<unreplicated::Replica> for Replica {
    fn borrow_mut(&mut self) -> &mut unreplicated::Replica {
        if let Self::Unreplicated(replica) = self {
            replica
        } else {
            unreachable!()
        }
    }
}

impl Borrow<unreplicated::Client> for Client {
    fn borrow(&self) -> &unreplicated::Client {
        if let Self::Unreplicated(client) = self {
            client
        } else {
            unreachable!()
        }
    }
}

impl BorrowMut<unreplicated::Client> for Client {
    fn borrow_mut(&mut self) -> &mut unreplicated::Client {
        if let Self::Unreplicated(client) = self {
            client
        } else {
            unreachable!()
        }
    }
}
