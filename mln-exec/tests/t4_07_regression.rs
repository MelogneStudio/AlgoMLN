use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use mln_exec::{Bar, Executor};
use mln_ir::lower_default;
use mln_lang::parse;

include!("../benches/workload.in");

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
    let hard_target_ms = read_json_f64(&raw, "hard_target_ms");

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
        best_step_ms <= hard_target_ms,
        "T4-07 step hard target exceeded: {best_step_ms:.1} ms > hard_target_ms {hard_target_ms:.1} ms"
    );
    assert!(
        best_chunk_ms <= chunk_limit,
        "T4-07 run_chunk_into regression: {best_chunk_ms:.1} ms > limit {chunk_limit:.1} ms (baseline {baseline_chunk_ms:.1} ms)"
    );
    assert!(
        best_chunk_ms <= hard_target_ms,
        "T4-07 run_chunk_into hard target exceeded: {best_chunk_ms:.1} ms > hard_target_ms {hard_target_ms:.1} ms"
    );
}
