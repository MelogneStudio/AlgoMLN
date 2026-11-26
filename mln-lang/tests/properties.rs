use std::time::Instant;

use proptest::prelude::*;
use rust_decimal::Decimal;

use mln_lang::{
    dump, lex, parse, Arg, BinOp, CmpOp, Expr, Ident, Program, Span, Stmt, TfUnit, Timeframe, UnOp,
};

fn check_expr_spans(expr: &Expr, parent_span: Span, src: &str) {
    let span = expr.span();
    assert!(
        span.start >= parent_span.start && span.end <= parent_span.end,
        "child span {span:?} outside parent {parent_span:?}"
    );
    assert!(span.start <= span.end && (span.end as usize) <= src.len());
    assert!(src.get(span.start as usize..span.end as usize).is_some());

    match expr {
        Expr::Unary { e, .. } | Expr::Not(e, _) => check_expr_spans(e, span, src),
        Expr::Binary { l, r, .. } | Expr::And(l, r, _) | Expr::Or(l, r, _) => {
            check_expr_spans(l, span, src);
            check_expr_spans(r, span, src);
        }
        Expr::Compare { first, rest, .. } => {
            check_expr_spans(first, span, src);
            for (_, rhs) in rest {
                check_expr_spans(rhs, span, src);
            }
        }
        Expr::IfElse {
            cond,
            then,
            otherwise,
            ..
        } => {
            check_expr_spans(cond, span, src);
            check_expr_spans(then, span, src);
            check_expr_spans(otherwise, span, src);
        }
        Expr::Call { callee, args, .. } => {
            check_expr_spans(callee, span, src);
            for a in args {
                assert!(a.span.start >= span.start && a.span.end <= span.end);
                check_expr_spans(&a.value, a.span, src);
            }
        }
        Expr::Index { base, index, .. } => {
            check_expr_spans(base, span, src);
            check_expr_spans(index, span, src);
        }
        Expr::Attr { base, name, .. } => {
            check_expr_spans(base, span, src);
            assert!(name.span.start >= span.start && name.span.end <= span.end);
        }
        Expr::At { base, tf, .. } => {
            check_expr_spans(base, span, src);
            assert!(tf.span.start >= span.start && tf.span.end <= span.end);
        }
        Expr::Num { .. }
        | Expr::Timeframe(_)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Name(_)
        | Expr::Error(_) => {}
    }
}

fn check_stmt_spans(stmt: &Stmt, src: &str) {
    let span = stmt.span();
    assert!(span.start <= span.end && (span.end as usize) <= src.len());
    assert!(src.get(span.start as usize..span.end as usize).is_some());
    match stmt {
        Stmt::Assign { name, value, .. } => {
            assert!(name.span.start >= span.start && name.span.end <= span.end);
            check_expr_spans(value, span, src);
        }
        Stmt::Def(d) => {
            assert!(d.name.span.start >= span.start && d.name.span.end <= span.end);
            for p in &d.params {
                assert!(p.span.start >= span.start && p.span.end <= span.end);
                if let Some(def_val) = &p.default {
                    check_expr_spans(def_val, p.span, src);
                }
            }
            for s in &d.body {
                assert!(s.span().start >= span.start && s.span().end <= span.end);
                check_stmt_spans(s, src);
            }
            check_expr_spans(&d.ret, span, src);
        }
        Stmt::Error(_) => {}
    }
}

fn arb_ident() -> impl Strategy<Value = Ident> {
    prop::sample::select(vec![
        "a", "b", "c", "close", "open", "high", "low", "ema", "sma", "n", "x", "state", "on",
    ])
    .prop_map(|s| Ident {
        name: s.to_string(),
        span: Span::empty(0),
    })
}

fn arb_tf() -> impl Strategy<Value = Timeframe> {
    (
        1u32..=60,
        prop::sample::select(vec![
            TfUnit::Minute,
            TfUnit::Hour,
            TfUnit::Day,
            TfUnit::Week,
        ]),
    )
        .prop_map(|(n, unit)| Timeframe {
            n,
            unit,
            span: Span::empty(0),
        })
}

