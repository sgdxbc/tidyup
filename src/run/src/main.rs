use std::{io::Write, iter::repeat_with, net::TcpStream, thread::sleep, time::Duration};

use bincode::Options;
use message::{AppMode, ClientCommand, Command, ProtocolMode, ReplicaCommand, TransportConfig};
use rand::thread_rng;
use secp256k1::Secp256k1;

fn main() {
    let (secret_keys, public_keys) =
        repeat_with(|| Secp256k1::new().generate_keypair(&mut thread_rng()))
            .take(4)
            .unzip::<_, _, Vec<_>, Vec<_>>();
    let mut command = Command {
        app: AppMode::Null,
        protocol: ProtocolMode::Unreplicated,
        config: TransportConfig {
            replica: Box::new([
                ([10, 0, 0, 1], 7001).into(),
                ([10, 0, 0, 2], 7001).into(),
                ([10, 0, 0, 3], 7001).into(),
                ([10, 0, 0, 4], 7001).into(),
            ]),
            n: 4,
            f: 0,
            public_keys: public_keys.into_boxed_slice(),
            secret_keys: secret_keys.into_boxed_slice(),
        },
        client: None,
        replica: None,
    };

    command.replica = Some(ReplicaCommand { id: 0, n_thread: 15 });
    command.client = None;
    for host in [
        "nsl-node1.d1.comp.nus.edu.sg",
        "nsl-node2.d1.comp.nus.edu.sg",
        "nsl-node3.d1.comp.nus.edu.sg",
    ] {
        TcpStream::connect((host, 7000))
            .unwrap()
            .write_all(&bincode::options().serialize(&command).unwrap())
            .unwrap();
        command.replica.as_mut().unwrap().id += 1;
    }

    sleep(Duration::from_secs(1));
    command.replica = None;
    command.client = Some(ClientCommand {
        n_client: 30.try_into().unwrap(),
        n_thread: 16.try_into().unwrap(),
        ip: [10, 0, 0, 4].into(),
        n_report: 10.try_into().unwrap(),
    });
    TcpStream::connect(("nsl-node4.d1.comp.nus.edu.sg", 7000))
        .unwrap()
        .write_all(&bincode::options().serialize(&command).unwrap())
        .unwrap();
}
