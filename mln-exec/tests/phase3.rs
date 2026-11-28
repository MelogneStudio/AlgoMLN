use std::fs;
use std::path::Path;

use mln_check::Tf;
use mln_exec::{
    eval_oracle, eval_source_oracle, Bar, ExecError, Executor, SeriesInputs, ValueSeries,
};
use mln_ir::{IrClock, IrConst, IrGraph, IrOp, IrTy, NodeId, SeriesField};
use mln_lang::{CmpOp, TfUnit, UnOp};
use mln_std::{tier1_registry, ReturnTy, TriBool, TIER1_OPS};

fn load_bars() -> SeriesInputs {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("mln-std")
        .join("tests")
        .join("fixtures")
        .join("ref")
        .join("bars.csv");
    let raw = fs::read_to_string(&path).expect("read bars.csv");
    let mut open = Vec::with_capacity(5000);
    let mut high = Vec::with_capacity(5000);
    let mut low = Vec::with_capacity(5000);
    let mut close = Vec::with_capacity(5000);
    let mut volume = Vec::with_capacity(5000);

    for (idx, line) in raw.lines().enumerate() {
        if idx == 0 || line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split(',').collect();
        assert_eq!(cols.len(), 5, "bad row {idx} in bars.csv");
        open.push(parse_csv_f64(cols[0]));
        high.push(parse_csv_f64(cols[1]));
        low.push(parse_csv_f64(cols[2]));
        close.push(parse_csv_f64(cols[3]));
        volume.push(parse_csv_f64(cols[4]));
    }
    SeriesInputs {
        open,
        high,
        low,
        close,
        volume,
    }
}

fn load_ref_col(fixture_path: &str) -> Vec<f64> {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("mln-std")
        .join(fixture_path);
    let raw = fs::read_to_string(&full).expect("read ref fixture");
    raw.lines()
        .enumerate()
        .filter(|(idx, line)| *idx > 0 && !line.trim().is_empty())
        .map(|(_, line)| parse_csv_f64(line.trim()))
        .collect()
}

fn parse_csv_f64(s: &str) -> f64 {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed == "\"\"" || trimmed.eq_ignore_ascii_case("nan") {
        f64::NAN
    } else {
        trimmed.parse::<f64>().expect("valid float")
    }
}

fn op_test_expr(name: &str) -> &'static str {
    match name {
        "abs" => "abs(close - open)",
        "sqrt" => "sqrt(close)",
        "log" => "log(close)",
        "exp" => "exp((close - open) / open)",
        "min" => "min(open, close)",
        "max" => "max(open, close)",
        "where" => "where(close > open, high, low)",
        "clamp" => "clamp(open + 1.0, low, high)",
        "mod" => "mod(close, 5)",
        "diff" => "diff(close, 1)",
        "pct_change" => "pct_change(close, 1)",
        "cumsum" => "cumsum(volume)",
        "sum" => "sum(volume, 20)",
        "mean" => "mean(close, 20)",
        "sma" => "sma(close, 20)",
        "std" => "std(close, 20)",
        "var" => "var(close, 20)",
        "ema" => "ema(close, 12)",
        "rma" => "rma(close, 14)",
        "wma" => "wma(close, 10)",
        "highest" => "highest(high, 20)",
        "lowest" => "lowest(low, 20)",
        "rsi" => "rsi(close, 14)",
        "tr" => "tr(high, low, close)",
        "atr" => "atr(high, low, close, 14)",
        "macd" => "macd(close, 12, 26, 9)",
        "bollinger" => "bollinger(close, 20, 2.0)",
        "corr" => "corr(close, volume, 20)",
        "cov" => "cov(close, volume, 20)",
        "linreg_slope" => "linreg_slope(close, 20)",
        "linreg_intercept" => "linreg_intercept(close, 20)",
        "percentile_rank" => "percentile_rank(close, 20)",
        "crossover" => "crossover(ema(close, 12), ema(close, 26))",
        "crossunder" => "crossunder(ema(close, 12), ema(close, 26))",
        "since" => "since(close > open)",
        other => panic!("missing test expression for `{other}`"),
    }
}