fn arb_expr() -> impl Strategy<Value = Expr> {
    let leaf = prop_oneof![
        (0u64..10000, any::<bool>()).prop_map(|(n, pct)| Expr::Num {
            value: Decimal::from(n),
            pct,
            span: Span::empty(0),
        }),
        arb_tf().prop_map(Expr::Timeframe),
        prop::sample::select(vec!["", "ab", "a\nb", "café"])
            .prop_map(|s| Expr::Str(s.to_string(), Span::empty(0),)),
        any::<bool>().prop_map(|b| Expr::Bool(b, Span::empty(0))),
        arb_ident().prop_map(Expr::Name),
    ];

    leaf.prop_recursive(5, 64, 6, |inner| {
        prop_oneof![
            (
                prop::sample::select(vec![UnOp::Neg, UnOp::Pos]),
                inner.clone()
            )
                .prop_map(|(op, e)| Expr::Unary {
                    op,
                    e: Box::new(e),
                    span: Span::empty(0),
                }),
            (
                prop::sample::select(vec![
                    BinOp::Add,
                    BinOp::Sub,
                    BinOp::Mul,
                    BinOp::Div,
                    BinOp::Pow
                ]),
                inner.clone(),
                inner.clone(),
            )
                .prop_map(|(op, l, r)| Expr::Binary {
                    op,
                    l: Box::new(l),
                    r: Box::new(r),
                    span: Span::empty(0),
                }),
            (
                inner.clone(),
                prop::collection::vec(
                    (
                        prop::sample::select(vec![
                            CmpOp::Eq,
                            CmpOp::NotEq,
                            CmpOp::Lt,
                            CmpOp::LtEq,
                            CmpOp::Gt,
                            CmpOp::GtEq,
                        ]),
                        inner.clone(),
                    ),
                    1..=3,
                ),
            )
                .prop_map(|(first, rest)| Expr::Compare {
                    first: Box::new(first),
                    rest,
                    span: Span::empty(0),
                }),
            inner
                .clone()
                .prop_map(|e| Expr::Not(Box::new(e), Span::empty(0))),
            (inner.clone(), inner.clone()).prop_map(|(l, r)| Expr::And(
                Box::new(l),
                Box::new(r),
                Span::empty(0)
            )),
            (inner.clone(), inner.clone()).prop_map(|(l, r)| Expr::Or(
                Box::new(l),
                Box::new(r),
                Span::empty(0)
            )),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(cond, then, otherwise)| {
                Expr::IfElse {
                    cond: Box::new(cond),
                    then: Box::new(then),
                    otherwise: Box::new(otherwise),
                    span: Span::empty(0),
                }
            }),
            (
                inner.clone(),
                prop::collection::vec(inner.clone(), 0..=2),
                prop::collection::vec((arb_ident(), inner.clone()), 0..=2),
            )
                .prop_map(|(callee, pos, kws)| {
                    let mut args: Vec<Arg> = pos
                        .into_iter()
                        .map(|value| Arg {
                            name: None,
                            value,
                            span: Span::empty(0),
                        })
                        .collect();
                    let mut seen = std::collections::HashSet::new();
                    for (name, value) in kws {
                        if seen.insert(name.name.clone()) {
                            args.push(Arg {
                                name: Some(name),
                                value,
                                span: Span::empty(0),
                            });
                        }
                    }
                    Expr::Call {
                        callee: Box::new(callee),
                        args,
                        span: Span::empty(0),
                    }
                }),
            (inner.clone(), inner.clone()).prop_map(|(base, index)| Expr::Index {
                base: Box::new(base),
                index: Box::new(index),
                span: Span::empty(0),
            }),
            (inner.clone(), arb_ident()).prop_map(|(base, name)| Expr::Attr {
                base: Box::new(base),
                name,
                span: Span::empty(0),
            }),
            (inner, arb_tf()).prop_map(|(base, tf)| Expr::At {
                base: Box::new(base),
                tf,
                span: Span::empty(0),
            }),
        ]
    })
}

fn print_leaf(expr: &Expr) -> String {
    match expr {
        Expr::Num { value, pct, .. } => {
            if *pct {
                format!("{}%", value.normalize())
            } else {
                format!("{}", value.normalize())
            }
        }
        Expr::Timeframe(tf) => format!("{}{}", tf.n, tf.unit),
        Expr::Str(s, _) => {
            let mut out = String::from("\"");
            for ch in s.chars() {
                match ch {
                    '\\' => out.push_str("\\\\"),
                    '"' => out.push_str("\\\""),
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    _ => out.push(ch),
                }
            }
            out.push('"');
            out
        }
        Expr::Bool(b, _) => if *b { "true" } else { "false" }.to_string(),
        Expr::Name(id) => id.name.clone(),
        Expr::Error(_) => unreachable!(),
        _ => unreachable!(),
    }
}

