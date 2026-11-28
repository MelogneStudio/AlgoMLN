use mln_check::{check_default, CheckOutput, CheckedStmt, Ty};
use mln_lang::{parse, Severity};

fn run_check(src: &str) -> CheckOutput {
    let parsed = parse(src);
    assert!(
        parsed.diags.is_empty(),
        "unexpected parse errors for:\n{src}\n{:?}",
        parsed.diags
    );
    check_default(&parsed.program)
}

fn assert_has_code(out: &CheckOutput, code: &str) {
    assert!(
        out.diags.iter().any(|d| d.code == code),
        "expected {code}, got {:?}",
        out.diags
    );
}

#[test]
fn valid_state_on_while_position_and_symbol_program() {
    let src = "\
state peak:
    init = 0.0
    reset = exit
state count:
    init = 0
    reset = close < sma(close, 5)
on enter:
    peak = close
while long:
    peak = max(peak, high)
    count = count + 1
other_daily = symbol(\"INFY\").close @ 1d
stale_sym = symbol(\"INFY\").stale
entry = position.side == \"flat\" and close > other_daily and not stale_sym
exit = position.side == \"long\" and (close < peak * 0.95 or position.bars_held >= 10 or position.pnl < -5.0 or count > 20)
";
    let out = run_check(src);
    assert!(out.is_ok(), "unexpected check errors: {:?}", out.diags);
    assert!(
        !out.diags.iter().any(|d| d.severity == Severity::Warning),
        "unexpected warnings: {:?}",
        out.diags
    );
    assert!(matches!(out.program.stmts[0], CheckedStmt::State { .. }));
    assert!(matches!(out.program.stmts[2], CheckedStmt::On { .. }));
    assert!(matches!(out.program.stmts[3], CheckedStmt::While { .. }));
}

#[test]
fn rejects_mutating_immutable_var_in_on_or_while_with_e0230() {
    let src = "\
limit = 10.0
on enter:
    limit = 20.0
entry = close > open and limit > 0.0
exit = false
";
    let out = run_check(src);
    assert_has_code(&out, "E0230");
}

#[test]
fn rejects_unknown_state_var_and_type_mismatch_in_state_blocks() {
    let unknown_target = "\
on enter:
    missing = 1.0
entry = true
exit = false
";
    assert_has_code(&run_check(unknown_target), "E0200");

    let wrong_assign_ty = "\
state flag:
    init = false
on enter:
    flag = 123
entry = flag
exit = false
";
    assert_has_code(&run_check(wrong_assign_ty), "E0300");

    let wrong_guard_ty = "\
state x:
    init = 0.0
while close:
    x = 1.0
entry = x > 0.0
exit = false
";
    assert_has_code(&run_check(wrong_guard_ty), "E0300");
}

#[test]
fn rejects_bad_position_and_symbol_uses() {
    assert_has_code(
        &run_check("entry = position.unknown > 0\nexit = false"),
        "E0300",
    );
    assert_has_code(
        &run_check("x = position\nentry = true\nexit = false"),
        "E0300",
    );
    assert_has_code(
        &run_check("entry = symbol(\"\").close > 0\nexit = false"),
        "E0303",
    );
    assert_has_code(
        &run_check("entry = symbol(123).close > 0\nexit = false"),
        "E0303",
    );
    assert_has_code(
        &run_check("entry = symbol(\"TCS\").bad_field > 0\nexit = false"),
        "E0300",
    );
    assert_has_code(
        &run_check("s = symbol(\"TCS\")\nentry = true\nexit = false"),
        "E0300",
    );
    assert_has_code(
        &run_check("position = 1\nentry = true\nexit = false"),
        "E0203",
    );
    assert_has_code(
        &run_check("symbol = 1\nentry = true\nexit = false"),
        "E0203",
    );
    assert_has_code(
        &run_check("def f(p) = p.side\nentry = f(position) == \"flat\"\nexit = false"),
        "E0300",
    );
    assert_has_code(
        &run_check("def g(s) = s.close\nentry = g(symbol(\"INFY\")) > 0.0\nexit = false"),
        "E0300",
    );
}

#[test]
fn unused_state_variable_triggers_w0201() {
    let src = "\
state orphan:
    init = 0.0
on enter:
    orphan = close
entry = close > open
exit = close < open
";
    let out = run_check(src);
    assert!(out.is_ok());
    assert_has_code(&out, "W0201");
    assert_ne!(Ty::Position, Ty::Symbol);
}

#[test]
fn self_referencing_state_reset_is_allowed() {
    let src = "\
state count:
    init = 0
    reset = count >= 10
on close > open:
    count = count + 1
entry = count > 0
exit = false
";
    let out = run_check(src);
    assert!(out.is_ok(), "unexpected errors: {:?}", out.diags);
    assert!(out.diags.is_empty(), "unexpected diags: {:?}", out.diags);
}

#[test]
fn state_blocks_propagate_warmups_and_check_clock_compatibility() {
    let warm_src = "\
state s:
    init = 0.0
on close > sma(close, 10):
    s = ema(close, 50)
entry = s > 0.0
exit = false
";
    let warm_out = run_check(warm_src);
    assert!(warm_out.is_ok(), "unexpected errors: {:?}", warm_out.diags);
    assert_eq!(warm_out.program.warmup.max_base_bars_estimate, 49);

    let guard_src = "\
state g:
    init = 0.0
while ema(close, 21) > 0.0:
    g = close
entry = g > 0.0
exit = false
";
    let guard_out = run_check(guard_src);
    assert!(
        guard_out.is_ok(),
        "unexpected errors: {:?}",
        guard_out.diags
    );
    assert_eq!(guard_out.program.warmup.max_base_bars_estimate, 20);

    let clock_err_src = "\
state daily_peak:
    init = close @ 1d
on enter:
    daily_peak = close
entry = daily_peak > 0.0
exit = false
";
    assert_has_code(&run_check(clock_err_src), "E0401");
}