#[test]
fn t3_05_oracle_matches_pandas_numpy_fixtures() {
    let bars = load_bars();
    let reg = tier1_registry();

    for op in TIER1_OPS {
        let expr = op_test_expr(op.name);
        let src = match op.ret {
            ReturnTy::Num => format!("val = {expr}\nentry = val > 0\nexit = false"),
            ReturnTy::Bool => format!("val = {expr}\nentry = val\nexit = false"),
        };
        let (graph, out) = eval_source_oracle(&src, &bars).expect("oracle eval");
        let val_node = graph
            .bindings
            .iter()
            .find(|(k, _)| k == "val")
            .expect("val binding")
            .1;
        let expected = load_ref_col(op.fixture_path);
        assert_eq!(expected.len(), 5000, "fixture length for {}", op.name);

        match op.ret {
            ReturnTy::Num => {
                let actual = out.node_num(val_node).expect("num series");
                assert_eq!(actual.len(), expected.len());
                for (t, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
                    if e.is_nan() {
                        assert!(a.is_nan(), "{} bar {t}: expected NaN, got {a}", op.name);
                    } else {
                        assert!(a.is_finite(), "{} bar {t}: expected {e}, got {a}", op.name);
                        let diff = (a - e).abs();
                        let scale = e.abs().max(1.0);
                        assert!(
                            diff <= 1e-9 * scale,
                            "{} bar {t}: actual={a}, expected={e}, diff={diff}",
                            op.name
                        );
                    }
                }
            }
            ReturnTy::Bool => {
                let actual = out.node_bool(val_node).expect("bool series");
                assert_eq!(actual.len(), expected.len());
                for (t, (&a, &e)) in actual.iter().zip(expected.iter()).enumerate() {
                    let expected_tb = if e.is_nan() {
                        TriBool::Unknown
                    } else if e == 1.0 {
                        TriBool::True
                    } else {
                        TriBool::False
                    };
                    assert_eq!(
                        a, expected_tb,
                        "{} bar {t}: actual={a:?}, expected={expected_tb:?}",
                        op.name
                    );
                }
            }
        }
        assert!(reg.get(op.name).is_some());
    }
}