fn print_fully_parens(expr: &Expr) -> String {
    match expr {
        Expr::Num { .. }
        | Expr::Timeframe(_)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Name(_) => print_leaf(expr),
        Expr::Unary { op, e, .. } => {
            let s = match op {
                UnOp::Neg => "-",
                UnOp::Pos => "+",
            };
            format!("({s} {})", print_fully_parens(e))
        }
        Expr::Binary { op, l, r, .. } => {
            let s = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Pow => "**",
            };
            format!("({} {s} {})", print_fully_parens(l), print_fully_parens(r))
        }
        Expr::Compare { first, rest, .. } => {
            let mut out = format!("({}", print_fully_parens(first));
            for (op, rhs) in rest {
                out.push_str(&format!(" {} {}", op.as_str(), print_fully_parens(rhs)));
            }
            out.push(')');
            out
        }
        Expr::Not(e, _) => format!("(not {})", print_fully_parens(e)),
        Expr::And(l, r, _) => format!("({} and {})", print_fully_parens(l), print_fully_parens(r)),
        Expr::Or(l, r, _) => format!("({} or {})", print_fully_parens(l), print_fully_parens(r)),
        Expr::IfElse {
            cond,
            then,
            otherwise,
            ..
        } => format!(
            "({} if {} else {})",
            print_fully_parens(then),
            print_fully_parens(cond),
            print_fully_parens(otherwise)
        ),
        Expr::Call { callee, args, .. } => {
            let rendered_args: Vec<String> = args
                .iter()
                .map(|a| {
                    if let Some(name) = &a.name {
                        format!("{}={}", name.name, print_fully_parens(&a.value))
                    } else {
                        print_fully_parens(&a.value)
                    }
                })
                .collect();
            format!(
                "({}({}))",
                print_fully_parens(callee),
                rendered_args.join(", ")
            )
        }
        Expr::Index { base, index, .. } => {
            format!(
                "({}[{}])",
                print_fully_parens(base),
                print_fully_parens(index)
            )
        }
        Expr::Attr { base, name, .. } => {
            let bs = if matches!(**base, Expr::Num { pct: false, .. }) {
                format!("({})", print_fully_parens(base))
            } else {
                print_fully_parens(base)
            };
            format!("({bs}.{})", name.name)
        }
        Expr::At { base, tf, .. } => {
            format!("({} @ {}{})", print_fully_parens(base), tf.n, tf.unit)
        }
        Expr::Error(_) => unreachable!(),
    }
}

fn prec(expr: &Expr) -> u8 {
    match expr {
        Expr::IfElse { .. } => 0,
        Expr::Or(_, _, _) => 1,
        Expr::And(_, _, _) => 2,
        Expr::Not(_, _) => 3,
        Expr::Compare { .. } => 4,
        Expr::Binary {
            op: BinOp::Add | BinOp::Sub,
            ..
        } => 5,
        Expr::Binary {
            op: BinOp::Mul | BinOp::Div,
            ..
        } => 6,
        Expr::Unary { .. } => 7,
        Expr::Binary { op: BinOp::Pow, .. } => 8,
        Expr::Call { .. } | Expr::Index { .. } | Expr::Attr { .. } | Expr::At { .. } => 9,
        Expr::Num { .. }
        | Expr::Timeframe(_)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Name(_)
        | Expr::Error(_) => 10,
    }
}

fn wrap_if(s: String, need: bool) -> String {
    if need {
        format!("({s})")
    } else {
        s
    }
}

