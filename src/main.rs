use std::{
    env::args,
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread::{sleep, spawn},
    time::{Duration, Instant},
};

use bincode::Options;
use message::{AppMode, ClientCommand, Command, ProtocolMode, ReplicaCommand};

use tidyup_v2::{
    program::{bench_client, bench_replica},
    unreplicated, App, TransportConfig, TransportConfig_,
};

fn main() {
    if args().nth(1).unwrap_or_default() == "command" {
        //
        let mut command = Command {
            app: AppMode::Null,
            protocol: ProtocolMode::Unreplicated,
            config: TransportConfig_::from(TransportConfig {
                replica: Box::new([
                    ([10, 0, 0, 1], 7001).into(),
                    ([10, 0, 0, 2], 7001).into(),
                    ([10, 0, 0, 3], 7001).into(),
                    ([10, 0, 0, 4], 7001).into(),
                ]),
                f: 0,
                // public_keys: Vec::new(),
                // secret_keys: Vec::new(),
            }),
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
        return;
    }

    let socket = TcpListener::bind(("0.0.0.0", 7000)).unwrap();
    let command = bincode::options()
        .deserialize_from::<_, Command<TransportConfig_>>(socket.accept().unwrap().0)
        .unwrap();
    let config = Arc::new(TransportConfig::from(command.config));
    match (command.replica, command.client) {
        (Some(replica), None) => {
            let app = match command.app {
                AppMode::Null => App::Null,
            };
            let mut program = bench_replica::Program::default();
            let args = bench_replica::Program::args(config, replica.id, app, replica.n_effect);
            match command.protocol {
                ProtocolMode::Unreplicated => unreplicated::Replica::new(args).deploy(&mut program),
            }
            program.run_until_interrupt();
        }
        (None, Some(client)) => {
            let n_result = Arc::new(AtomicU32::new(0));
            let handles = (0..client.n_thread.get())
                .map(|_| {
                    let config = config.clone();
                    let client = client.clone();
                    let n_result = n_result.clone();
                    spawn(move || match command.protocol {
                        ProtocolMode::Unreplicated => {
                            bench_client::Program::<unreplicated::Client>::new(
                                client.n_client,
                                config,
                                client.ip,
                                Duration::from_secs(client.n_report.get() as _)
                                    + Duration::from_millis(100),
                                n_result,
                            )
                            .run()
                        }
                    })
                })
                .collect::<Vec<_>>();
            let start = Instant::now();
            for _ in 0..client.n_report.get() {
                sleep(Duration::from_secs(1));
                println!(
                    "* [{:6.2?}] Interval throughput {} op/sec",
                    Instant::now() - start,
                    n_result.swap(0, Ordering::SeqCst)
                );
            }
            for handle in handles {
                handle.join().unwrap();
            }
            //     let mut latencies = Arc::try_unwrap(run_client.latencies)
            //         .unwrap()
            //         .into_inner()
            //         .unwrap();
            //     latencies.sort_unstable();
            //     println!(
            //         "Latency 50th {:.3?} 99th {:.3?}",
            //         latencies.get(latencies.len() / 2),
            //         latencies.get(latencies.len() * 99 / 100)
            //     );
        }
        _ => unreachable!(),
    }
}
