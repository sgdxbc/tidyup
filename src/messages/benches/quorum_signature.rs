use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion};
use messages::crypto::{PublicKey, QuorumKey, QuorumSigned, SecretKey, Signed};
use rand::random;

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("Verify");
    for f in [0, 1, 2, 4, 8, 16, 32, 64] {
        group.bench_with_input(BenchmarkId::new("Vec", f), &f, |b, &f| {
            b.iter_batched(
                || {
                    let message = random::<[u8; 32]>();
                    let secret_keys = (0..3 * f + 1)
                        .map(|_| SecretKey::generate_secp256k1())
                        .collect::<Vec<_>>();
                    let quorum_key = QuorumKey::Vec(
                        secret_keys
                            .iter()
                            .map(|k| PublicKey::new_secp256k1(k))
                            .collect::<Vec<_>>(),
                    );
                    let message = QuorumSigned::new(
                        message,
                        secret_keys
                            .iter()
                            .take(2 * f + 1)
                            .enumerate()
                            .map(|(i, k)| (i as _, Signed::sign(message, k).signature)),
                        &quorum_key,
                    );
                    (message, quorum_key)
                },
                |(message, quorum_key)| message.verify(f, &quorum_key).unwrap(),
                BatchSize::SmallInput,
            )
        });
        group.bench_with_input(BenchmarkId::new("Blsttc", f), &f, move |b, &f| {
            b.iter_batched(
                || {
                    let message = random::<[u8; 32]>();
                    let (secret_keys, quorum_key) = SecretKey::generate_blsttc(f);
                    let message = QuorumSigned::new(
                        message,
                        secret_keys
                            .iter()
                            .take(2 * f + 1)
                            .enumerate()
                            .map(|(i, k)| (i as _, Signed::sign(message, k).signature)),
                        &quorum_key,
                    );
                    (message, quorum_key)
                },
                |(message, quorum_key)| message.verify(f, &quorum_key).unwrap(),
                BatchSize::SmallInput,
            )
        });
    }
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