fn print_minimal_parens(expr: &Expr) -> String {
    match expr {
        Expr::Num { .. }
        | Expr::Timeframe(_)
        | Expr::Str(_, _)
        | Expr::Bool(_, _)
        | Expr::Name(_) => print_leaf(expr),
        Expr::Unary { op, e, .. } => {
            let sym = match op {
                UnOp::Neg => "-",
                UnOp::Pos => "+",
            };
            let inner = wrap_if(print_minimal_parens(e), prec(e) < 7);
            format!("{sym} {inner}")
        }
        Expr::Binary { op, l, r, .. } => match op {
            BinOp::Add | BinOp::Sub => {
                let sym = if *op == BinOp::Add { "+" } else { "-" };
                let ls = wrap_if(print_minimal_parens(l), prec(l) < 5);
                let rs = wrap_if(print_minimal_parens(r), prec(r) <= 5);
                format!("{ls} {sym} {rs}")
            }
            BinOp::Mul | BinOp::Div => {
                let sym = if *op == BinOp::Mul { "*" } else { "/" };
                let ls = wrap_if(print_minimal_parens(l), prec(l) < 6);
                let rs = wrap_if(print_minimal_parens(r), prec(r) <= 6);
                format!("{ls} {sym} {rs}")
            }
            BinOp::Pow => {
                // Left operand cannot be Unary or Pow without parens (`(-2)**2`, `(2**3)**2`).
                // Right operand can be Unary or Pow (`2**-1`, `2**3**2`), so only wrap if `< 7`.
                let ls = wrap_if(print_minimal_parens(l), prec(l) <= 8);
                let rs = wrap_if(print_minimal_parens(r), prec(r) < 7);
                format!("{ls} ** {rs}")
            }
        },
        Expr::Compare { first, rest, .. } => {
            let mut out = wrap_if(print_minimal_parens(first), prec(first) <= 4);
            for (op, rhs) in rest {
                let rs = wrap_if(print_minimal_parens(rhs), prec(rhs) <= 4);
                out.push_str(&format!(" {} {rs}", op.as_str()));
            }
            out
        }
        Expr::Not(e, _) => {
            let inner = wrap_if(print_minimal_parens(e), prec(e) < 3);
            format!("not {inner}")
        }
        Expr::And(l, r, _) => {
            let ls = wrap_if(print_minimal_parens(l), prec(l) < 2);
            let rs = wrap_if(print_minimal_parens(r), prec(r) <= 2);
            format!("{ls} and {rs}")
        }
        Expr::Or(l, r, _) => {
            let ls = wrap_if(print_minimal_parens(l), prec(l) < 1);
            let rs = wrap_if(print_minimal_parens(r), prec(r) <= 1);
            format!("{ls} or {rs}")
        }
        Expr::IfElse {
            cond,
            then,
            otherwise,
            ..
        } => {
            let ts = wrap_if(print_minimal_parens(then), prec(then) < 1);
            let cs = wrap_if(print_minimal_parens(cond), prec(cond) < 1);
            let os = print_minimal_parens(otherwise);
            format!("{ts} if {cs} else {os}")
        }
        Expr::Call { callee, args, .. } => {
            let bs = wrap_if(print_minimal_parens(callee), prec(callee) < 9);
            let rendered_args: Vec<String> = args
                .iter()
                .map(|a| {
                    if let Some(name) = &a.name {
                        format!("{}={}", name.name, print_minimal_parens(&a.value))
                    } else {
                        print_minimal_parens(&a.value)
                    }
                })
                .collect();
            format!("{bs}({})", rendered_args.join(", "))
        }
        Expr::Index { base, index, .. } => {
            let bs = wrap_if(print_minimal_parens(base), prec(base) < 9);
            format!("{bs}[{}]", print_minimal_parens(index))
        }
        Expr::Attr { base, name, .. } => {
            let need = prec(base) < 9 || matches!(**base, Expr::Num { pct: false, .. });
            let bs = wrap_if(print_minimal_parens(base), need);
            format!("{bs}.{}", name.name)
        }
        Expr::At { base, tf, .. } => {
            let bs = wrap_if(print_minimal_parens(base), prec(base) < 9);
            format!("{bs} @ {}{}", tf.n, tf.unit)
        }
        Expr::Error(_) => unreachable!(),
    }
}

