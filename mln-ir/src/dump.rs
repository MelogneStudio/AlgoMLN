use std::fmt::Write;

use mln_lang::{BinOp, UnOp};

use crate::ir::{IrClock, IrConst, IrGraph, IrNode, IrOp, IrReset, IrTrigger};

/// Formats an [`IrGraph`] into its canonical deterministic text dump.
pub fn dump_ir(graph: &IrGraph) -> String {
    let mut out = String::new();
    for st in &graph.states {
        let _ = write!(out, "{} ({}) = state_init({}", st.id, st.name, st.init);
        match st.reset {
            IrReset::None => out.push(')'),
            IrReset::Enter => out.push_str(", reset=enter)"),
            IrReset::Exit => out.push_str(", reset=exit)"),
            IrReset::Expr(id) => {
                let _ = write!(out, ", reset={id})");
            }
        }
        out.push('\n');
    }
    for node in &graph.nodes {
        dump_node(node, &graph.symbols, &mut out);
        out.push('\n');
    }
    for (idx, blk) in graph.state_blocks.iter().enumerate() {
        out.push_str("block ");
        let _ = write!(out, "b{idx} ");
        match blk.trigger {
            IrTrigger::OnEnter => out.push_str("on enter"),
            IrTrigger::OnExit => out.push_str("on exit"),
            IrTrigger::OnExpr(id) => {
                let _ = write!(out, "on {id}");
            }
            IrTrigger::WhileLong => out.push_str("while long"),
            IrTrigger::WhileShort => out.push_str("while short"),
            IrTrigger::WhileFlat => out.push_str("while flat"),
            IrTrigger::WhileExpr(id) => {
                let _ = write!(out, "while {id}");
            }
        }
        out.push_str(": ");
        for (i, (sid, val_id)) in blk.updates.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{sid} = {val_id}");
        }
        out.push('\n');
    }
    let _ = writeln!(out, "entry = {}", graph.entry);
    let _ = write!(out, "exit = {}", graph.exit);
    out
}

fn dump_node(node: &IrNode, symbols: &[String], out: &mut String) {
    let _ = write!(out, "{} = ", node.id);
    match &node.op {
        IrOp::Series(field) => {
            let _ = write!(out, "series({})", field.as_str());
        }
        IrOp::Position(field) => {
            let _ = write!(out, "position({})", field.as_str());
        }
        IrOp::SymbolSeries { sym_idx, field } => {
            let name = symbols.get(*sym_idx as usize).map_or("?", String::as_str);
            let _ = write!(out, "symbol({name:?}, {})", field.as_str());
        }
        IrOp::StateRead(sid) => {
            let _ = write!(out, "state({sid})");
        }
        IrOp::Const(c) => {
            out.push_str("const(");
            dump_const(c, out);
            out.push(')');
        }
        IrOp::Unary(op) => {
            let name = match op {
                UnOp::Neg => "neg",
                UnOp::Pos => "pos",
            };
            let _ = write!(out, "{name}({})", node.inputs[0]);
        }
        IrOp::Binary(op) => {
            let name = match op {
                BinOp::Add => "add",
                BinOp::Sub => "sub",
                BinOp::Mul => "mul",
                BinOp::Div => "div",
                BinOp::Pow => "pow",
            };
            let _ = write!(out, "{name}({}, {})", node.inputs[0], node.inputs[1]);
        }
        IrOp::Compare(op) => {
            let _ = write!(
                out,
                "cmp({}, {}, {})",
                node.inputs[0],
                op.as_str(),
                node.inputs[1]
            );
        }
        IrOp::Not => {
            let _ = write!(out, "not({})", node.inputs[0]);
        }
        IrOp::And => {
            let _ = write!(out, "and({}, {})", node.inputs[0], node.inputs[1]);
        }
        IrOp::Or => {
            let _ = write!(out, "or({}, {})", node.inputs[0], node.inputs[1]);
        }
        IrOp::IfElse => {
            let _ = write!(
                out,
                "if_else({}, {}, {})",
                node.inputs[0], node.inputs[1], node.inputs[2]
            );
        }
        IrOp::Lag(k) => {
            let _ = write!(out, "lag({}, {k})", node.inputs[0]);
        }
        IrOp::Builtin(name) => {
            let _ = write!(out, "{name}(");
            let mut first = true;
            for id in &node.inputs {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                let _ = write!(out, "{id}");
            }
            for c in &node.const_args {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                dump_const(c, out);
            }
            out.push(')');
        }
    }

    out.push(' ');
    match node.clock {
        IrClock::Constant => out.push_str("@const"),
        IrClock::Base => out.push_str("@base"),
        IrClock::Tf(tf) => {
            let _ = write!(out, "@{tf}");
        }
    }
}

fn dump_const(c: &IrConst, out: &mut String) {
    match c {
        IrConst::Dec { value, pct } => {
            let _ = write!(out, "{}", value.normalize());
            if *pct {
                out.push('%');
            }
        }
        IrConst::Int(i) => {
            let _ = write!(out, "{i}");
        }
        IrConst::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        IrConst::Tf(tf) => {
            let _ = write!(out, "{tf}");
        }
        IrConst::Str(s) => {
            let _ = write!(out, "{s:?}");
        }
    }
}
