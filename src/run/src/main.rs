use std::{io::Write, net::TcpStream, thread::sleep, time::Duration};

use bincode::Options;
use message::{AppMode, ClientCommand, Command, ProtocolMode, ReplicaCommand, TransportConfig};

fn main() {
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
            f: 0,
            // public_keys: Vec::new(),
            // secret_keys: Vec::new(),
        },
        client: None,
        replica: None,
    };
    // for _ in 0..command.config.remotes.len() {
    //     let secret_key = SecretKey::generate_secp256k1();
    //     let public_key = PublicKey::new_secp256k1(&secret_key);
    //     command.config.public_keys.push(public_key);
    //     command.config.secret_keys.push(secret_key);
    // }

    command.replica = Some(ReplicaCommand { id: 0, n_effect: 3 });
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