#[test]
fn t3_06_kleene_truth_tables() {
    // Construct 9 bars covering every combination of (True, False, Unknown) for a and b:
    // close > 0 -> True (1.0), False (-1.0), Unknown (NaN)
    let vals = [
        1.0,
        1.0,
        1.0,
        -1.0,
        -1.0,
        -1.0,
        f64::NAN,
        f64::NAN,
        f64::NAN,
    ];
    let others = [
        1.0,
        -1.0,
        f64::NAN,
        1.0,
        -1.0,
        f64::NAN,
        1.0,
        -1.0,
        f64::NAN,
    ];
    let bars = SeriesInputs {
        open: others.to_vec(),
        high: vals.to_vec(),
        low: vals.to_vec(),
        close: vals.to_vec(),
        volume: vec![1.0; 9],
    };

    let src = "\
a = close > 0
b = open > 0
and_res = a and b
or_res = a or b
not_res = not a
entry = and_res
exit = or_res
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let and_id = graph
        .bindings
        .iter()
        .find(|(k, _)| k == "and_res")
        .unwrap()
        .1;
    let or_id = graph
        .bindings
        .iter()
        .find(|(k, _)| k == "or_res")
        .unwrap()
        .1;
    let not_id = graph
        .bindings
        .iter()
        .find(|(k, _)| k == "not_res")
        .unwrap()
        .1;

    use TriBool::{False as F, True as T, Unknown as U};
    assert_eq!(out.node_bool(and_id).unwrap(), &[T, F, U, F, F, F, U, F, U]);
    assert_eq!(out.node_bool(or_id).unwrap(), &[T, T, T, T, F, U, T, U, U]);
    assert_eq!(out.node_bool(not_id).unwrap(), &[F, F, F, T, T, T, U, U, U]);
    // Unknown program outputs count as 0 signal.
    assert_eq!(out.entry_signals, vec![1, 0, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn t3_07_comparisons_and_arithmetic_with_nan() {
    let bars = SeriesInputs {
        open: vec![1.0, 2.0, f64::NAN],
        high: vec![2.0, f64::NAN, 3.0],
        low: vec![0.5, 1.0, 1.0],
        close: vec![1.5, f64::NAN, 2.0],
        volume: vec![10.0, 10.0, f64::NAN],
    };
    let src = "\
arith = close + open - high * low / volume + (-close)
entry = close > open
exit = close == close
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let arith_id = graph.bindings.iter().find(|(k, _)| k == "arith").unwrap().1;
    let arith = out.node_num(arith_id).unwrap();
    assert!(arith[0].is_finite());
    assert!(arith[1].is_nan());
    assert!(arith[2].is_nan());

    assert_eq!(
        out.entry,
        vec![TriBool::True, TriBool::Unknown, TriBool::Unknown]
    );
    assert_eq!(
        out.exit,
        vec![TriBool::True, TriBool::Unknown, TriBool::True]
    );
}

#[test]
fn t3_08_division_by_zero_gives_nan_never_infinity() {
    let bars = SeriesInputs {
        open: vec![0.0, 0.0, 0.0],
        high: vec![1.0, -1.0, 0.0],
        low: vec![0.0, 0.0, 0.0],
        close: vec![1.0, -1.0, 0.0],
        volume: vec![0.0, 0.0, 0.0],
    };
    let src = "\
div = close / open
entry = div > 0
exit = false
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let div_id = graph.bindings.iter().find(|(k, _)| k == "div").unwrap().1;
    for &v in out.node_num(div_id).unwrap() {
        assert!(v.is_nan(), "expected NaN on 0 divisor, got {v}");
        assert!(!v.is_infinite());
    }
}

#[test]
fn t3_09_crossover_semantics_and_nan_unknown() {
    let bars = SeriesInputs {
        // t=0: a=1, b=2
        // t=1: a=2, b=2 (a0<=b0, a1==b1 -> False)
        // t=2: a=3, b=2 (a1<=b1, a2>b2 -> True!)
        // t=3: a=4, b=3 (a2>b2 -> False)
        // t=4: a=NaN, b=3 -> Unknown
        // t=5: a=5, b=4 (prev a was NaN -> Unknown)
        // t=6: a=2, b=3 (a5>=b5, a6<b6 -> crossunder True!)
        open: vec![2.0, 2.0, 2.0, 3.0, 3.0, 4.0, 3.0],
        high: vec![2.0; 7],
        low: vec![1.0; 7],
        close: vec![1.0, 2.0, 3.0, 4.0, f64::NAN, 5.0, 2.0],
        volume: vec![10.0; 7],
    };
    let src = "\
entry = crossover(close, open)
exit = crossunder(close, open)
";
    let (_, out) = eval_source_oracle(src, &bars).unwrap();
    use TriBool::{False as F, True as T, Unknown as U};
    assert_eq!(out.entry, vec![U, F, T, F, U, U, F]);
    assert_eq!(out.exit, vec![U, F, F, F, U, U, T]);
}

#[test]
fn t3_10_first_non_nan_index_equals_declared_warmup_for_every_op() {
    let bars = load_bars();
    // Use only the clean prefix (first 100 bars have no NaN holes, and volume[0..100] > 0).
    let clean_bars = SeriesInputs {
        open: bars.open[..100].to_vec(),
        high: bars.high[..100].to_vec(),
        low: bars.low[..100].to_vec(),
        close: bars.close[..100].to_vec(),
        volume: bars.volume[..100].to_vec(),
    };

    for op in TIER1_OPS {
        let expr = op_test_expr(op.name);
        let src = match op.ret {
            ReturnTy::Num => format!("val = {expr}\nentry = val > 0\nexit = false"),
            ReturnTy::Bool => format!("val = {expr}\nentry = val\nexit = false"),
        };
        let (graph, out) = eval_source_oracle(&src, &clean_bars).unwrap();
        let val_node = graph.bindings.iter().find(|(k, _)| k == "val").unwrap().1;

        let expected_wu = match op.name {
            "diff" | "pct_change" => op.warmup.compute(&[None, Some(1)]),
            "sum" | "mean" | "sma" | "std" | "var" | "highest" | "lowest" | "bollinger"
            | "percentile_rank" | "linreg_slope" | "linreg_intercept" => {
                op.warmup.compute(&[None, Some(20)])
            }
            "corr" | "cov" => op.warmup.compute(&[None, None, Some(20)]),
            "ema" => op.warmup.compute(&[None, Some(12)]),
            "rma" | "rsi" => op.warmup.compute(&[None, Some(14)]),
            "wma" => op.warmup.compute(&[None, Some(10)]),
            "atr" => op.warmup.compute(&[None, None, None, Some(14)]),
            "macd" => op.warmup.compute(&[None, Some(12), Some(26), Some(9)]),
            "crossover" | "crossunder" => 26, // ema(26) warmup 25 + crossover warmup 1
            _ => op.warmup.compute(&[]),
        } as usize;

        let first_valid = match op.ret {
            ReturnTy::Num => out
                .node_num(val_node)
                .unwrap()
                .iter()
                .position(|v| !v.is_nan())
                .unwrap_or(usize::MAX),
            ReturnTy::Bool => out
                .node_bool(val_node)
                .unwrap()
                .iter()
                .position(|&b| b != TriBool::Unknown)
                .unwrap_or(usize::MAX),
        };
        assert_eq!(
            first_valid, expected_wu,
            "first non-NaN index mismatch for `{}`",
            op.name
        );

        // Also check crossover/crossunder directly on series inputs (`warmup == 1`).
        if matches!(op.name, "crossover" | "crossunder") {
            let direct_src = format!("entry = {}(close, open)\nexit = false", op.name);
            let (_, direct_out) = eval_source_oracle(&direct_src, &clean_bars).unwrap();
            let idx = direct_out
                .entry
                .iter()
                .position(|&b| b != TriBool::Unknown)
                .unwrap();
            assert_eq!(idx, 1, "direct {} warmup should be 1", op.name);
        }
    }
}

#[test]
fn t3_11_rolling_sum_mean_std_match_naive_window() {
    let bars = load_bars();
    let src = "\
s = sum(close, 15)
m = mean(close, 15)
sd = std(close, 15)
entry = sd > 0
exit = false
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let s_id = graph.bindings.iter().find(|(k, _)| k == "s").unwrap().1;
    let m_id = graph.bindings.iter().find(|(k, _)| k == "m").unwrap().1;
    let sd_id = graph.bindings.iter().find(|(k, _)| k == "sd").unwrap().1;

    let actual_s = out.node_num(s_id).unwrap();
    let actual_m = out.node_num(m_id).unwrap();
    let actual_sd = out.node_num(sd_id).unwrap();

    for t in 0..bars.close.len() {
        if t < 14 || bars.close[t - 14..=t].iter().any(|v| !v.is_finite()) {
            assert!(actual_s[t].is_nan());
            assert!(actual_m[t].is_nan());
            assert!(actual_sd[t].is_nan());
        } else {
            let w = &bars.close[t - 14..=t];
            let sum: f64 = w.iter().sum();
            let mean = sum / 15.0;
            let var = w.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 14.0;
            let sd = var.sqrt();
            assert!((actual_s[t] - sum).abs() <= 1e-11 * sum.abs().max(1.0));
            assert!((actual_m[t] - mean).abs() <= 1e-11 * mean.abs().max(1.0));
            assert!((actual_sd[t] - sd).abs() <= 1e-11 * sd.abs().max(1.0));
        }
    }
}

#[test]
fn t3_12_std_adversarial_1e9_with_noise_1() {
    let n = 100;
    let close: Vec<f64> = (0..n)
        .map(|i| 1e9 + (((i * 37) % 11) as f64 - 5.0) * 0.2)
        .collect();
    let bars = SeriesInputs {
        open: close.clone(),
        high: close.clone(),
        low: close.clone(),
        close: close.clone(),
        volume: vec![1.0; n],
    };
    let src = "\
sd = std(close, 20)
entry = sd > 0
exit = false
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let sd_id = graph.bindings.iter().find(|(k, _)| k == "sd").unwrap().1;
    let sd = out.node_num(sd_id).unwrap();

    for t in 19..n {
        let w = &close[t - 19..=t];
        let mean = w.iter().sum::<f64>() / 20.0;
        let expected = (w.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 19.0).sqrt();
        assert!(
            (sd[t] - expected).abs() <= 1e-6 * expected.abs().max(1.0),
            "bar {t}: sd={} expected={expected}",
            sd[t]
        );
    }
}

#[test]
fn t3_13_index_lag_nan_for_first_k_bars() {
    let bars = SeriesInputs {
        open: vec![10.0, 20.0, 30.0, 40.0, 50.0],
        high: vec![10.0; 5],
        low: vec![10.0; 5],
        close: vec![1.0, 2.0, 3.0, 4.0, 5.0],
        volume: vec![1.0; 5],
    };
    let src = "\
lag3 = close[3]
entry = (close > 2)[2]
exit = false
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let lag_id = graph.bindings.iter().find(|(k, _)| k == "lag3").unwrap().1;
    let lag = out.node_num(lag_id).unwrap();
    assert!(lag[0].is_nan() && lag[1].is_nan() && lag[2].is_nan());
    assert_eq!(lag[3], 1.0);
    assert_eq!(lag[4], 2.0);

    use TriBool::{False as F, True as T, Unknown as U};
    assert_eq!(out.entry, vec![U, U, F, F, T]);
}

#[test]
fn t3_14_where_with_unknown_condition_gives_nan() {
    let bars = SeriesInputs {
        open: vec![1.0, 5.0, 2.0],
        high: vec![10.0, 20.0, 30.0],
        low: vec![1.0, 2.0, 3.0],
        close: vec![2.0, 3.0, f64::NAN],
        volume: vec![1.0; 3],
    };
    let src = "\
w = where(close > open, high, low)
entry = w > 0
exit = false
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let w_id = graph.bindings.iter().find(|(k, _)| k == "w").unwrap().1;
    let w = out.node_num(w_id).unwrap();
    assert_eq!(w[0], 10.0);
    assert_eq!(w[1], 2.0);
    assert!(w[2].is_nan());
}

#[test]
fn non_base_clock_returns_unsupported_op() {
    let bars = SeriesInputs {
        open: vec![1.0, 2.0, 3.0],
        high: vec![2.0, 3.0, 4.0],
        low: vec![0.5, 1.5, 2.5],
        close: vec![1.5, 2.5, 3.5],
        volume: vec![10.0, 20.0, 30.0],
    };
    let err = eval_source_oracle("entry = close @ 1d > open @ 1d\nexit = false", &bars)
        .expect_err("non-base clock should be rejected");
    assert!(matches!(err, ExecError::UnsupportedOp(_)), "got {err:?}");
}

#[test]
fn malformed_ir_and_huge_lag_return_error_or_nan_without_panicking() {
    let bars = SeriesInputs {
        open: vec![1.0, 2.0, 3.0],
        high: vec![2.0, 3.0, 4.0],
        low: vec![0.5, 1.5, 2.5],
        close: vec![1.5, 2.5, 3.5],
        volume: vec![10.0, 20.0, 30.0],
    };

    let base = Tf::new(1, TfUnit::Day);

    // Missing input on Unary node must return InvalidNode, not panic.
    let mut missing_input = IrGraph::new(base);
    let n0 = missing_input.intern(
        IrOp::Unary(UnOp::Neg),
        vec![],
        vec![],
        IrClock::Base,
        IrTy::Num,
    );
    missing_input.entry = n0;
    missing_input.exit = n0;
    assert!(matches!(
        eval_oracle(&missing_input, &bars),
        Err(ExecError::InvalidNode(_))
    ));

    // Out-of-bounds NodeId input must return InvalidNode, not panic.
    let mut bad_ref = IrGraph::new(base);
    let b0 = bad_ref.intern(
        IrOp::Unary(UnOp::Neg),
        vec![NodeId(99)],
        vec![],
        IrClock::Base,
        IrTy::Num,
    );
    bad_ref.entry = b0;
    bad_ref.exit = b0;
    assert!(matches!(
        eval_oracle(&bad_ref, &bars),
        Err(ExecError::InvalidNode(_))
    ));

    // Lag >= len (within MAX_BUFFER_LEN) produces all NaN / Unknown cleanly in both engines.
    let mut long_lag = IrGraph::new(base);
    let close = long_lag.intern(
        IrOp::Series(SeriesField::Close),
        vec![],
        vec![],
        IrClock::Base,
        IrTy::Num,
    );
    let lag_num = long_lag.intern(IrOp::Lag(10), vec![close], vec![], IrClock::Base, IrTy::Num);
    let bool_const = long_lag.intern(
        IrOp::Const(IrConst::Bool(true)),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    let lag_bool = long_lag.intern(
        IrOp::Lag(10),
        vec![bool_const],
        vec![],
        IrClock::Base,
        IrTy::Bool,
    );
    long_lag.entry = lag_bool;
    long_lag.exit = lag_bool;
    let out = eval_oracle(&long_lag, &bars).expect("long lag should evaluate cleanly");
    assert!(out.node_num(lag_num).unwrap().iter().all(|v| v.is_nan()));
    assert_eq!(out.entry, vec![TriBool::Unknown; 3]);

    let mut exec = Executor::new(&long_lag).expect("long lag within limit compiles");
    let bar_vec: Vec<Bar> = (0..3)
        .map(|i| {
            Bar::new(
                1_700_000_000 + (i as i64) * 60,
                bars.open[i],
                bars.high[i],
                bars.low[i],
                bars.close[i],
                bars.volume[i],
            )
        })
        .collect();
    let exec_out = exec.run_chunk(&bar_vec).expect("run_chunk");
    assert!(exec_out.iter().all(|o| o.entry == TriBool::Unknown));

    // Lag > MAX_BUFFER_LEN (1_000_001 and u64::MAX) returns InvalidNode in both oracle and Executor.
    for huge_k in [1_000_001_u64, u64::MAX] {
        let mut huge_lag = IrGraph::new(base);
        let c = huge_lag.intern(
            IrOp::Series(SeriesField::Close),
            vec![],
            vec![],
            IrClock::Base,
            IrTy::Num,
        );
        let ln = huge_lag.intern(IrOp::Lag(huge_k), vec![c], vec![], IrClock::Base, IrTy::Num);
        let b = huge_lag.intern(
            IrOp::Const(IrConst::Bool(true)),
            vec![],
            vec![],
            IrClock::Constant,
            IrTy::Bool,
        );
        let lb = huge_lag.intern(
            IrOp::Lag(huge_k),
            vec![b],
            vec![],
            IrClock::Base,
            IrTy::Bool,
        );
        huge_lag.entry = lb;
        huge_lag.exit = lb;
        let _ = ln;
        assert!(matches!(
            eval_oracle(&huge_lag, &bars),
            Err(ExecError::InvalidNode(_))
        ));
        assert!(matches!(
            Executor::new(&huge_lag),
            Err(ExecError::InvalidNode(_))
        ));
    }
}

#[test]
fn const_tf_and_str_series_handled_explicitly() {
    let bars = SeriesInputs {
        open: vec![1.0, 2.0, 3.0],
        high: vec![2.0, 3.0, 4.0],
        low: vec![0.5, 1.5, 2.5],
        close: vec![1.5, 2.5, 3.5],
        volume: vec![10.0, 20.0, 30.0],
    };

    // End-to-end folded constant comparisons still evaluate cleanly.
    let (_, src_out) = eval_source_oracle(
        "a = \"x\"\nentry = (1h == 60m) and (a == \"x\")\nexit = false",
        &bars,
    )
    .expect("source with Tf and Str constants succeeds");
    assert_eq!(src_out.entry, vec![TriBool::True; 3]);

    // Unfolded Compare, IfElse, and Lag on Const(Tf) / Const(Str) nodes in IR.
    let base = Tf::new(1, TfUnit::Day);
    let mut g = IrGraph::new(base);
    let tf_1h = g.intern(
        IrOp::Const(IrConst::Tf(Tf::new(1, TfUnit::Hour))),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Tf,
    );
    let tf_60m = g.intern(
        IrOp::Const(IrConst::Tf(Tf::new(60, TfUnit::Minute))),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Tf,
    );
    let str_a = g.intern(
        IrOp::Const(IrConst::Str("a".into())),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Str,
    );
    let str_b = g.intern(
        IrOp::Const(IrConst::Str("a".into())),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Str,
    );
    let cmp_tf = g.intern(
        IrOp::Compare(CmpOp::Eq),
        vec![tf_1h, tf_60m],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    let cmp_str = g.intern(
        IrOp::Compare(CmpOp::Eq),
        vec![str_a, str_b],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    let entry = g.intern(
        IrOp::And,
        vec![cmp_tf, cmp_str],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    let exit = g.intern(
        IrOp::Const(IrConst::Bool(false)),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    g.entry = entry;
    g.exit = exit;

    let out = eval_oracle(&g, &bars).expect("unfolded Tf/Str comparisons succeed");
    assert_eq!(out.entry, vec![TriBool::True; 3]);
    assert!(matches!(out.values[tf_1h.0 as usize], ValueSeries::Tf(_)));
    assert!(matches!(out.values[str_a.0 as usize], ValueSeries::Str(_)));

    // Lag on Tf/Str returns InvalidNode cleanly instead of producing Num(NaN).
    let mut bad_lag = IrGraph::new(base);
    let s = bad_lag.intern(
        IrOp::Const(IrConst::Str("a".into())),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Str,
    );
    let l = bad_lag.intern(IrOp::Lag(1), vec![s], vec![], IrClock::Base, IrTy::Str);
    bad_lag.entry = l;
    bad_lag.exit = l;
    assert!(matches!(
        eval_oracle(&bad_lag, &bars),
        Err(ExecError::InvalidNode(_))
    ));
    assert!(matches!(
        Executor::new(&bad_lag),
        Err(ExecError::InvalidNode(_))
    ));

    // Runtime IfElse on Tf/Str returns InvalidNode cleanly in both oracle and Executor.
    let mut bad_ifelse = IrGraph::new(base);
    let cond = bad_ifelse.intern(
        IrOp::Const(IrConst::Bool(true)),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    let s1 = bad_ifelse.intern(
        IrOp::Const(IrConst::Str("a".into())),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Str,
    );
    let s2 = bad_ifelse.intern(
        IrOp::Const(IrConst::Str("b".into())),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Str,
    );
    let ie = bad_ifelse.intern(
        IrOp::IfElse,
        vec![cond, s1, s2],
        vec![],
        IrClock::Constant,
        IrTy::Str,
    );
    bad_ifelse.entry = ie;
    bad_ifelse.exit = ie;
    assert!(matches!(
        eval_oracle(&bad_ifelse, &bars),
        Err(ExecError::InvalidNode(_))
    ));
    assert!(matches!(
        Executor::new(&bad_ifelse),
        Err(ExecError::InvalidNode(_))
    ));
}

#[test]
fn ifelse_num_and_bool_branches_with_unknown_condition() {
    let bars = SeriesInputs {
        open: vec![2.0, 5.0, 2.0],
        high: vec![10.0, 20.0, 30.0],
        low: vec![1.0, 2.0, 3.0],
        close: vec![3.0, 4.0, f64::NAN],
        volume: vec![1.0; 3],
    };
    let src = "\
num_branch = high if close > open else low
bool_branch = (high > 15) if close > open else (low < 3)
entry = bool_branch
exit = num_branch > 10
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let num_id = graph
        .bindings
        .iter()
        .find(|(k, _)| k == "num_branch")
        .unwrap()
        .1;
    let nums = out.node_num(num_id).unwrap();
    assert_eq!(nums[0], 10.0);
    assert_eq!(nums[1], 2.0);
    assert!(nums[2].is_nan());

    use TriBool::{False as F, True as T, Unknown as U};
    assert_eq!(out.entry, vec![F, T, U]);
    assert_eq!(out.exit, vec![F, F, U]);
}

#[test]
fn mod_negative_inputs_and_percent_literal() {
    let bars = SeriesInputs {
        open: vec![3.0, -3.0, -3.0, 0.0],
        high: vec![1.0; 4],
        low: vec![1.0; 4],
        close: vec![-7.0, 7.0, -7.0, 5.0],
        volume: vec![1.0; 4],
    };
    let src = "\
m = mod(close, open)
entry = 40% == 0.4
exit = false
";
    let (graph, out) = eval_source_oracle(src, &bars).unwrap();
    let m_id = graph.bindings.iter().find(|(k, _)| k == "m").unwrap().1;
    let m = out.node_num(m_id).unwrap();
    assert_eq!(m[0], 2.0);
    assert_eq!(m[1], -2.0);
    assert_eq!(m[2], -1.0);
    assert!(m[3].is_nan());
    assert_eq!(out.entry, vec![TriBool::True; 4]);
}

#[test]
fn empty_bars_and_length_mismatch_errors() {
    let empty = SeriesInputs {
        open: vec![],
        high: vec![],
        low: vec![],
        close: vec![],
        volume: vec![],
    };
    let (_, out) = eval_source_oracle("entry = ema(close, 5) > 0\nexit = false", &empty).unwrap();
    assert!(out.entry.is_empty());
    assert!(out.exit.is_empty());
    assert!(out.entry_signals.is_empty());
    assert!(out.exit_signals.is_empty());

    let mismatch = SeriesInputs {
        open: vec![1.0, 2.0],
        high: vec![1.0],
        low: vec![1.0, 2.0],
        close: vec![1.0, 2.0],
        volume: vec![1.0, 2.0],
    };
    let err = eval_source_oracle("entry = close > open\nexit = false", &mismatch)
        .expect_err("unequal series lengths must return LengthMismatch");
    assert_eq!(err, ExecError::LengthMismatch);
}
