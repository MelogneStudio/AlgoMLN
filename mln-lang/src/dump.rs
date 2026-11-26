use std::fmt::Write;

use crate::ast::{BinOp, Def, Expr, Program, Stmt, UnOp};

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
        Stmt::Error(_) => out.push_str("(error)"),
    }
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

/// Formats a single [`Expr`] in canonical s-expression syntax.
pub fn dump_expr(expr: &Expr, out: &mut String) {
    match expr {
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
            dump_expr(e, out);
            out.push(')');
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
            dump_expr(l, out);
            out.push(' ');
            dump_expr(r, out);
            out.push(')');
        }
        Expr::Compare { first, rest, .. } => {
            out.push_str("(cmp ");
            dump_expr(first, out);
            for (op, rhs) in rest {
                let _ = write!(out, " {} ", op.as_str());
                dump_expr(rhs, out);
            }
            out.push(')');
        }
        Expr::Not(e, _) => {
            out.push_str("(not ");
            dump_expr(e, out);
            out.push(')');
        }
        Expr::And(l, r, _) => {
            out.push_str("(and ");
            dump_expr(l, out);
            out.push(' ');
            dump_expr(r, out);
            out.push(')');
        }
        Expr::Or(l, r, _) => {
            out.push_str("(or ");
            dump_expr(l, out);
            out.push(' ');
            dump_expr(r, out);
            out.push(')');
        }
        Expr::IfElse {
            cond,
            then,
            otherwise,
            ..
        } => {
            out.push_str("(if ");
            dump_expr(cond, out);
            out.push(' ');
            dump_expr(then, out);
            out.push(' ');
            dump_expr(otherwise, out);
            out.push(')');
        }
        Expr::Call { callee, args, .. } => {
            out.push_str("(call ");
            dump_expr(callee, out);
            for arg in args {
                out.push(' ');
                if let Some(name) = &arg.name {
                    let _ = write!(out, "(kw {} ", name.name);
                    dump_expr(&arg.value, out);
                    out.push(')');
                } else {
                    dump_expr(&arg.value, out);
                }
            }
            out.push(')');
        }
        Expr::Index { base, index, .. } => {
            out.push_str("(idx ");
            dump_expr(base, out);
            out.push(' ');
            dump_expr(index, out);
            out.push(')');
        }
        Expr::Attr { base, name, .. } => {
            out.push_str("(attr ");
            dump_expr(base, out);
            let _ = write!(out, " {})", name.name);
        }
        Expr::At { base, tf, .. } => {
            out.push_str("(at ");
            dump_expr(base, out);
            let _ = write!(out, " {}{})", tf.n, tf.unit);
        }
        Expr::Error(_) => out.push_str("(error)"),
    }
}
