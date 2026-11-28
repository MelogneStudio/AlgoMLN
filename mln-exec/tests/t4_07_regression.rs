use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

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

fn read_json_f64(raw: &str, key: &str) -> f64 {
    let needle = format!("\"{key}\":");
    let rest = raw
        .split(&needle)
        .nth(1)
        .unwrap_or_else(|| panic!("missing key {key} in baseline.json"));
    let token = rest
        .trim_start()
        .split([',', '}', '\n', '\r'])
        .next()
        .expect("value token");
    token
        .trim()
        .parse::<f64>()
        .unwrap_or_else(|e| panic!("invalid float for {key}: {e}"))
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "T4-07 performance regression gate requires --release"
)]
fn t4_07_one_million_bar_regression_gate() {
    let baseline_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("benches")
        .join("baseline.json");
    let raw = fs::read_to_string(&baseline_path).expect("read baseline.json");
    let baseline_step_ms = read_json_f64(&raw, "step_ms");
    let baseline_chunk_ms = read_json_f64(&raw, "run_chunk_into_ms");
    let max_ratio = read_json_f64(&raw, "max_regression_ratio");

    let parsed = parse(STRATEGY_20_OPS);
    assert!(parsed.diags.is_empty(), "parse failed: {:?}", parsed.diags);
    let ir = lower_default(&parsed.program).expect("lower failed");
    let bars = generate_1m_bars(1_000_000);

    // Warm up once before measuring.
    let mut out = Vec::with_capacity(bars.len());
    let mut warm_exec = Executor::new(&ir).expect("exec");
    warm_exec.run_chunk_into(&bars, &mut out).expect("warm");
    black_box(out.len());

    let mut best_step_ms = f64::INFINITY;
    let mut best_chunk_ms = f64::INFINITY;
    for _ in 0..3 {
        let mut exec = Executor::new(&ir).expect("exec");
        let start = Instant::now();
        let mut last = None;
        for &bar in &bars {
            last = Some(exec.step(black_box(bar)).expect("step"));
        }
        black_box(last);
        best_step_ms = best_step_ms.min(start.elapsed().as_secs_f64() * 1_000.0);

        let mut chunk_exec = Executor::new(&ir).expect("chunk exec");
        out.clear();
        let start = Instant::now();
        chunk_exec
            .run_chunk_into(black_box(&bars), &mut out)
            .expect("chunk");
        black_box(out.len());
        best_chunk_ms = best_chunk_ms.min(start.elapsed().as_secs_f64() * 1_000.0);
    }

    let step_limit = baseline_step_ms * max_ratio;
    let chunk_limit = baseline_chunk_ms * max_ratio;
    assert!(
        best_step_ms <= step_limit,
        "T4-07 step regression: {best_step_ms:.1} ms > limit {step_limit:.1} ms (baseline {baseline_step_ms:.1} ms)"
    );
    assert!(
        best_chunk_ms <= chunk_limit,
        "T4-07 run_chunk_into regression: {best_chunk_ms:.1} ms > limit {chunk_limit:.1} ms (baseline {baseline_chunk_ms:.1} ms)"
    );
}
