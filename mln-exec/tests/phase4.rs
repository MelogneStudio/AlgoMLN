use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::time::Instant;

use mln_check::check_default;
use mln_exec::{
    eval_oracle, Bar, BarOutput, ExecError, Executor, ExecutorSnapshot, NodeVal, SeriesInputs,
    ValueSeries,
};
use mln_ir::{lower_default, IrGraph, IrTy, NodeId};
use mln_lang::parse;
use mln_std::TriBool;
use proptest::prelude::*;

fn compile_src(src: &str) -> IrGraph {
    let parsed = parse(src);
    assert!(
        parsed.diags.is_empty(),
        "parse failed: {:?}\nsrc:\n{src}",
        parsed.diags
    );
    lower_default(&parsed.program).unwrap_or_else(|e| panic!("lower failed: {e:?}\nsrc:\n{src}"))
}

fn bars_to_series(bars: &[Bar]) -> SeriesInputs {
    SeriesInputs {
        open: bars.iter().map(|b| b.open).collect(),
        high: bars.iter().map(|b| b.high).collect(),
        low: bars.iter().map(|b| b.low).collect(),
        close: bars.iter().map(|b| b.close).collect(),
        volume: bars.iter().map(|b| b.volume).collect(),
    }
}

fn assert_num_close(actual: f64, expected: f64, ctx: &str) {
    if expected.is_nan() {
        assert!(actual.is_nan(), "{ctx}: expected NaN, got {actual}");
    } else {
        assert!(
            !actual.is_nan() && actual.is_finite(),
            "{ctx}: expected {expected}, got non-finite {actual}"
        );
        let tol = 1e-6 * expected.abs().max(1.0);
        assert!(
            (actual - expected).abs() <= tol,
            "{ctx}: actual={actual} expected={expected} diff={} tol={tol}",
            (actual - expected).abs()
        );
    }
}

fn assert_exec_matches_oracle(ir: &IrGraph, bars: &[Bar], label: &str) {
    let oracle = eval_oracle(ir, &bars_to_series(bars)).expect("oracle eval");
    let mut exec = Executor::new(ir).expect("executor init");

    for (bar_idx, &bar) in bars.iter().enumerate() {
        let out = exec
            .step_allowing_nans(bar)
            .unwrap_or_else(|e| panic!("{label} bar {bar_idx}: step failed: {e}"));
        assert_eq!(
            out.timestamp, bar.timestamp,
            "{label} bar {bar_idx} timestamp"
        );
        assert_eq!(
            out.entry, oracle.entry[bar_idx],
            "{label} bar {bar_idx} entry"
        );
        assert_eq!(out.exit, oracle.exit[bar_idx], "{label} bar {bar_idx} exit");
        assert_eq!(
            out.entry_signal, oracle.entry_signals[bar_idx],
            "{label} bar {bar_idx} entry_signal"
        );
        assert_eq!(
            out.exit_signal, oracle.exit_signals[bar_idx],
            "{label} bar {bar_idx} exit_signal"
        );

        for node in &ir.nodes {
            let val = exec.node_value(node.id).expect("node value exists");
            match (&oracle.values[node.id.0 as usize], val) {
                (ValueSeries::Num(v), NodeVal::Num(actual)) => {
                    assert_num_close(
                        actual,
                        v[bar_idx],
                        &format!("{label} bar {bar_idx} node {} ({:?})", node.id.0, node.ty),
                    );
                }
                (ValueSeries::Bool(v), NodeVal::Bool(actual)) => {
                    assert_eq!(
                        actual, v[bar_idx],
                        "{label} bar {bar_idx} bool node {}",
                        node.id.0
                    );
                }
                (ValueSeries::Tf(v), NodeVal::Tf(actual)) => {
                    assert_eq!(
                        actual, v[bar_idx],
                        "{label} bar {bar_idx} tf node {}",
                        node.id.0
                    );
                }
                (ValueSeries::Str(_), NodeVal::Str(_)) => {}
                other => panic!(
                    "{label} bar {bar_idx} node {} type mismatch: {other:?}",
                    node.id.0
                ),
            }
        }
    }
}

