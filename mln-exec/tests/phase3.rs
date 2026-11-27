use std::fs;
use std::path::Path;

use mln_check::Tf;
use mln_exec::{eval_oracle, eval_source_oracle, ExecError, SeriesInputs};
use mln_ir::{IrClock, IrConst, IrGraph, IrOp, IrTy, NodeId, SeriesField};
use mln_lang::{TfUnit, UnOp};
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
        open: vec![2.0, 2.0, 2.0, 3.0, 3.0, 4.0],
        high: vec![2.0; 6],
        low: vec![1.0; 6],
        close: vec![1.0, 2.0, 3.0, 4.0, f64::NAN, 5.0],
        volume: vec![10.0; 6],
    };
    let src = "\
entry = crossover(close, open)
exit = crossunder(close, open)
";
    let (_, out) = eval_source_oracle(src, &bars).unwrap();
    use TriBool::{False as F, True as T, Unknown as U};
    assert_eq!(out.entry, vec![U, F, T, F, U, U]);
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

    // Huge lag (>= len, including u64::MAX) must produce all NaN / Unknown without panicking.
    let mut huge_lag = IrGraph::new(base);
    let close = huge_lag.intern(
        IrOp::Series(SeriesField::Close),
        vec![],
        vec![],
        IrClock::Base,
        IrTy::Num,
    );
    let lag_num = huge_lag.intern(
        IrOp::Lag(u64::MAX),
        vec![close],
        vec![],
        IrClock::Base,
        IrTy::Num,
    );
    let bool_const = huge_lag.intern(
        IrOp::Const(IrConst::Bool(true)),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Bool,
    );
    let lag_bool = huge_lag.intern(
        IrOp::Lag(u64::MAX),
        vec![bool_const],
        vec![],
        IrClock::Base,
        IrTy::Bool,
    );
    huge_lag.entry = lag_bool;
    huge_lag.exit = lag_bool;
    let out = eval_oracle(&huge_lag, &bars).expect("huge lag should evaluate cleanly");
    assert!(out.node_num(lag_num).unwrap().iter().all(|v| v.is_nan()));
    assert_eq!(out.entry, vec![TriBool::Unknown; 3]);
}
