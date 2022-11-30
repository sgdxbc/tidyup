use criterion::{criterion_group, criterion_main, Criterion};
use tidyup_v2::{EffectContext, EffectRunner};

#[derive(Debug, Clone, Copy)]
struct Unit;
impl EffectContext for Unit {
    fn idle_poll(&mut self) -> bool {
        false
    }
}

fn benchmark(c: &mut Criterion) {
    let mut runner = EffectRunner::new([Unit; 4].into_iter());
    c.bench_function("Run effect", |b| {
        b.iter(|| {
            runner.run(|_| {});
        })
    });
}

criterion_group!(benches, benchmark);
criterion_main!(benches);
