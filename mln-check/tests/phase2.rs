use proptest::prelude::*;

use mln_check::{check_default, check_with_base, CheckOutput, Tf};
use mln_lang::{parse, Severity, TfUnit};

fn run_check(src: &str) -> CheckOutput {
    let parsed = parse(src);
    assert!(
        parsed.diags.is_empty(),
        "unexpected parse diagnostics for {src:?}: {:?}",
        parsed.diags
    );
    check_default(&parsed.program)
}

fn assert_has_code(out: &CheckOutput, code: &str) {
    assert!(
        out.diags.iter().any(|d| d.code == code),
        "expected diagnostic {code}, got {:?}",
        out.diags
    );
}

#[test]
fn t2_01_valid_basic_entry_exit() {
    let out = run_check("entry = close > open\nexit = close < open");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    assert!(out.diags.is_empty());
    assert_eq!(out.program.warmup.max_base_bars_estimate, 0);
}

#[test]
fn t2_02_missing_exit() {
    let out = run_check("entry = close > 1");
    assert!(!out.is_ok());
    assert_has_code(&out, "E0220");
}

#[test]
fn t2_03_entry_must_be_bool() {
    let out = run_check("entry = close + 1\nexit = true");
    assert!(!out.is_ok());
    assert_has_code(&out, "E0300");
}

#[test]
fn t2_04_unknown_identifier() {
    let out = run_check("entry = foo > 1\nexit = true");
    assert_has_code(&out, "E0200");
}

#[test]
fn t2_05_use_before_definition() {
    let out = run_check("a = a + 1");
    assert_has_code(&out, "E0202");
}

#[test]
fn t2_06_redefinition() {
    let out = run_check("a = 1\na = 2");
    assert_has_code(&out, "E0201");
}

#[test]
fn t2_07_recursive_def() {
    let out = run_check("def f(x) = f(x)");
    assert_has_code(&out, "E0210");
}

#[test]
fn t2_08_wrong_arity() {
    let out = run_check("entry = ema(close) > 1\nexit = true");
    assert_has_code(&out, "E0301");
}

#[test]
fn t2_09_unknown_keyword_arg() {
    let out = run_check("entry = rsi(close, m=5) > 1\nexit = true");
    assert_has_code(&out, "E0302");
}

#[test]
fn t2_10_argument_type_mismatch() {
    let out = run_check("entry = ema(close, true) > 1\nexit = true");
    assert_has_code(&out, "E0303");
}

#[test]
fn t2_11_negative_index() {
    let out = run_check("x = close[-1]");
    assert_has_code(&out, "E0310");
}

#[test]
fn t2_12_non_constant_vs_constant_index() {
    let bad = run_check("x = close[close]");
    assert_has_code(&bad, "E0311");

    let good = run_check("n = 2\nentry = close[n] > 0\nexit = true");
    assert!(good.is_ok(), "unexpected errors: {:?}", good.diags);
    assert_eq!(
        good.program.warmup.entry.as_ref().unwrap().own_clock_bars,
        2
    );
}

#[test]
fn t2_13_ema_warmup() {
    let out = run_check("entry = ema(close, 50) > 1\nexit = true");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    let entry_wu = out.program.warmup.entry.as_ref().unwrap();
    assert_eq!(entry_wu.own_clock_bars, 49);
    assert_eq!(entry_wu.base_bars_estimate, 49);
}

#[test]
fn t2_14_nested_rsi_ema_warmup() {
    let out = run_check("entry = ema(rsi(close, 14), 9) > 50\nexit = true");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    let entry_wu = out.program.warmup.entry.as_ref().unwrap();
    assert_eq!(entry_wu.own_clock_bars, 22);
    assert_eq!(entry_wu.base_bars_estimate, 22);
}

#[test]
fn t2_15_daily_clock_aligned_to_base() {
    let out = run_check("entry = close @ 1d > open @ 1d\nexit = true");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    let entry_wu = out.program.warmup.entry.as_ref().unwrap();
    assert_eq!(entry_wu.clock, Tf::new(1, TfUnit::Day));
}

#[test]
fn t2_16_daily_warmup_reported_in_own_clock_and_base_estimate() {
    let out = run_check("entry = ema(close @ 1d, 20) > close\nexit = true");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    let entry_wu = out.program.warmup.entry.as_ref().unwrap();
    let d1 = Tf::new(1, TfUnit::Day);
    assert_eq!(entry_wu.by_clock.get(&d1), Some(&19));
    assert_eq!(entry_wu.primary_clock, d1);
    assert_eq!(entry_wu.primary_clock_bars, 19);
    assert_eq!(entry_wu.base_bars_estimate, 19 * 1440);
}

#[test]
fn t2_17_finer_clock_inside_coarser_base_clock() {
    let parsed = parse("entry = close @ 1m > 1\nexit = true");
    let out = check_with_base(&parsed.program, Tf::new(1, TfUnit::Day));
    assert_has_code(&out, "E0401");
}

#[test]
fn t2_18_at_on_constant() {
    let out = run_check("x = 5 @ 1d");
    assert_has_code(&out, "E0402");
}

#[test]
fn t2_19_def_call_sites_get_separate_instances() {
    let out = run_check("def f(x, n=14) = ema(x, n)\nentry = f(close) > f(open)\nexit = true");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    assert_eq!(out.program.def_instances.len(), 2);
    assert_ne!(
        out.program.def_instances[0].id,
        out.program.def_instances[1].id
    );
}

#[test]
fn t2_20_shadowing_builtin() {
    let out = run_check("ema = 1");
    assert_has_code(&out, "E0203");
}

#[test]
fn t2_21_bool_op_type_mismatch() {
    let out = run_check("entry = close and 1\nexit = true");
    assert_has_code(&out, "E0300");
}

#[test]
fn t2_22_unused_variable_warning() {
    let out = run_check("y = 1\nentry = true\nexit = false");
    assert!(out.is_ok());
    assert_eq!(out.diags.len(), 1);
    assert_eq!(out.diags[0].code, "W0201");
    assert_eq!(out.diags[0].severity, Severity::Warning);
}

#[test]
fn t2_23_constant_folding_window() {
    let out = run_check("n = 10 * 2\nentry = ema(close, n) > 1\nexit = true");
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    assert_eq!(
        out.program.warmup.entry.as_ref().unwrap().own_clock_bars,
        19
    );
}

#[test]
fn t2_24_timeframe_plus_number_is_type_error() {
    let out = run_check("x = 1d + 1");
    assert_has_code(&out, "E0300");
}

#[test]
fn t2_25_error_nodes_produce_no_panic_and_no_extra_diagnostics() {
    let p1 = parse("entry = \nexit = true");
    let c1 = check_default(&p1.program);
    assert!(c1.diags.is_empty(), "got extra diags: {:?}", c1.diags);

    let p2 = parse("1 = x");
    let c2 = check_default(&p2.program);
    assert!(c2.diags.is_empty(), "got extra diags: {:?}", c2.diags);

    let p3 = parse("entry = 1 +\nexit = true");
    let c3 = check_default(&p3.program);
    assert!(c3.diags.is_empty(), "got extra diags: {:?}", c3.diags);
}

proptest! {
    #[test]
    fn t2_25_arbitrary_phase1_outputs_never_panic(src in "\\PC*") {
        let parsed = parse(&src);
        let _ = check_default(&parsed.program);
    }
}
