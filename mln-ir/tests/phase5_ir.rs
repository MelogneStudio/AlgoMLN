use mln_ir::{dump_ir, lower_default, IrClock, IrExecStep, IrOp, IrReset};
use mln_lang::parse;

#[test]
fn lowers_state_on_while_position_and_symbol_with_tf() {
    let src = "\
state peak:
    init = 0.0
    reset = exit
before = peak
on enter:
    peak = close
while long:
    peak = max(peak, high)
after = peak
infy = symbol(\"INFY\").close @ 1d
stale_infy = symbol(\"INFY\").stale
entry = position.side == \"flat\" and close > infy and not stale_infy
exit = close < after * 0.95 or position.bars_held >= 5
";
    let parsed = parse(src);
    assert!(parsed.diags.is_empty(), "{:?}", parsed.diags);
    let graph = lower_default(&parsed.program).expect("lower succeeds");

    assert_eq!(graph.symbols, vec!["INFY".to_string()]);
    assert_eq!(graph.states.len(), 1);
    assert_eq!(graph.state_blocks.len(), 2);

    // StateRead before and after the blocks must get distinct NodeIds (distinct epochs).
    let before_id = graph
        .bindings
        .iter()
        .find(|(k, _)| k == "before")
        .unwrap()
        .1;
    let after_id = graph.bindings.iter().find(|(k, _)| k == "after").unwrap().1;
    assert_ne!(before_id, after_id);

    let sym_daily = graph
        .nodes
        .iter()
        .find(|n| matches!(n.op, IrOp::SymbolSeries { sym_idx: 0, .. } if matches!(n.clock, IrClock::Tf(_))))
        .expect("daily symbol node");
    assert!(matches!(sym_daily.clock, IrClock::Tf(tf) if tf.minutes() == 1440));

    assert!(graph
        .schedule
        .iter()
        .any(|s| matches!(s, IrExecStep::ResetState(_))));
    assert!(graph
        .schedule
        .iter()
        .any(|s| matches!(s, IrExecStep::ExecBlock(_))));

    let dumped = dump_ir(&graph);
    assert!(dumped.contains("state_init("));
    assert!(dumped.contains("block b0 on enter:"));
    assert!(dumped.contains("block b1 while long:"));
    assert!(dumped.contains("symbol(\"INFY\", close)"));
    assert!(dumped.contains("position(side)"));
}

#[test]
fn self_referencing_state_reset_lowers_to_expr_with_state_read() {
    let src = "\
state count:
    init = 0
    reset = count >= 10
on close > open:
    count = count + 1
entry = count > 0
exit = false
";
    let parsed = parse(src);
    assert!(parsed.diags.is_empty(), "{:?}", parsed.diags);
    let graph = lower_default(&parsed.program).expect("lower succeeds");
    assert!(matches!(graph.states[0].reset, IrReset::Expr(_)));
    assert!(graph
        .nodes
        .iter()
        .any(|n| matches!(n.op, IrOp::StateRead(id) if id == graph.states[0].id)));
}

#[test]
fn higher_tf_state_read_respects_epoch_and_block_nodes_stay_scoped() {
    let src = "\
state dp:
    init = close @ 1d
before = dp
on enter:
    dp = high @ 1d
after = dp
entry = after > before
exit = false
";
    let parsed = parse(src);
    assert!(parsed.diags.is_empty(), "{:?}", parsed.diags);
    let graph = lower_default(&parsed.program).expect("lower succeeds");

    let before_id = graph
        .bindings
        .iter()
        .find(|(k, _)| k == "before")
        .unwrap()
        .1;
    let after_id = graph.bindings.iter().find(|(k, _)| k == "after").unwrap().1;
    assert_ne!(before_id, after_id);

    // `high @ 1d` inside `on enter:` must live in `state_blocks[0].nodes` and not top-level `schedule`.
    assert!(
        !graph.state_blocks[0].nodes.is_empty(),
        "expected block-scoped @1d node in blk.nodes"
    );
    for &blk_node in &graph.state_blocks[0].nodes {
        assert!(!graph
            .schedule
            .iter()
            .any(|step| matches!(step, IrExecStep::EvalNode(id) if *id == blk_node)));
    }
}