fn generate_bars_from_spec(
    steps: &[i16],
    spreads: &[u16],
    vols: &[u16],
    nan_flags: &[u8],
    gap_flags: &[u8],
) -> Vec<Bar> {
    let mut price = 100.0_f64;
    let mut ts = 1_700_000_000_i64;
    let mut bars = Vec::with_capacity(steps.len());

    for i in 0..steps.len() {
        let gap = match gap_flags.get(i).copied().unwrap_or(0) % 16 {
            0 => 3_600,
            1 => 86_400,
            _ => 60,
        };
        ts += gap;

        let flat = nan_flags.get(i).copied().unwrap_or(0) & 0x40 != 0;
        let delta = if flat {
            0.0
        } else {
            f64::from(steps[i]) * 0.01
        };
        let open = price;
        let close = (price + delta).max(0.5);
        price = close;

        let spread = if flat {
            0.0
        } else {
            f64::from(spreads[i] % 200) * 0.01
        };
        let high = open.max(close) + spread;
        let low = (open.min(close) - spread * 0.5).max(0.1);
        let volume = if flat { 0.0 } else { f64::from(vols[i]) + 1.0 };

        let mut bar = Bar::new(ts, open, high, low, close, volume);
        let flags = nan_flags.get(i).copied().unwrap_or(0);
        if flags & 0x3f == 1 {
            bar.close = f64::NAN;
        } else if flags & 0x3f == 2 {
            bar.high = f64::NAN;
        } else if flags & 0x3f == 3 {
            bar.low = f64::NAN;
        } else if flags & 0x3f == 4 {
            bar.volume = f64::NAN;
        } else if flags & 0x3f == 5 {
            bar.open = f64::NAN;
        }
        bars.push(bar);
    }

    bars
}

fn clean_bars(len: usize) -> Vec<Bar> {
    let mut bars = Vec::with_capacity(len);
    let mut price = 100.0_f64;
    let mut ts = 1_700_000_000_i64;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for i in 0..len {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let r = ((state >> 32) as i32 % 200) as f64 * 0.01;
        let open = price;
        let close = (price + r).max(1.0);
        price = close;
        let high = open.max(close) + 0.5 + (i % 5) as f64 * 0.1;
        let low = (open.min(close) - 0.4).max(0.5);
        let volume = 1000.0 + ((state >> 16) % 5000) as f64;
        ts += 60;
        bars.push(Bar::new(ts, open, high, low, close, volume));
    }
    bars
}

fn arb_expr_program() -> impl Strategy<Value = String> {
    let series_atom = prop_oneof![
        Just("close".to_string()),
        Just("open".to_string()),
        Just("high".to_string()),
        Just("low".to_string()),
        Just("volume".to_string()),
    ];

    let op1 = (series_atom, 2usize..=12).prop_map(|(s, w)| {
        vec![
            format!("sma({s}, {w})"),
            format!("ema({s}, {w})"),
            format!("rma({s}, {w})"),
            format!("wma({s}, {w})"),
            format!("sum({s}, {w})"),
            format!("std({s}, {w})"),
            format!("var({s}, {w})"),
            format!("highest({s}, {w})"),
            format!("lowest({s}, {w})"),
            format!("rsi({s}, {w})"),
            format!("diff({s}, 1)"),
            format!("pct_change({s}, 1)"),
            format!("percentile_rank({s}, {w})"),
            format!("linreg_slope({s}, {w})"),
            format!("linreg_intercept({s}, {w})"),
            format!("atr(high, low, close, {w})"),
            format!("corr(close, volume, {w})"),
            format!("cov(high, low, {w})"),
            format!("cumsum(diff({s}, 1))"),
        ]
    });

    (op1, 0usize..19, 0usize..19, 1usize..=4).prop_map(|(cands, i1, i2, lag_k)| {
        let e1 = &cands[i1 % cands.len()];
        let e2 = &cands[i2 % cands.len()];
        format!(
            "def rel_spread(x, y) = (x - y) / max(abs(y), 1.0)\n\
             a = {e1}\n\
             b = {e2}\n\
             c = rel_spread(a, b)\n\
             bb = bollinger(close, 8, 2.0)\n\
             mc = macd(close, 4, 8, 3)\n\
             b_ref = b + 0.0005\n\
             d = where(a > b_ref, a[1], bb)\n\
             cond1 = crossover(a, b_ref) or (c > 0.0005 and mc >= 0.0005)\n\
             cond2 = crossunder(a, b_ref) or since(a < b - 0.0005) > {lag_k}\n\
             entry = true if (cond1 and d == d) else false\n\
             exit = cond2 or not cond1\n"
        )
    })
}

