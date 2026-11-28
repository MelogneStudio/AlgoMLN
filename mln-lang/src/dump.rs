use std::fmt::Write;

use crate::ast::{
    BinOp, Def, EventSpec, Expr, GuardSpec, OnBlock, Program, StateDecl, StateReset, Stmt, UnOp,
    WhileBlock,
};
use crate::lexer::TfUnit;

/// Formats a parsed [`Program`] into its canonical s-expression representation.
pub fn dump(p: &Program) -> String {
    let mut out = String::new();
    for (i, stmt) in p.stmts.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        dump_stmt(stmt, &mut out);
    }
    out
}

fn dump_stmt(stmt: &Stmt, out: &mut String) {
    match stmt {
        Stmt::Assign { name, value, .. } => {
            let _ = write!(out, "(assign {} ", name.name);
            dump_expr(value, out);
            out.push(')');
        }
        Stmt::Def(def) => dump_def(def, out),
        Stmt::State(state) => dump_state(state, out),
        Stmt::On(on_blk) => dump_on(on_blk, out),
        Stmt::While(while_blk) => dump_while(while_blk, out),
        Stmt::Error(_) => out.push_str("(error)"),
    }
}

fn dump_state(state: &StateDecl, out: &mut String) {
    let _ = write!(out, "(state {} (init ", state.name.name);
    dump_expr(&state.init, out);
    out.push(')');
    if let Some(reset) = &state.reset {
        out.push_str(" (reset ");
        match reset {
            StateReset::Enter => out.push_str("enter"),
            StateReset::Exit => out.push_str("exit"),
            StateReset::Expr(e) => dump_expr(e, out),
        }
        out.push(')');
    }
    out.push(')');
}

fn dump_on(block: &OnBlock, out: &mut String) {
    out.push_str("(on ");
    match &block.event {
        EventSpec::Enter => out.push_str("enter"),
        EventSpec::Exit => out.push_str("exit"),
        EventSpec::Expr(e) => dump_expr(e, out),
    }
    for stmt in &block.body {
        out.push(' ');
        dump_stmt(stmt, out);
    }
    out.push(')');
}

fn dump_while(block: &WhileBlock, out: &mut String) {
    out.push_str("(while ");
    match &block.guard {
        GuardSpec::Long => out.push_str("long"),
        GuardSpec::Short => out.push_str("short"),
        GuardSpec::Flat => out.push_str("flat"),
        GuardSpec::Expr(e) => dump_expr(e, out),
    }
    for stmt in &block.body {
        out.push(' ');
        dump_stmt(stmt, out);
    }
    out.push(')');
}

fn dump_def(def: &Def, out: &mut String) {
    let _ = write!(out, "(def {} (", def.name.name);
    for (i, param) in def.params.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if let Some(default) = &param.default {
            let _ = write!(out, "({} ", param.name.name);
            dump_expr(default, out);
            out.push(')');
        } else {
            out.push_str(&param.name.name);
        }
    }
    out.push_str(") (body");
    for stmt in &def.body {
        out.push(' ');
        dump_stmt(stmt, out);
    }
    out.push_str(") (return ");
    dump_expr(&def.ret, out);
    out.push_str("))");
}

enum DumpTask<'a> {
    Expr(&'a Expr),
    Str(&'static str),
    Char(char),
    Ident(&'a str),
    Tf(u32, TfUnit),
}

/// Formats a single [`Expr`] in canonical s-expression syntax.
pub fn dump_expr(expr: &Expr, out: &mut String) {
    let mut stack = vec![DumpTask::Expr(expr)];
    while let Some(task) = stack.pop() {
        match task {
            DumpTask::Str(s) => out.push_str(s),
            DumpTask::Char(c) => out.push(c),
            DumpTask::Ident(s) => out.push_str(s),
            DumpTask::Tf(n, unit) => {
                let _ = write!(out, "{n}{unit}");
            }
            DumpTask::Expr(node) => match node {
                Expr::Num { value, pct, .. } => {
                    let _ = write!(out, "{}", value.normalize());
                    if *pct {
                        out.push('%');
                    }
                }
                Expr::Timeframe(tf) => {
                    let _ = write!(out, "{}{}", tf.n, tf.unit);
                }
                Expr::Str(s, _) => {
                    out.push('"');
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
                }
                Expr::Bool(b, _) => out.push_str(if *b { "true" } else { "false" }),
                Expr::Name(id) => out.push_str(&id.name),
                Expr::Unary { op, e, .. } => {
                    let tag = match op {
                        UnOp::Neg => "neg",
                        UnOp::Pos => "pos",
                    };
                    let _ = write!(out, "({tag} ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(e));
                }
                Expr::Binary { op, l, r, .. } => {
                    let sym = match op {
                        BinOp::Add => "+",
                        BinOp::Sub => "-",
                        BinOp::Mul => "*",
                        BinOp::Div => "/",
                        BinOp::Pow => "**",
                    };
                    let _ = write!(out, "({sym} ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(r));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(l));
                }
                Expr::Compare { first, rest, .. } => {
                    out.push_str("(cmp ");
                    stack.push(DumpTask::Char(')'));
                    for (op, rhs) in rest.iter().rev() {
                        stack.push(DumpTask::Expr(rhs));
                        stack.push(DumpTask::Char(' '));
                        stack.push(DumpTask::Str(op.as_str()));
                        stack.push(DumpTask::Char(' '));
                    }
                    stack.push(DumpTask::Expr(first));
                }
                Expr::Not(e, _) => {
                    out.push_str("(not ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(e));
                }
                Expr::And(l, r, _) => {
                    out.push_str("(and ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(r));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(l));
                }
                Expr::Or(l, r, _) => {
                    out.push_str("(or ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(r));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(l));
                }
                Expr::IfElse {
                    cond,
                    then,
                    otherwise,
                    ..
                } => {
                    out.push_str("(if ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(otherwise));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(then));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(cond));
                }
                Expr::Call { callee, args, .. } => {
                    out.push_str("(call ");
                    stack.push(DumpTask::Char(')'));
                    for arg in args.iter().rev() {
                        if let Some(name) = &arg.name {
                            stack.push(DumpTask::Char(')'));
                            stack.push(DumpTask::Expr(&arg.value));
                            stack.push(DumpTask::Char(' '));
                            stack.push(DumpTask::Ident(&name.name));
                            stack.push(DumpTask::Str(" (kw "));
                        } else {
                            stack.push(DumpTask::Expr(&arg.value));
                            stack.push(DumpTask::Char(' '));
                        }
                    }
                    stack.push(DumpTask::Expr(callee));
                }
                Expr::Index { base, index, .. } => {
                    out.push_str("(idx ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Expr(index));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(base));
                }
                Expr::Attr { base, name, .. } => {
                    out.push_str("(attr ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Ident(&name.name));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(base));
                }
                Expr::At { base, tf, .. } => {
                    out.push_str("(at ");
                    stack.push(DumpTask::Char(')'));
                    stack.push(DumpTask::Tf(tf.n, tf.unit));
                    stack.push(DumpTask::Char(' '));
                    stack.push(DumpTask::Expr(base));
                }
                Expr::Error(_) => out.push_str("(error)"),
            },
        }
    }
}
