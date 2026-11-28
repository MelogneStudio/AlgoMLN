use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use mln_exec::{Bar, Executor};
use mln_ir::lower_default;
use mln_lang::parse;

const STRATEGY_20_OPS: &str = "\
s = sma(close, 20)
e = ema(close, 20)
r = rma(close, 14)
w = wma(close, 15)
sv = sum(volume, 20)
sd = std(close, 20)
vr = var(close, 20)
hi = highest(high, 20)
lo = lowest(low, 20)
rs = rsi(close, 14)
trv = tr(high, low, close)
atrv = atr(high, low, close, 14)
mc = macd(close, 12, 26, 9)
bb = bollinger(close, 20, 2.0)
co = corr(close, volume, 20)
cv = cov(high, low, 20)
lrs = linreg_slope(close, 20)
lri = linreg_intercept(close, 20)
pc = pct_change(close, 1)
band = clamp(s - e, -10.0, 10.0)
entry = crossover(s, e) or (rs > 55.0 and mc > 0.0 and bb > 0.0 and atrv > 0.0 and sd > 0.0 and vr > 0.0 and co > -2.0 and cv == cv and lrs == lrs and lri == lri and hi >= lo and pc == pc and sv > 0.0 and r > 0.0 and w > 0.0 and trv >= 0.0 and band == band)
exit = crossunder(s, e) or since(s > e) > 12
";

fn generate_1m_bars(count: usize) -> Vec<Bar> {
    let mut bars = Vec::with_capacity(count);
    let mut price = 100.0_f64;
    let mut ts = 1_700_000_000_i64;
    let mut rng = 0x9e37_79b9_7f4a_7c15_u64;
    for i in 0..count {
        rng = rng
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let delta = ((rng >> 32) as i32 % 120) as f64 * 0.01;
        let open = price;
        let close = (price + delta).max(1.0);
        price = close;
        let high = open.max(close) + 0.4 + (i % 5) as f64 * 0.05;
        let low = (open.min(close) - 0.35).max(0.5);
        let volume = 1_000.0 + (i % 2_000) as f64;
        ts += 60;
        bars.push(Bar::new(ts, open, high, low, close, volume));
    }
    bars
}

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