fn assert_outputs_and_state_eq(
    a_out: &[BarOutput],
    b_out: &[BarOutput],
    a_exec: &Executor,
    b_exec: &Executor,
    ir: &IrGraph,
    label: &str,
) {
    assert_eq!(a_out.len(), b_out.len(), "{label} len mismatch");
    for (i, (x, y)) in a_out.iter().zip(b_out.iter()).enumerate() {
        assert_eq!(x, y, "{label} output[{i}] mismatch");
    }
    assert_eq!(
        a_exec.snapshot(),
        b_exec.snapshot(),
        "{label} snapshot mismatch"
    );
    for node in &ir.nodes {
        match (a_exec.node_value(node.id), b_exec.node_value(node.id)) {
            (Some(NodeVal::Num(x)), Some(NodeVal::Num(y))) => {
                assert_eq!(x.to_bits(), y.to_bits(), "{label} node {} bits", node.id.0);
            }
            (x, y) => assert_eq!(x, y, "{label} node {}", node.id.0),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn t4_01_differential_random_programs_and_data(
        src in arb_expr_program(),
        len in 30usize..90,
        steps in proptest::collection::vec(-300i16..=300i16, 90),
        spreads in proptest::collection::vec(any::<u16>(), 90),
        vols in proptest::collection::vec(any::<u16>(), 90),
        nan_flags in proptest::collection::vec(any::<u8>(), 90),
        gap_flags in proptest::collection::vec(any::<u8>(), 90),
    ) {
        let ir = compile_src(&src);
        let bars = generate_bars_from_spec(
            &steps[..len],
            &spreads[..len],
            &vols[..len],
            &nan_flags[..len],
            &gap_flags[..len],
        );
        assert_exec_matches_oracle(&ir, &bars, "proptest");
    }
}

#[test]
fn t4_01_all_35_tier1_ops_match_oracle_with_nans_and_flat_bars() {
    let src = "\
x0 = abs(close - open)
x1 = sqrt(high)
x2 = log(close)
x3 = exp(clamp((close - open) / close, -2.0, 2.0))
x4 = min(open, close)
x5 = max(open, close)
x6 = where(close > open, high, low)
x7 = clamp(close, low, high)
x8 = mod(close, 5.0)
x9 = cumsum(abs(close - open))
x10 = diff(close, 2)
x11 = pct_change(close, 1)
x12 = sma(close, 5)
x13 = ema(close, 5)
x14 = rma(close, 5)
x15 = wma(close, 5)
x16 = sum(volume, 5)
x17 = std(close, 6)
x18 = var(close, 6)
x19 = highest(high, 5)
x20 = lowest(low, 5)
x21 = rsi(close, 6)
x22 = tr(high, low, close)
x23 = atr(high, low, close, 5)
x24 = macd(close, 3, 6, 3)
x25 = bollinger(close, 5, 2.0)
x26 = corr(close, volume, 6)
x27 = cov(high, low, 6)
x28 = linreg_slope(close, 6)
x29 = linreg_intercept(close, 6)
x30 = percentile_rank(close, 6)
c1 = crossover(x12, x13)
c2 = crossunder(x12, x13)
s0 = since(c1)
entry = (x24 > -1e9 and x25 > -1e9 and c1) or (x0 + x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10 + x11 + x14 + x15 + x16 + x17 + x18 + x19 + x20 + x21 + x22 + x23 + x26 + x27 + x28 + x29 + x30) > 0.0
exit = c2 or s0 >= 3
";
    let ir = compile_src(src);
    let mut bars = clean_bars(80);
    // Inject flat stretch and NaN holes.
    for b in &mut bars[15..20] {
        b.open = 102.0;
        b.high = 102.0;
        b.low = 102.0;
        b.close = 102.0;
        b.volume = 0.0;
    }
    bars[25].close = f64::NAN;
    bars[41].high = f64::NAN;
    bars[53].volume = f64::NAN;
    assert_exec_matches_oracle(&ir, &bars, "all_35_ops");
}

#[test]
fn t4_02_split_run_snapshot_restore() {
    let src = "\
s = sma(close, 5)
e = ema(close, 8)
r = rsi(close, 7)
a = atr(high, low, close, 6)
m = macd(close, 4, 8, 3)
bb = bollinger(close, 6, 2.0)
st = std(close, 7)
co = corr(close, volume, 7)
lr = linreg_slope(close, 7)
pr = percentile_rank(close, 7)
cs = cumsum(diff(close, 1))
lag = close[3]
entry = crossover(s, e) or (r > 55.0 and m == m and st > 0.0 and co == co and lr == lr and pr >= 0.5 and cs == cs and bb > lag and a > 0.0)
exit = crossunder(s, e) or since(s > e) >= 4
";
    let ir = compile_src(src);
    let bars = clean_bars(120);

    let mut full_exec = Executor::new(&ir).expect("full exec");
    let full_out = full_exec.run_chunk(&bars).expect("full run");

    for split in [0, 1, 5, 11, 45, 119, 120] {
        let mut first = Executor::new(&ir).expect("first exec");
        let mut out_combined = first.run_chunk(&bars[..split]).expect("first half");
        let snap: ExecutorSnapshot = first.snapshot();

        let mut resumed = Executor::new(&ir).expect("resumed exec");
        resumed.restore(&snap).expect("restore");
        let second_out = resumed.run_chunk(&bars[split..]).expect("second half");
        out_combined.extend(second_out);

        assert_outputs_and_state_eq(
            &full_out,
            &out_combined,
            &full_exec,
            &resumed,
            &ir,
            &format!("split={split}"),
        );
    }

    let other_ir = compile_src("entry = close > open\nexit = close < open\n");
    let snap = full_exec.snapshot();
    let mut other_exec = Executor::new(&other_ir).expect("other exec");
    assert_eq!(other_exec.restore(&snap), Err(ExecError::SnapshotMismatch));
}

#[test]
fn t4_03_determinism_output_hash() {
    let src = "\
s = sma(close, 14)
e = ema(close, 14)
st = std(close, 20)
lr = linreg_slope(close, 20)
m = macd(close, 12, 26, 9)
entry = crossover(s, e) and st > 0.0 and lr > -10.0 and m == m
exit = crossunder(s, e) or rsi(close, 14) > 70.0
";
    let ir = compile_src(src);
    let bars = clean_bars(2_000);

    let run_hash = |ir: &IrGraph, bars: &[Bar]| -> u64 {
        let mut exec = Executor::new(ir).expect("exec");
        let mut hasher = DefaultHasher::new();
        for &bar in bars {
            let out = exec.step(bar).expect("step");
            out.timestamp.hash(&mut hasher);
            out.entry_signal.hash(&mut hasher);
            out.exit_signal.hash(&mut hasher);
            let tag = |b: TriBool| match b {
                TriBool::True => 1u8,
                TriBool::False => 2u8,
                TriBool::Unknown => 3u8,
            };
            tag(out.entry).hash(&mut hasher);
            tag(out.exit).hash(&mut hasher);
            for node in &ir.nodes {
                match exec.node_value(node.id).expect("node val") {
                    NodeVal::Num(v) => {
                        let bits = if v.is_nan() {
                            f64::NAN.to_bits()
                        } else {
                            v.to_bits()
                        };
                        bits.hash(&mut hasher);
                    }
                    NodeVal::Bool(b) => match b {
                        TriBool::True => 1u8.hash(&mut hasher),
                        TriBool::False => 2u8.hash(&mut hasher),
                        TriBool::Unknown => 3u8.hash(&mut hasher),
                    },
                    NodeVal::Tf(tf) => format!("{tf:?}").hash(&mut hasher),
                    NodeVal::Str(s) => s.hash(&mut hasher),
                }
            }
        }
        hasher.finish()
    };

    let h1 = run_hash(&ir, &bars);
    let h2 = run_hash(&ir, &bars);
    assert_eq!(h1, h2);
}

#[test]
fn t4_05_warmup_honesty() {
    let cases = [
        ("simple_cmp", "entry = close > open\nexit = high > low\n"),
        ("lag", "entry = close > close[4]\nexit = close < open\n"),
        ("sma", "entry = sma(close, 10) > 0.0\nexit = sma(low, 5) > 0.0\n"),
        ("ema", "entry = ema(close, 10) > 0.0\nexit = rma(close, 7) > 0.0\n"),
        ("rsi", "entry = rsi(close, 14) > 0.0\nexit = rsi(close, 5) >= 0.0\n"),
        (
            "ema_of_rsi",
            "entry = ema(rsi(close, 7), 5) >= 0.0\nexit = sma(rsi(close, 5), 4) >= 0.0\n",
        ),
        (
            "tr_atr",
            "entry = tr(high, low, close) >= 0.0\nexit = atr(high, low, close, 10) > 0.0\n",
        ),
        (
            "macd_bb",
            "entry = macd(close, 4, 8, 3) == macd(close, 4, 8, 3)\nexit = bollinger(close, 6, 2.0) > -1e9\n",
        ),
        (
            "std_corr_linreg",
            "entry = std(close, 9) >= 0.0\nexit = corr(close, volume, 8) == corr(close, volume, 8) and linreg_slope(close, 6) == linreg_slope(close, 6)\n",
        ),
        (
            "crossover",
            "entry = crossover(sma(close, 5), ema(close, 9))\nexit = since(close > open) >= 0\n",
        ),
    ];

    let bars = clean_bars(80);
    for (name, src) in cases {
        let parsed = parse(src);
        assert!(
            parsed.diags.is_empty(),
            "{name}: parse failed: {:?}",
            parsed.diags
        );
        let checked = check_default(&parsed.program);
        assert!(checked.is_ok(), "{name}: check failed: {:?}", checked.diags);
        let ir = lower_default(&parsed.program).expect("lower");
        let mut exec = Executor::new(&ir).expect("exec");
        let out = exec.run_chunk(&bars).expect("run");

        let actual_entry_warmup = out
            .iter()
            .position(|o| !matches!(o.entry, TriBool::Unknown))
            .expect("entry exits warmup");
        let actual_exit_warmup = out
            .iter()
            .position(|o| !matches!(o.exit, TriBool::Unknown))
            .expect("exit exits warmup");

        assert_eq!(
            actual_entry_warmup,
            checked
                .program
                .warmup
                .entry
                .as_ref()
                .expect("entry warmup")
                .base_bars_estimate as usize,
            "{name}: entry warmup mismatch"
        );
        assert_eq!(
            actual_exit_warmup,
            checked
                .program
                .warmup
                .exit
                .as_ref()
                .expect("exit warmup")
                .base_bars_estimate as usize,
            "{name}: exit warmup mismatch"
        );
    }
}

#[test]
fn t4_06_constant_time_operators_stay_flat() {
    let measure_ns_per_bar = |src: &str, warmup: usize, measure_bars: usize| -> f64 {
        let ir = compile_src(src);
        let warm_data = clean_bars(warmup);
        let bench_data = clean_bars(measure_bars);

        let mut best_ns = f64::INFINITY;
        for _ in 0..3 {
            let mut exec = Executor::new(&ir).expect("exec");
            let mut out = Vec::with_capacity(bench_data.len().max(warm_data.len()));
            exec.run_chunk_into(&warm_data, &mut out).expect("warmup");
            out.clear();
            let start = Instant::now();
            exec.run_chunk_into(&bench_data, &mut out).expect("measure");
            let elapsed = start.elapsed();
            black_box(&out);
            let ns_per_bar = elapsed.as_secs_f64() * 1e9 / (measure_bars as f64);
            if ns_per_bar < best_ns {
                best_ns = ns_per_bar;
            }
        }
        best_ns
    };

    let cases = [
        (
            "sma",
            "entry = sma(close, 10) > 0.0\nexit = false\n",
            "entry = sma(close, 10000) > 0.0\nexit = false\n",
        ),
        (
            "std",
            "entry = std(close, 10) >= 0.0\nexit = false\n",
            "entry = std(close, 10000) >= 0.0\nexit = false\n",
        ),
        (
            "corr",
            "entry = corr(close, volume, 10) > -2.0\nexit = false\n",
            "entry = corr(close, volume, 10000) > -2.0\nexit = false\n",
        ),
        (
            "linreg_slope",
            "entry = linreg_slope(close, 10) > -1e9\nexit = false\n",
            "entry = linreg_slope(close, 10000) > -1e9\nexit = false\n",
        ),
    ];

    for (name, small_src, large_src) in cases {
        let t_small = measure_ns_per_bar(small_src, 10_050, 20_000);
        let t_large = measure_ns_per_bar(large_src, 10_050, 20_000);
        let ratio = t_large / t_small;
        assert!(
            ratio <= 2.0,
            "{name}: window 10000 ({t_large:.1} ns/bar) vs window 10 ({t_small:.1} ns/bar), ratio={ratio:.2}x > 2.0x"
        );
    }
}

#[test]
fn t4_08_bad_bars_rejected_state_unchanged() {
    let src = "\
s = sma(close, 4)
e = ema(close, 4)
r = rsi(close, 4)
entry = s > e and r > 0.0
exit = crossunder(s, e)
";
    let ir = compile_src(src);
    let good_bars = clean_bars(20);

    let mut clean_exec = Executor::new(&ir).expect("clean exec");
    let clean_out = clean_exec.run_chunk(&good_bars).expect("clean run");

    let mut noisy_exec = Executor::new(&ir).expect("noisy exec");
    let mut noisy_out = noisy_exec.run_chunk(&good_bars[..10]).expect("prefix");
    let snap_before = noisy_exec.snapshot();

    let bad_bars = [
        Bar::new(1_700_001_000, 100.0, 99.0, 101.0, 100.0, 10.0), // high < low
        Bar::new(1_700_001_060, 100.0, 102.0, 99.0, 101.0, -0.01), // negative volume
        Bar::new(1_700_001_120, f64::NAN, 102.0, 99.0, 101.0, 10.0),
        Bar::new(1_700_001_180, 100.0, f64::NAN, 99.0, 101.0, 10.0),
        Bar::new(1_700_001_240, 100.0, 102.0, f64::NAN, 101.0, 10.0),
        Bar::new(1_700_001_300, 100.0, 102.0, 99.0, f64::NAN, 10.0),
        Bar::new(1_700_001_360, 100.0, 102.0, 99.0, 101.0, f64::NAN),
        Bar::new(1_700_001_420, 100.0, f64::INFINITY, 99.0, 101.0, 10.0),
        Bar::new(1_700_001_480, 100.0, 102.0, f64::NEG_INFINITY, 101.0, 10.0),
    ];

    for bad in bad_bars {
        let err = noisy_exec.step(bad).unwrap_err();
        assert!(
            matches!(err, ExecError::InvalidBar(_)),
            "unexpected err: {err:?}"
        );
        assert_eq!(noisy_exec.bar_count(), 10);
        assert_eq!(noisy_exec.snapshot(), snap_before);
    }

    noisy_out.extend(noisy_exec.run_chunk(&good_bars[10..]).expect("suffix"));
    assert_outputs_and_state_eq(
        &clean_out,
        &noisy_out,
        &clean_exec,
        &noisy_exec,
        &ir,
        "bad_bar",
    );
}

#[test]
fn t4_09_timestamp_gaps_passed_through() {
    let src = "entry = sma(close, 3) > 100.0\nexit = close < open\n";
    let ir = compile_src(src);
    let gapped = vec![
        Bar::new(1_700_000_000, 100.0, 102.0, 99.0, 101.0, 50.0),
        Bar::new(1_700_000_060, 101.0, 103.0, 100.0, 102.0, 50.0),
        Bar::new(1_700_003_600, 102.0, 104.0, 101.0, 103.0, 50.0), // +1 hour gap
        Bar::new(1_700_090_000, 103.0, 105.0, 102.0, 104.0, 50.0), // +24 hour gap
        Bar::new(1_700_694_800, 104.0, 106.0, 103.0, 105.0, 50.0), // +7 day gap
    ];

    let mut exec = Executor::new(&ir).expect("exec");
    let out = exec.run_chunk(&gapped).expect("run");

    assert_eq!(out.len(), gapped.len());
    assert_eq!(exec.bar_count() as usize, gapped.len());
    for (o, b) in out.iter().zip(gapped.iter()) {
        assert_eq!(o.timestamp, b.timestamp);
    }
    assert_eq!(out[0].entry, TriBool::Unknown);
    assert_eq!(out[1].entry, TriBool::Unknown);
    assert_eq!(out[2].entry, TriBool::True);
    assert_eq!(out[3].entry, TriBool::True);
    assert_eq!(out[4].entry, TriBool::True);
}

#[test]
fn t4_10_chunked_equals_bar_by_bar() {
    let src = "\
s = sma(close, 5)
e = ema(close, 8)
st = std(close, 6)
p = percentile_rank(close, 6)
h = highest(high, 5)
l = lowest(low, 5)
entry = crossover(s, e) or (st > 0.0 and p >= 0.5 and h > l)
exit = crossunder(s, e)
";
    let ir = compile_src(src);
    let bars = clean_bars(100);

    let mut step_exec = Executor::new(&ir).expect("step exec");
    let mut step_out = Vec::with_capacity(bars.len());
    for &b in &bars {
        step_out.push(step_exec.step(b).expect("step"));
    }

    for chunk_size in [1, 2, 7, 13, 100] {
        let mut chunk_exec = Executor::new(&ir).expect("chunk exec");
        let mut chunk_out = Vec::with_capacity(bars.len());
        let mut scratch = Vec::new();
        for chunk in bars.chunks(chunk_size) {
            scratch.clear();
            chunk_exec
                .run_chunk_into(chunk, &mut scratch)
                .expect("run_chunk_into");
            chunk_out.extend_from_slice(&scratch);
        }
        assert_outputs_and_state_eq(
            &step_out,
            &chunk_out,
            &step_exec,
            &chunk_exec,
            &ir,
            &format!("chunk={chunk_size}"),
        );
    }

    assert!(step_exec.node_value(NodeId(0)).is_some());
    assert_ne!(ir.nodes[0].ty, IrTy::Str);
}

#[test]
fn executor_rejects_mismatched_node_ids_and_inconsistent_op_types() {
    let mut bad_id = compile_src("entry = close > open\nexit = false");
    bad_id.nodes[0].id = NodeId(2);
    assert!(matches!(
        Executor::new(&bad_id),
        Err(ExecError::InvalidNode(_))
    ));

    let mut non_topo = compile_src("entry = close > open\nexit = false");
    non_topo.nodes[0].id = NodeId(2);
    non_topo.nodes[2].inputs[0] = NodeId(1);
    assert!(matches!(
        Executor::new(&non_topo),
        Err(ExecError::InvalidNode(_))
    ));

    let mut bad_series_ty = compile_src("entry = close > open\nexit = false");
    bad_series_ty.nodes[0].ty = IrTy::Bool;
    assert!(matches!(
        Executor::new(&bad_series_ty),
        Err(ExecError::InvalidNode(_))
    ));

    let mut bad_builtin_ty = compile_src("entry = sma(close, 3) > 0\nexit = false");
    let sma_idx = bad_builtin_ty
        .nodes
        .iter()
        .position(|n| matches!(n.op, mln_ir::IrOp::Builtin("sma")))
        .expect("sma node");
    bad_builtin_ty.nodes[sma_idx].ty = IrTy::Bool;
    assert!(matches!(
        Executor::new(&bad_builtin_ty),
        Err(ExecError::InvalidNode(_))
    ));
}
