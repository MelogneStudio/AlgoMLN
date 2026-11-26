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

#[test]
fn def_call_uses_lexical_scoping_not_caller_locals() {
    let out = run_check(
        "def inner(x) = x + secret\n\
         def outer(secret) = inner(secret)\n\
         entry = outer(1) > 0\n\
         exit = true",
    );
    assert_has_code(&out, "E0200");

    let out_default = run_check(
        "def inner(x, y = secret) = x + y\n\
         def outer(secret) = inner(secret)\n\
         entry = outer(1) > 0\n\
         exit = true",
    );
    assert_has_code(&out_default, "E0200");

    let out_later_local = run_check(
        "def inner(x) = x + later\n\
         def outer(x):\n    y = inner(x)\n    later = 1\n    return y + later\n\
         entry = outer(1) > 0\n\
         exit = true",
    );
    assert_has_code(&out_later_local, "E0200");
    assert!(
        !out_later_local.diags.iter().any(|d| d.code == "E0202"),
        "caller local should not trigger E0202 in callee: {:?}",
        out_later_local.diags
    );
}

#[test]
fn mutual_recursion_and_uncalled_defs_are_deterministic() {
    let src = "def a(x) = b(x)\ndef b(x) = a(x)\nentry = true\nexit = false";
    let first = run_check(src);
    assert_has_code(&first, "E0210");
    for _ in 0..20 {
        let again = run_check(src);
        assert_eq!(first.diags, again.diags);
    }

    let uncalled_src =
        "def helper(x) = x + 1\ndef first_u(x) = helper(x)\ndef second_u(x) = helper(x)\nentry = true\nexit = false";
    let first_u = run_check(uncalled_src);
    for _ in 0..20 {
        let again_u = run_check(uncalled_src);
        assert_eq!(first_u.diags, again_u.diags);
        assert_eq!(first_u.program.def_instances, again_u.program.def_instances);
    }
}

#[test]
fn uncalled_polymorphic_and_windowed_defs_do_not_false_error() {
    let out = run_check(
        "def my_ema(x, n) = ema(x, n)\n\
         def lag(x, k) = x[k]\n\
         def both(a, b) = a and b\n\
         entry = close > open\n\
         exit = false",
    );
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    assert_eq!(out.diags.len(), 3);
    assert!(out.diags.iter().all(|d| d.code == "W0201"));

    let bad_name = run_check("def f(x) = x + nope\nentry = true\nexit = false");
    assert_has_code(&bad_name, "E0200");

    let bad_before =
        run_check("def f(x):\n    y = y + 1\n    return x\nentry = true\nexit = false");
    assert_has_code(&bad_before, "E0202");

    let bad_shadow =
        run_check("def f(x):\n    close = 1\n    return x\nentry = true\nexit = false");
    assert_has_code(&bad_shadow, "E0203");

    let bad_default = run_check("def f(x, n = nope) = ema(x, n)\nentry = true\nexit = false");
    assert_has_code(&bad_default, "E0200");
}

#[test]
fn equivalent_timeframes_do_not_trigger_e0401() {
    let parsed_1h = parse("entry = close @ 1h > open\nexit = true");
    let out_60m = check_with_base(&parsed_1h.program, Tf::new(60, TfUnit::Minute));
    assert!(out_60m.is_ok(), "unexpected errors: {:?}", out_60m.diags);

    let parsed_60m = parse("entry = close @ 60m > open\nexit = true");
    let out_1h = check_with_base(&parsed_60m.program, Tf::new(1, TfUnit::Hour));
    assert!(out_1h.is_ok(), "unexpected errors: {:?}", out_1h.diags);

    let nested = run_check("entry = (close @ 1h) @ 60m > open\nexit = true");
    assert!(nested.is_ok(), "unexpected errors: {:?}", nested.diags);

    let cmp_tf = run_check("entry = 1h == 60m and not (1h < 60m)\nexit = true");
    assert!(cmp_tf.is_ok(), "unexpected errors: {:?}", cmp_tf.diags);
}

#[test]
fn later_local_in_def_triggers_e0202_even_when_global_exists() {
    let out = run_check(
        "a = 10\n\
         def f(x):\n    b = a + 1\n    a = x\n    return b\n\
         entry = f(close) > a\n\
         exit = true",
    );
    assert_has_code(&out, "E0202");

    let out_call = run_check(
        "a = 10\n\
         def f(x):\n    b = a()\n    a = x\n    return b\n\
         entry = f(close) > a\n\
         exit = true",
    );
    assert_has_code(&out_call, "E0202");
}

#[test]
fn def_referencing_later_top_level_variable_triggers_e0202() {
    let out = run_check(
        "def f(x) = x + later_top\n\
         entry = f(close) > 0\n\
         later_top = 1\n\
         exit = later_top > 0",
    );
    assert_has_code(&out, "E0202");
    assert!(
        !out.diags.iter().any(|d| d.code == "E0200"),
        "expected E0202 instead of E0200: {:?}",
        out.diags
    );

    let out_call = run_check(
        "def f(x) = later_top(x)\n\
         entry = f(close) > 0\n\
         later_top = 1\n\
         exit = later_top > 0",
    );
    assert_has_code(&out_call, "E0202");
    assert!(
        !out_call.diags.iter().any(|d| d.code == "E0200"),
        "expected E0202 instead of E0200: {:?}",
        out_call.diags
    );
}

#[test]
fn unused_local_inside_def_emits_w0201_once() {
    let out = run_check(
        "def f(x):\n    unused = x + 1\n    return x\n\
         entry = f(close) > f(open)\n\
         exit = true",
    );
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    let w0201s: Vec<_> = out.diags.iter().filter(|d| d.code == "W0201").collect();
    assert_eq!(w0201s.len(), 1, "expected 1 W0201, got {:?}", out.diags);
    assert!(w0201s[0].message.contains("unused"));
}

#[test]
fn default_param_expr_is_checked_even_when_all_calls_pass_arg() {
    let out = run_check(
        "def f(x, n = true + 1) = ema(x, n)\n\
         entry = f(close, 14) > 0\n\
         exit = true",
    );
    assert_has_code(&out, "E0300");

    // When omitted at multiple call sites, the invalid default reports E0300 once.
    let out_omitted = run_check(
        "def f(x, n = true + 1) = ema(x, n)\n\
         entry = f(close) > f(open)\n\
         exit = true",
    );
    let e0300s: Vec<_> = out_omitted
        .diags
        .iter()
        .filter(|d| d.code == "E0300")
        .collect();
    assert_eq!(
        e0300s.len(),
        1,
        "expected 1 E0300, got {:?}",
        out_omitted.diags
    );
}
