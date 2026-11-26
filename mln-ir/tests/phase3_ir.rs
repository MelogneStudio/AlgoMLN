use mln_ir::{dump_ir, lower_default, IrOp};
use mln_lang::parse;

#[test]
fn t3_01_crossover_ir_dump_snapshot() {
    let src = "\
fast = ema(close, 12)
slow = ema(close, 26)
entry = crossover(fast, slow)
exit = crossunder(fast, slow)
";
    let parsed = parse(src);
    assert!(parsed.diags.is_empty());
    let graph = lower_default(&parsed.program).expect("lower succeeds");
    insta::assert_snapshot!("t3_01_crossover", dump_ir(&graph));
}

#[test]
fn t3_02_cse_merges_identical_ema() {
    let src = "\
a = ema(close, 12)
b = ema(close, 12)
entry = a > b
exit = a < b
";
    let parsed = parse(src);
    let graph = lower_default(&parsed.program).expect("lower succeeds");
    let ema_count = graph
        .nodes
        .iter()
        .filter(|n| n.op == IrOp::Builtin("ema"))
        .count();
    assert_eq!(
        ema_count,
        1,
        "expected 1 ema node, got:\n{}",
        dump_ir(&graph)
    );
}

#[test]
fn t3_03_different_windows_produce_two_ema_nodes() {
    let src = "\
a = ema(close, 12)
b = ema(close, 26)
entry = a > b
exit = a < b
";
    let parsed = parse(src);
    let graph = lower_default(&parsed.program).expect("lower succeeds");
    let ema_count = graph
        .nodes
        .iter()
        .filter(|n| n.op == IrOp::Builtin("ema"))
        .count();
    assert_eq!(
        ema_count,
        2,
        "expected 2 ema nodes, got:\n{}",
        dump_ir(&graph)
    );
}
