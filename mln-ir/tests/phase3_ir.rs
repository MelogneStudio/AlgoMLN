use mln_check::{CheckConfig, CheckedProgram, Tf};
use mln_ir::{dump_ir, lower, lower_default, IrClock, IrConst, IrOp};
use mln_lang::{parse, Expr, Ident, Program, Span, Stmt, TfUnit};
use mln_std::tier1_registry;

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

#[test]
fn eval_const_only_supports_compare_and_strict_ifelse() {
    let src = "\
def f(a, b) = ema(close, 10 if a > b else 20)
entry = f(2, 1) > close[1 if 2 > 1 else 2]
exit = ema(close, 10 if 2 > 1 else 20) > 0
";
    let parsed = parse(src);
    assert!(parsed.diags.is_empty(), "parse diags: {:?}", parsed.diags);
    let graph = lower_default(&parsed.program).expect("lower succeeds");
    let ema_node = graph
        .nodes
        .iter()
        .find(|n| n.op == IrOp::Builtin("ema"))
        .expect("ema node");
    assert_eq!(ema_node.const_args, vec![IrConst::Int(10)]);
    let lag_node = graph
        .nodes
        .iter()
        .find(|n| matches!(n.op, IrOp::Lag(_)))
        .expect("lag node");
    assert_eq!(lag_node.op, IrOp::Lag(1));

    // Unchecked non-constant IfElse branch must be rejected by eval_const_only.
    let bad_parsed = parse("entry = ema(close, 10 if true else close) > 0\nexit = true");
    let reg = tier1_registry();
    let checked = mln_check::check(&bad_parsed.program, &reg, &CheckConfig::default());
    assert!(!checked.is_ok());
    let dummy_checked: CheckedProgram = checked.program;
    assert!(lower(&bad_parsed.program, &dummy_checked, &reg).is_err());
}

#[test]
fn index_and_int_params_check_exact_scale_and_preserve_const_lag() {
    let const_lag = parse("n = 5[2]\nentry = ema(close, n) > 0\nexit = ema(close, 5[2]) > 0");
    let graph = lower_default(&const_lag.program).expect("lower succeeds");
    assert!(
        !graph.nodes.iter().any(|n| matches!(n.op, IrOp::Lag(_))),
        "constant lag should not emit Lag node:\n{}",
        dump_ir(&graph)
    );
    let ema_node = graph
        .nodes
        .iter()
        .find(|n| n.op == IrOp::Builtin("ema"))
        .expect("ema node");
    assert_eq!(ema_node.const_args, vec![IrConst::Int(5)]);

    let reg = tier1_registry();
    for bad in [
        "entry = close[1.5] > 0\nexit = true",
        "entry = ema(close, 1.5) > 0\nexit = true",
        "entry = ema(close, 0) > 0\nexit = true",
        "entry = diff(close, -1) > 0\nexit = true",
    ] {
        let p = parse(bad);
        let checked = mln_check::check(&p.program, &reg, &CheckConfig::default());
        assert!(!checked.is_ok(), "expected check failure for {bad}");
        assert!(
            lower(&p.program, &checked.program, &reg).is_err(),
            "expected lower error for {bad}"
        );
    }
}

#[test]
fn equivalent_timeframes_canonicalize_and_cse_with_base() {
    let reg = tier1_registry();
    let cfg_60m = CheckConfig {
        base_clock: Tf::new(60, TfUnit::Minute),
    };
    let p1 = parse("entry = (close @ 1h) > close\nexit = true");
    let c1 = mln_check::check(&p1.program, &reg, &cfg_60m);
    assert!(c1.is_ok(), "check diags: {:?}", c1.diags);
    let g1 = lower(&p1.program, &c1.program, &reg).expect("lower succeeds");
    let series_count = g1
        .nodes
        .iter()
        .filter(|n| matches!(n.op, IrOp::Series(_)))
        .count();
    assert_eq!(
        series_count,
        1,
        "close @ 1h should CSE with close @base when base=60m:\n{}",
        dump_ir(&g1)
    );
    assert_eq!(g1.nodes[0].clock, IrClock::Base);

    let p2 = parse("a = close @ 60m\nb = close @ 1h\nentry = a > b\nexit = true");
    let g2 = lower_default(&p2.program).expect("lower succeeds");
    let series_count_2 = g2
        .nodes
        .iter()
        .filter(|n| matches!(n.op, IrOp::Series(_)))
        .count();
    assert_eq!(
        series_count_2,
        1,
        "close @ 60m and close @ 1h should CSE:\n{}",
        dump_ir(&g2)
    );
}

#[test]
fn lower_returns_err_instead_of_panicking_on_unchecked_inputs() {
    let reg = tier1_registry();
    for bad in [
        "def f(x) = f(x)\nentry = f(close) > 0\nexit = true",
        "entry = ema(close, 12, 99) > 0\nexit = true",
        "def f(x) = x\nentry = f(close, 99) > 0\nexit = true",
        "entry = (5 @ 1d) > 0\nexit = true",
    ] {
        let p = parse(bad);
        let checked = mln_check::check(&p.program, &reg, &CheckConfig::default());
        assert!(!checked.is_ok(), "expected check error for {bad}");
        assert!(
            lower(&p.program, &checked.program, &reg).is_err(),
            "expected lower Err for {bad}"
        );
    }

    // Synthetic AST with empty Compare.rest must not panic.
    let s = Span::empty(0);
    let empty_cmp = Program {
        stmts: vec![
            Stmt::Assign {
                name: Ident {
                    name: "entry".to_string(),
                    span: s,
                },
                value: Expr::Compare {
                    first: Box::new(Expr::Bool(true, s)),
                    rest: Vec::new(),
                    span: s,
                },
                span: s,
            },
            Stmt::Assign {
                name: Ident {
                    name: "exit".to_string(),
                    span: s,
                },
                value: Expr::Bool(true, s),
                span: s,
            },
        ],
    };
    let checked = mln_check::check(&empty_cmp, &reg, &CheckConfig::default());
    assert!(lower(&empty_cmp, &checked.program, &reg).is_err());
}

#[test]
fn constant_compare_does_not_intern_dead_compare_nodes() {
    let p = parse("entry = 2 > 1\nexit = 1 < 2 < 3");
    let graph = lower_default(&p.program).expect("lower succeeds");
    assert!(
        !graph
            .nodes
            .iter()
            .any(|n| matches!(n.op, IrOp::Compare(_) | IrOp::And)),
        "constant comparison should not intern Compare or And nodes:\n{}",
        dump_ir(&graph)
    );
    assert_eq!(graph.entry, graph.exit);
    assert_eq!(
        graph.nodes[graph.entry.0 as usize].op,
        IrOp::Const(IrConst::Bool(true))
    );
}
