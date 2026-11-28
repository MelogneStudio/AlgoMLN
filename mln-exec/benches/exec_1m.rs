use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mln_exec::{Bar, Executor};
use mln_ir::lower_default;
use mln_lang::parse;

include!("workload.in");

fn bench_exec_1m(c: &mut Criterion) {
    const N: usize = 1_000_000;
    let parsed = parse(STRATEGY_20_OPS);
    assert!(parsed.diags.is_empty(), "parse failed");
    let ir = lower_default(&parsed.program).expect("lower failed");
    let bars = generate_1m_bars(N);

    let mut group = c.benchmark_group("exec_1m_20_ops");
    group.sample_size(10);
    group.throughput(Throughput::Elements(N as u64));

    group.bench_with_input(BenchmarkId::new("step", N), &bars, |b, bars| {
        b.iter(|| {
            let mut exec = Executor::new(&ir).expect("exec");
            let mut last = None;
            for &bar in bars {
                last = Some(exec.step(black_box(bar)).expect("step"));
            }
            black_box(last)
        });
    });

    group.bench_with_input(BenchmarkId::new("run_chunk_into", N), &bars, |b, bars| {
        let mut out = Vec::with_capacity(N);
        b.iter(|| {
            let mut exec = Executor::new(&ir).expect("exec");
            out.clear();
            exec.run_chunk_into(black_box(bars), &mut out)
                .expect("chunk");
            black_box(out.len())
        });
    });

    group.finish();
}

criterion_group!(benches, bench_exec_1m);
criterion_main!(benches);
