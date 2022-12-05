use std::{env, io::Write, iter::repeat_with, net::TcpStream, thread::sleep, time::Duration};

use bincode::Options;
use message::{AppMode, ClientCommand, Command, ProtocolMode, ReplicaCommand, TransportConfig};
use rand::thread_rng;
use secp256k1::Secp256k1;

const REPLICA_HOSTS: &[&str] = &[
    "127.0.0.1",
    "127.0.0.2",
    "127.0.0.3",
    "127.0.0.4",
];

const CLIENT_HOSTS: &[&str] = &[
    "127.0.0.101",
    //
];

const SYNC_HOSTS: &[&str] = &[
    "127.0.0.1",
    //
];

fn main() {
    match env::args().nth(1).as_deref() {
        Some("export") => {
            for &host in SYNC_HOSTS {
                println!("SYNC {host}");
            }
            for &host in REPLICA_HOSTS {
                println!("RUN_KILL {host}")
            }
            for &host in CLIENT_HOSTS {
                println!("RUN_WAIT {host}");
            }
            return;
        }
        Some("liftoff") => {}
        _ => panic!(),
    }

    println!("[R] * Lift off");
    let (secret_keys, public_keys) =
        repeat_with(|| Secp256k1::new().generate_keypair(&mut thread_rng()))
            .take(4)
            .unzip::<_, _, Vec<_>, Vec<_>>();
    let mut command = Command {
        app: AppMode::Null,
        protocol: ProtocolMode::Pbft,
        config: TransportConfig {
            replica: Box::new([
                ([127, 0, 0, 1], 7001).into(),
                ([127, 0, 0, 2], 7001).into(),
                ([127, 0, 0, 3], 7001).into(),
                ([127, 0, 0, 4], 7001).into(),
            ]),
            n: 4,
            f: 1,
            public_keys: public_keys.into_boxed_slice(),
            secret_keys: secret_keys.into_boxed_slice(),
        },
        client: None,
        replica: None,
    };

    command.replica = Some(ReplicaCommand { id: 0, n_thread: 4 });
    command.client = None;
    for &host in REPLICA_HOSTS {
        TcpStream::connect((host, 7000))
            .unwrap()
            .write_all(&bincode::options().serialize(&command).unwrap())
            .unwrap();
        command.replica.as_mut().unwrap().id += 1;
    }

    sleep(Duration::from_secs(2));
    command.replica = None;
    command.client = Some(ClientCommand {
        n_client: 1.try_into().unwrap(),
        n_thread: 1.try_into().unwrap(),
        ip: [127, 0, 0, 101].into(),
        n_report: 10.try_into().unwrap(),
    });
    for &host in CLIENT_HOSTS {
        TcpStream::connect((host, 7000))
            .unwrap()
            .write_all(&bincode::options().serialize(&command).unwrap())
            .unwrap();
    }
}