proptest! {
    #[test]
    fn f01_no_panic_and_fast_on_arbitrary_strings(s in "\\PC*") {
        let t0 = Instant::now();
        let out = parse(&s);
        let elapsed = t0.elapsed();
        prop_assert!(elapsed.as_millis() < 100, "parse took {:?} on {:?}", elapsed, s);
        prop_assert!(out.diags.len() <= 51);
    }

    #[test]
    fn f01_bytes_lossy_no_panic(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        let s = String::from_utf8_lossy(&bytes);
        let t0 = Instant::now();
        let out = parse(&s);
        prop_assert!(t0.elapsed().as_millis() < 100);
        prop_assert!(out.diags.len() <= 51);
    }

    #[test]
    fn f02_spans_are_valid_and_nested(s in "\\PC*") {
        let out = parse(&s);
        for d in &out.diags {
            prop_assert!(d.span.start <= d.span.end);
            prop_assert!((d.span.end as usize) <= s.len());
            prop_assert!(s.is_char_boundary(d.span.start as usize));
            prop_assert!(s.is_char_boundary(d.span.end as usize));
        }
        for stmt in &out.program.stmts {
            check_stmt_spans(stmt, &s);
        }
    }

    #[test]
    fn f03_fully_parenthesized_roundtrip(expr in arb_expr()) {
        let tree_prog = Program {
            stmts: vec![Stmt::Assign {
                name: Ident { name: "x".to_string(), span: Span::empty(0) },
                value: expr.clone(),
                span: Span::empty(0),
            }],
        };
        let src = format!("x = {}", print_fully_parens(&expr));
        let parsed = parse(&src);
        prop_assert!(parsed.diags.is_empty(), "diags for {src}: {:?}", parsed.diags);
        prop_assert_eq!(dump(&parsed.program), dump(&tree_prog));
    }

    #[test]
    fn f04_minimal_parentheses_roundtrip(expr in arb_expr()) {
        let tree_prog = Program {
            stmts: vec![Stmt::Assign {
                name: Ident { name: "x".to_string(), span: Span::empty(0) },
                value: expr.clone(),
                span: Span::empty(0),
            }],
        };
        let src = format!("x = {}", print_minimal_parens(&expr));
        let parsed = parse(&src);
        prop_assert!(parsed.diags.is_empty(), "diags for {src}: {:?}", parsed.diags);
        prop_assert_eq!(dump(&parsed.program), dump(&tree_prog));
    }

    #[test]
    fn f05_extra_spaces_and_crlf_preserve_token_kinds(expr in arb_expr()) {
        let base = format!("def f(x):\n    y = {}\n    return y\nz = f(1)\n", print_minimal_parens(&expr));
        let spaced = base
            .replace(" + ", "   +   ")
            .replace(" * ", "   *   ")
            .replace(", ", "  ,   ");
        let crlf = spaced.replace('\n', "\r\n");

        let k1: Vec<_> = lex(&base).tokens.into_iter().map(|t| t.kind).collect();
        let k2: Vec<_> = lex(&spaced).tokens.into_iter().map(|t| t.kind).collect();
        let k3: Vec<_> = lex(&crlf).tokens.into_iter().map(|t| t.kind).collect();
        prop_assert_eq!(&k1, &k2);
        prop_assert_eq!(&k1, &k3);
    }

    #[test]
    fn f06_parse_is_deterministic(s in "\\PC*") {
        let a = parse(&s);
        let b = parse(&s);
        prop_assert_eq!(dump(&a.program), dump(&b.program));
        prop_assert_eq!(a.diags, b.diags);
    }

    #[test]
    fn f07_dump_invariant_under_trailing_newline(expr in arb_expr()) {
        let without = format!("x = {}", print_minimal_parens(&expr));
        let with = format!("{without}\n");
        let a = parse(&without);
        let b = parse(&with);
        prop_assert_eq!(dump(&a.program), dump(&b.program));
    }

    #[test]
    fn f09_diagnostic_count_at_most_51(s in "\\PC*") {
        let out = parse(&s);
        prop_assert!(out.diags.len() <= 51);
    }
}

#[test]
fn f08_depth_bomb_and_large_inputs_under_1s() {
    // 10,000 nested `(`
    let nested = format!("x = {}1{}", "(".repeat(10_000), ")".repeat(10_000));
    let t0 = Instant::now();
    let out1 = parse(&nested);
    assert!(t0.elapsed().as_secs_f64() < 1.0);
    assert_eq!(out1.diags.len(), 1);
    assert_eq!(out1.diags[0].code, "E0120");

    // 100,000-token expression (`x = 1 + 1 + ...`)
    let mut big_expr = String::from("x = 1");
    for _ in 0..50_000 {
        big_expr.push_str(" + 1");
    }
    let t1 = Instant::now();
    let out2 = parse(&big_expr);
    assert!(t1.elapsed().as_secs_f64() < 1.0);
    assert!(out2.diags.is_empty());

    // 50,000-statement program
    let mut big_prog = String::new();
    for i in 0..50_000 {
        big_prog.push_str(&format!("x{i} = {i}\n"));
    }
    let t2 = Instant::now();
    let out3 = parse(&big_prog);
    assert!(t2.elapsed().as_secs_f64() < 1.0);
    assert!(out3.diags.is_empty());
    assert_eq!(out3.program.stmts.len(), 50_000);
}
