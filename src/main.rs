use std::{
    env::var,
    net::TcpListener,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread::{sleep, spawn},
    time::{Duration, Instant},
};

use bincode::Options;
use message::{AppMode, Command, ProtocolMode};

use tidyup_v2::{
    pbft,
    program::{bench_client, replica},
    unreplicated, App,
};

fn main() {
    let host = var("SSH_CONNECTION")
        .map(|s| s.split(' ').nth(2).unwrap().trim().to_owned())
        .unwrap_or(String::from("0.0.0.0"));
    let socket = TcpListener::bind((host, 7000)).unwrap();
    let command = bincode::options()
        .deserialize_from::<_, Command>(socket.accept().unwrap().0)
        .unwrap();
    let config = Arc::new(command.config);
    match (command.replica, command.client) {
        (Some(replica), None) => {
            let app = match command.app {
                AppMode::Null => App::Null,
            };
            let mut program = replica::Program::new(replica.n_thread);
            let args = replica::Program::args(config, replica.id, app);
            match command.protocol {
                ProtocolMode::Unreplicated => unreplicated::Replica::new(args).deploy(&mut program),
                ProtocolMode::Pbft => pbft::Replica::new(args).deploy(&mut program),
            }
            program.run_until_interrupt();
        }
        (None, Some(client)) => {
            let n_result = Arc::new(AtomicU32::new(0));
            let handles = (0..client.n_thread.get())
                .map(|_| {
                    let protocol = command.protocol.clone();
                    let config = config.clone();
                    let client = client.clone();
                    let n_result = n_result.clone();
                    let run_duration = Duration::from_secs(client.n_report.get() as _)
                        + Duration::from_millis(100);
                    spawn(move || match protocol {
                        ProtocolMode::Unreplicated => {
                            bench_client::Program::<unreplicated::Client>::new(
                                client.n_client,
                                config,
                                client.ip,
                                run_duration,
                                n_result,
                            )
                            .run()
                        }
                        ProtocolMode::Pbft => bench_client::Program::<pbft::Client>::new(
                            client.n_client,
                            config,
                            client.ip,
                            run_duration,
                            n_result,
                        )
                        .run(),
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
