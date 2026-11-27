use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use mln_lang::{BinOp, CmpOp, UnOp};
use mln_std::eval::{clean, safe_div};
use mln_std::TriBool;
use rust_decimal::prelude::ToPrimitive;

use crate::oracle::ExecError;
use crate::rolling::{
    ExtremaSummary, LagRing, PairStatsSummary, PercentileWindow, StatsSummary, SumSummary,
    TwoStackWindow, WmaSummary,
};
use crate::state::{
    validate_bar, validate_bar_allowing_nans, AtrState, Bar, BarOutput, CrossState, CumsumState,
    EwmState, ExecutorSnapshot, MacdState, NodeState, NodeVal, RsiState, SinceState, TrState,
};
use mln_ir::{IrClock, IrConst, IrGraph, IrNode, IrOp, IrTy, NodeId, SeriesField};

const MAX_BUFFER_LEN: usize = 1_000_000;

#[derive(Debug, Clone, Copy)]
enum CompiledOp {
    Series(SeriesField),
    Const(NodeVal),
    Neg(usize),
    Pos(usize),
    Bin { op: BinOp, l: usize, r: usize },
    Cmp { op: CmpOp, l: usize, r: usize },
    Not(usize),
    And(usize, usize),
    Or(usize, usize),
    IfElse { c: usize, t: usize, e: usize },
    Lag { src: usize, always_missing: bool },
    Abs(usize),
    Sqrt(usize),
    Log(usize),
    Exp(usize),
    Min(usize, usize),
    Max(usize, usize),
    Where { c: usize, a: usize, b: usize },
    Clamp { x: usize, lo: usize, hi: usize },
    Mod(usize, usize),
    Diff(usize),
    PctChange(usize),
    Cumsum(usize),
    Sum(usize),
    Mean(usize),
    Std(usize),
    Var(usize),
    Ema(usize),
    Wma { x: usize, denom: f64 },
    Highest(usize),
    Lowest(usize),
    Rsi(usize),
    Tr { h: usize, l: usize, c: usize },
    Atr { h: usize, l: usize, c: usize },
    Macd(usize),
    Bollinger { x: usize, mult: f64 },
    Corr(usize, usize),
    Cov(usize, usize),
    LinregSlope(usize),
    LinregIntercept(usize),
    PercentileRank(usize),
    Crossover(usize, usize),
    Crossunder(usize, usize),
    Since(usize),
}

/// Incremental bar-by-bar and chunked executor for a lowered [`IrGraph`].
#[derive(Debug, Clone)]
pub struct Executor {
    ir_fingerprint: u64,
    ops: Vec<CompiledOp>,
    states: Vec<NodeState>,
    vals: Vec<NodeVal>,
    strings: Vec<String>,
    entry_idx: usize,
    exit_idx: usize,
    bar_count: u64,
}

impl Executor {
    /// Compiles and allocates all per-node buffers for `ir`.
    ///
    /// No heap allocations happen inside [`Self::step`] after construction.
    pub fn new(ir: &IrGraph) -> Result<Self, ExecError> {
        let mut ops = Vec::with_capacity(ir.nodes.len());
        let mut states = Vec::with_capacity(ir.nodes.len());
        let mut vals = Vec::with_capacity(ir.nodes.len());
        let mut strings = Vec::new();

        for node in &ir.nodes {
            if matches!(node.clock, IrClock::Tf(_)) {
                return Err(ExecError::UnsupportedOp(
                    "multi-timeframe clocks are not yet supported by incremental executor".into(),
                ));
            }
            let (op, state, init_val) = compile_node(node, ir.nodes.len(), &mut strings)?;
            ops.push(op);
            states.push(state);
            vals.push(init_val);
        }

        let entry_idx = ir.entry.0 as usize;
        let exit_idx = ir.exit.0 as usize;
        match (ir.nodes.get(entry_idx), ir.nodes.get(exit_idx)) {
            (Some(en), Some(ex)) if en.ty == IrTy::Bool && ex.ty == IrTy::Bool => {}
            _ => {
                return Err(ExecError::InvalidNode(
                    "entry and exit must be valid Bool nodes".into(),
                ))
            }
        }

        Ok(Self {
            ir_fingerprint: fingerprint_ir(ir),
            ops,
            states,
            vals,
            strings,
            entry_idx,
            exit_idx,
            bar_count: 0,
        })
    }

    /// Processes one validated bar, updating node state and returning `entry`/`exit` outputs.
    ///
    /// Rejects bad bars (`high < low`, negative `volume`, non-finite OHLCV) with `Err`
    /// while leaving all internal state completely unchanged.
    pub fn step(&mut self, bar: Bar) -> Result<BarOutput, ExecError> {
        validate_bar(&bar)?;
        Ok(self.step_inner(bar))
    }

    /// Processes one bar while permitting `NaN` holes in OHLCV fields (for differential testing).
    ///
    /// Still rejects infinities, `high < low`, and negative `volume` without mutating state.
    pub fn step_allowing_nans(&mut self, bar: Bar) -> Result<BarOutput, ExecError> {
        validate_bar_allowing_nans(&bar)?;
        Ok(self.step_inner(bar))
    }

    /// Processes a chunk of bars bar-by-bar, returning a freshly allocated `Vec<BarOutput>`.
    pub fn run_chunk(&mut self, bars: &[Bar]) -> Result<Vec<BarOutput>, ExecError> {
        let mut out = Vec::with_capacity(bars.len());
        self.run_chunk_into(bars, &mut out)?;
        Ok(out)
    }

    /// Appends outputs for `bars` bar-by-bar into `out` (zero allocation when
    /// `out.capacity() - out.len() >= bars.len()`).
    pub fn run_chunk_into(
        &mut self,
        bars: &[Bar],
        out: &mut Vec<BarOutput>,
    ) -> Result<(), ExecError> {
        for &bar in bars {
            out.push(self.step(bar)?);
        }
        Ok(())
    }

    /// Captures a point-in-time snapshot of all per-node state.
    pub fn snapshot(&self) -> ExecutorSnapshot {
        ExecutorSnapshot {
            ir_fingerprint: self.ir_fingerprint,
            bar_count: self.bar_count,
            node_states: self.states.clone(),
            node_vals: self.vals.clone(),
        }
    }

    /// Restores executor state from a previously taken [`ExecutorSnapshot`].
    pub fn restore(&mut self, snap: &ExecutorSnapshot) -> Result<(), ExecError> {
        if snap.ir_fingerprint != self.ir_fingerprint
            || snap.node_states.len() != self.states.len()
            || snap.node_vals.len() != self.vals.len()
        {
            return Err(ExecError::SnapshotMismatch);
        }
        self.bar_count = snap.bar_count;
        self.states.clone_from(&snap.node_states);
        self.vals.clone_from(&snap.node_vals);
        Ok(())
    }

    /// Returns the number of bars processed so far.
    pub fn bar_count(&self) -> u64 {
        self.bar_count
    }

    /// Returns the latest computed [`NodeVal`] for `id`.
    pub fn node_value(&self, id: NodeId) -> Option<NodeVal> {
        self.vals.get(id.0 as usize).copied()
    }

    fn step_inner(&mut self, bar: Bar) -> BarOutput {
        for i in 0..self.ops.len() {
            let val = eval_compiled(
                self.ops[i],
                &mut self.states[i],
                &self.vals,
                &self.strings,
                &bar,
            );
            self.vals[i] = val;
        }
        self.bar_count += 1;

        let entry = match self.vals[self.entry_idx] {
            NodeVal::Bool(b) => b,
            _ => TriBool::Unknown,
        };
        let exit = match self.vals[self.exit_idx] {
            NodeVal::Bool(b) => b,
            _ => TriBool::Unknown,
        };

        BarOutput {
            timestamp: bar.timestamp,
            entry,
            exit,
            entry_signal: entry.to_signal(),
            exit_signal: exit.to_signal(),
        }
    }
}

#[inline]
fn num_at(vals: &[NodeVal], idx: usize) -> f64 {
    match vals[idx] {
        NodeVal::Num(v) => v,
        _ => f64::NAN,
    }
}

#[inline]
fn bool_at(vals: &[NodeVal], idx: usize) -> TriBool {
    match vals[idx] {
        NodeVal::Bool(b) => b,
        _ => TriBool::Unknown,
    }
}

fn eval_compiled(
    op: CompiledOp,
    state: &mut NodeState,
    vals: &[NodeVal],
    strings: &[String],
    bar: &Bar,
) -> NodeVal {
    match op {
        CompiledOp::Series(field) => {
            let v = match field {
                SeriesField::Open => bar.open,
                SeriesField::High => bar.high,
                SeriesField::Low => bar.low,
                SeriesField::Close => bar.close,
                SeriesField::Volume => bar.volume,
            };
            NodeVal::Num(clean(v))
        }
        CompiledOp::Const(v) => v,
        CompiledOp::Neg(x) => {
            let v = num_at(vals, x);
            NodeVal::Num(if v.is_finite() { clean(-v) } else { f64::NAN })
        }
        CompiledOp::Pos(x) => NodeVal::Num(num_at(vals, x)),
        CompiledOp::Bin { op, l, r } => {
            let a = num_at(vals, l);
            let b = num_at(vals, r);
            if !a.is_finite() || !b.is_finite() {
                return NodeVal::Num(f64::NAN);
            }
            let out = match op {
                BinOp::Add => clean(a + b),
                BinOp::Sub => clean(a - b),
                BinOp::Mul => clean(a * b),
                BinOp::Div => safe_div(a, b),
                BinOp::Pow => clean(a.powf(b)),
            };
            NodeVal::Num(out)
        }
        CompiledOp::Cmp { op, l, r } => {
            let res = match (vals[l], vals[r]) {
                (NodeVal::Num(a), NodeVal::Num(b)) => {
                    if !a.is_finite() || !b.is_finite() {
                        TriBool::Unknown
                    } else {
                        let b_res = match op {
                            CmpOp::Eq => a == b,
                            CmpOp::NotEq => a != b,
                            CmpOp::Lt => a < b,
                            CmpOp::LtEq => a <= b,
                            CmpOp::Gt => a > b,
                            CmpOp::GtEq => a >= b,
                        };
                        if b_res {
                            TriBool::True
                        } else {
                            TriBool::False
                        }
                    }
                }
                (NodeVal::Bool(a), NodeVal::Bool(b)) => match (a, b) {
                    (TriBool::Unknown, _) | (_, TriBool::Unknown) => TriBool::Unknown,
                    _ => {
                        let b_res = match op {
                            CmpOp::Eq => a == b,
                            CmpOp::NotEq => a != b,
                            _ => return NodeVal::Bool(TriBool::Unknown),
                        };
                        if b_res {
                            TriBool::True
                        } else {
                            TriBool::False
                        }
                    }
                },
                (NodeVal::Tf(a), NodeVal::Tf(b)) => {
                    let b_res = match op {
                        CmpOp::Eq => a.minutes() == b.minutes(),
                        CmpOp::NotEq => a.minutes() != b.minutes(),
                        CmpOp::Lt => a.minutes() < b.minutes(),
                        CmpOp::LtEq => a.minutes() <= b.minutes(),
                        CmpOp::Gt => a.minutes() > b.minutes(),
                        CmpOp::GtEq => a.minutes() >= b.minutes(),
                    };
                    if b_res {
                        TriBool::True
                    } else {
                        TriBool::False
                    }
                }
                (NodeVal::Str(a), NodeVal::Str(b)) => {
                    let sa = &strings[a as usize];
                    let sb = &strings[b as usize];
                    let b_res = match op {
                        CmpOp::Eq => sa == sb,
                        CmpOp::NotEq => sa != sb,
                        _ => return NodeVal::Bool(TriBool::Unknown),
                    };
                    if b_res {
                        TriBool::True
                    } else {
                        TriBool::False
                    }
                }
                _ => TriBool::Unknown,
            };
            NodeVal::Bool(res)
        }
        CompiledOp::Not(x) => NodeVal::Bool(bool_at(vals, x).not()),
        CompiledOp::And(l, r) => NodeVal::Bool(bool_at(vals, l).and(bool_at(vals, r))),
        CompiledOp::Or(l, r) => NodeVal::Bool(bool_at(vals, l).or(bool_at(vals, r))),
        CompiledOp::IfElse { c, t, e } => {
            let cond = bool_at(vals, c);
            match (vals[t], vals[e]) {
                (NodeVal::Num(a), NodeVal::Num(b)) => NodeVal::Num(match cond {
                    TriBool::True => clean(a),
                    TriBool::False => clean(b),
                    TriBool::Unknown => f64::NAN,
                }),
                (NodeVal::Bool(a), NodeVal::Bool(b)) => NodeVal::Bool(match cond {
                    TriBool::True => a,
                    TriBool::False => b,
                    TriBool::Unknown => TriBool::Unknown,
                }),
                (NodeVal::Tf(a), NodeVal::Tf(b)) => match cond {
                    TriBool::True => NodeVal::Tf(a),
                    TriBool::False => NodeVal::Tf(b),
                    TriBool::Unknown => NodeVal::Num(f64::NAN),
                },
                (NodeVal::Str(a), NodeVal::Str(b)) => match cond {
                    TriBool::True => NodeVal::Str(a),
                    TriBool::False => NodeVal::Str(b),
                    TriBool::Unknown => NodeVal::Num(f64::NAN),
                },
                _ => NodeVal::Num(f64::NAN),
            }
        }
        CompiledOp::Lag {
            src,
            always_missing,
        } => {
            let cur = vals[src];
            if always_missing {
                return match cur {
                    NodeVal::Bool(_) => NodeVal::Bool(TriBool::Unknown),
                    _ => NodeVal::Num(f64::NAN),
                };
            }
            let NodeState::Lag(ring) = state else {
                return cur;
            };
            ring.push(cur)
        }
        CompiledOp::Abs(x) => {
            let v = num_at(vals, x);
            NodeVal::Num(if v.is_finite() { v.abs() } else { f64::NAN })
        }
        CompiledOp::Sqrt(x) => {
            let v = num_at(vals, x);
            NodeVal::Num(if v.is_finite() && v >= 0.0 {
                v.sqrt()
            } else {
                f64::NAN
            })
        }
        CompiledOp::Log(x) => {
            let v = num_at(vals, x);
            NodeVal::Num(if v.is_finite() && v > 0.0 {
                v.ln()
            } else {
                f64::NAN
            })
        }
        CompiledOp::Exp(x) => {
            let v = num_at(vals, x);
            NodeVal::Num(if v.is_finite() {
                clean(v.exp())
            } else {
                f64::NAN
            })
        }
        CompiledOp::Min(a, b) => {
            let x = num_at(vals, a);
            let y = num_at(vals, b);
            NodeVal::Num(if x.is_finite() && y.is_finite() {
                x.min(y)
            } else {
                f64::NAN
            })
        }
        CompiledOp::Max(a, b) => {
            let x = num_at(vals, a);
            let y = num_at(vals, b);
            NodeVal::Num(if x.is_finite() && y.is_finite() {
                x.max(y)
            } else {
                f64::NAN
            })
        }
        CompiledOp::Where { c, a, b } => {
            let cond = bool_at(vals, c);
            let x = num_at(vals, a);
            let y = num_at(vals, b);
            NodeVal::Num(match cond {
                TriBool::True => clean(x),
                TriBool::False => clean(y),
                TriBool::Unknown => f64::NAN,
            })
        }
        CompiledOp::Clamp { x, lo, hi } => {
            let v = num_at(vals, x);
            let l = num_at(vals, lo);
            let h = num_at(vals, hi);
            NodeVal::Num(
                if v.is_finite() && l.is_finite() && h.is_finite() && l <= h {
                    v.clamp(l, h)
                } else {
                    f64::NAN
                },
            )
        }
        CompiledOp::Mod(a, b) => {
            let x = num_at(vals, a);
            let y = num_at(vals, b);
            NodeVal::Num(if x.is_finite() && y.is_finite() && y != 0.0 {
                clean(x - y * (x / y).floor())
            } else {
                f64::NAN
            })
        }
        CompiledOp::Diff(x) => {
            let cur = num_at(vals, x);
            let NodeState::Diff(ring) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let prev = ring.push(cur);
            NodeVal::Num(if cur.is_finite() && prev.is_finite() {
                clean(cur - prev)
            } else {
                f64::NAN
            })
        }
        CompiledOp::PctChange(x) => {
            let cur = num_at(vals, x);
            let NodeState::PctChange(ring) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let prev = ring.push(cur);
            NodeVal::Num(safe_div(cur - prev, prev))
        }
        CompiledOp::Cumsum(x) => {
            let cur = num_at(vals, x);
            let NodeState::Cumsum(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(cur))
        }
        CompiledOp::Sum(x) => {
            let cur = num_at(vals, x);
            let NodeState::Sum(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(SumSummary::Valid { sum, .. }) => clean(sum),
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Mean(x) => {
            let cur = num_at(vals, x);
            let NodeState::Sum(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(SumSummary::Valid { count, sum }) => clean(sum / (count as f64)),
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Var(x) => {
            let cur = num_at(vals, x);
            let NodeState::Stats(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(StatsSummary::Valid { count, m2, .. }) if count >= 2 => {
                    clean(m2 / ((count - 1) as f64))
                }
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Std(x) => {
            let cur = num_at(vals, x);
            let NodeState::Stats(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(StatsSummary::Valid { count, m2, .. }) if count >= 2 => {
                    clean((m2 / ((count - 1) as f64)).sqrt())
                }
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Bollinger { x, mult } => {
            let cur = num_at(vals, x);
            let NodeState::Stats(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(StatsSummary::Valid { count, mean, m2 }) if count >= 2 => {
                    let sd = (m2 / ((count - 1) as f64)).sqrt();
                    clean(mean + mult * sd)
                }
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Ema(x) => {
            let cur = num_at(vals, x);
            let NodeState::Ewm(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(cur))
        }
        CompiledOp::Wma { x, denom } => {
            let cur = num_at(vals, x);
            let NodeState::Wma(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(WmaSummary::Valid { wsum, .. }) => clean(wsum / denom),
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Highest(x) => {
            let cur = num_at(vals, x);
            let NodeState::Extrema(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(ExtremaSummary::Valid { max, .. }) => clean(max),
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Lowest(x) => {
            let cur = num_at(vals, x);
            let NodeState::Extrema(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(ExtremaSummary::Valid { min, .. }) => clean(min),
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Rsi(x) => {
            let cur = num_at(vals, x);
            let NodeState::Rsi(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(cur))
        }
        CompiledOp::Tr { h, l, c } => {
            let NodeState::Tr(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(num_at(vals, h), num_at(vals, l), num_at(vals, c)))
        }
        CompiledOp::Atr { h, l, c } => {
            let NodeState::Atr(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(num_at(vals, h), num_at(vals, l), num_at(vals, c)))
        }
        CompiledOp::Macd(x) => {
            let NodeState::Macd(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(num_at(vals, x)))
        }
        CompiledOp::Cov(x, y) => {
            let vx = num_at(vals, x);
            let vy = num_at(vals, y);
            let NodeState::PairStats(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push((vx, vy)) {
                Some(PairStatsSummary::Valid { count, cxy, .. }) if count >= 2 => {
                    clean(cxy / ((count - 1) as f64))
                }
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::Corr(x, y) => {
            let vx = num_at(vals, x);
            let vy = num_at(vals, y);
            let NodeState::PairStats(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push((vx, vy)) {
                Some(PairStatsSummary::Valid {
                    count,
                    m2x,
                    m2y,
                    cxy,
                    ..
                }) if count >= 2 => {
                    let denom = (m2x * m2y).sqrt();
                    if denom == 0.0 || !denom.is_finite() {
                        f64::NAN
                    } else {
                        clean(cxy / denom)
                    }
                }
                _ => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::LinregSlope(x) => {
            let cur = num_at(vals, x);
            let NodeState::Linreg(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(s) => s.slope_and_intercept().0,
                None => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::LinregIntercept(x) => {
            let cur = num_at(vals, x);
            let NodeState::Linreg(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            let out = match win.push(cur) {
                Some(s) => s.slope_and_intercept().1,
                None => f64::NAN,
            };
            NodeVal::Num(out)
        }
        CompiledOp::PercentileRank(x) => {
            let cur = num_at(vals, x);
            let NodeState::Percentile(win) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(win.push(cur))
        }
        CompiledOp::Crossover(a, b) | CompiledOp::Crossunder(a, b) => {
            let NodeState::Cross(st) = state else {
                return NodeVal::Bool(TriBool::Unknown);
            };
            NodeVal::Bool(st.step(num_at(vals, a), num_at(vals, b)))
        }
        CompiledOp::Since(c) => {
            let NodeState::Since(st) = state else {
                return NodeVal::Num(f64::NAN);
            };
            NodeVal::Num(st.step(bool_at(vals, c)))
        }
    }
}

fn input_idx(node: &IrNode, pos: usize, node_count: usize) -> Result<usize, ExecError> {
    let id = node
        .inputs
        .get(pos)
        .ok_or_else(|| ExecError::InvalidNode(format!("missing input {pos} on {}", node.id)))?;
    let idx = id.0 as usize;
    if idx >= node.id.0 as usize || idx >= node_count {
        return Err(ExecError::InvalidNode(format!(
            "non-topological input {id} on {}",
            node.id
        )));
    }
    Ok(idx)
}

fn const_usize(node: &IrNode, idx: usize, min: usize) -> Result<usize, ExecError> {
    match node.const_args.get(idx) {
        Some(IrConst::Int(v)) => {
            let n = usize::try_from(*v).map_err(|_| {
                ExecError::InvalidNode(format!("invalid int const arg {idx} on {}", node.id))
            })?;
            if n < min || n > MAX_BUFFER_LEN {
                return Err(ExecError::InvalidNode(format!(
                    "window {n} out of range [{min}, {MAX_BUFFER_LEN}] on {}",
                    node.id
                )));
            }
            Ok(n)
        }
        _ => Err(ExecError::InvalidNode(format!(
            "expected int const arg {idx} on {}",
            node.id
        ))),
    }
}

fn const_f64(node: &IrNode, idx: usize) -> Result<f64, ExecError> {
    match node.const_args.get(idx) {
        Some(IrConst::Dec { value, pct }) => {
            let mut v = value.to_f64().unwrap_or(f64::NAN);
            if *pct {
                v /= 100.0;
            }
            Ok(v)
        }
        Some(IrConst::Int(i)) => Ok(*i as f64),
        _ => Err(ExecError::InvalidNode(format!(
            "expected decimal const arg {idx} on {}",
            node.id
        ))),
    }
}

fn init_val_for_ty(ty: IrTy) -> NodeVal {
    match ty {
        IrTy::Bool => NodeVal::Bool(TriBool::Unknown),
        _ => NodeVal::Num(f64::NAN),
    }
}

fn compile_node(
    node: &IrNode,
    node_count: usize,
    strings: &mut Vec<String>,
) -> Result<(CompiledOp, NodeState, NodeVal), ExecError> {
    let init = init_val_for_ty(node.ty);
    match &node.op {
        IrOp::Series(field) => Ok((CompiledOp::Series(*field), NodeState::Stateless, init)),
        IrOp::Const(c) => {
            let v = match c {
                IrConst::Dec { value, pct } => {
                    let mut f = value.to_f64().unwrap_or(f64::NAN);
                    if *pct {
                        f /= 100.0;
                    }
                    NodeVal::Num(clean(f))
                }
                IrConst::Int(i) => NodeVal::Num(*i as f64),
                IrConst::Bool(b) => NodeVal::Bool(if *b { TriBool::True } else { TriBool::False }),
                IrConst::Tf(tf) => NodeVal::Tf(*tf),
                IrConst::Str(s) => {
                    let sid = strings.len() as u32;
                    strings.push(s.clone());
                    NodeVal::Str(sid)
                }
            };
            Ok((CompiledOp::Const(v), NodeState::Stateless, v))
        }
        IrOp::Unary(op) => {
            let x = input_idx(node, 0, node_count)?;
            let cop = match op {
                UnOp::Neg => CompiledOp::Neg(x),
                UnOp::Pos => CompiledOp::Pos(x),
            };
            Ok((cop, NodeState::Stateless, init))
        }
        IrOp::Binary(op) => {
            let l = input_idx(node, 0, node_count)?;
            let r = input_idx(node, 1, node_count)?;
            Ok((
                CompiledOp::Bin { op: *op, l, r },
                NodeState::Stateless,
                init,
            ))
        }
        IrOp::Compare(op) => {
            let l = input_idx(node, 0, node_count)?;
            let r = input_idx(node, 1, node_count)?;
            Ok((
                CompiledOp::Cmp { op: *op, l, r },
                NodeState::Stateless,
                init,
            ))
        }
        IrOp::Not => {
            let x = input_idx(node, 0, node_count)?;
            Ok((CompiledOp::Not(x), NodeState::Stateless, init))
        }
        IrOp::And => {
            let l = input_idx(node, 0, node_count)?;
            let r = input_idx(node, 1, node_count)?;
            Ok((CompiledOp::And(l, r), NodeState::Stateless, init))
        }
        IrOp::Or => {
            let l = input_idx(node, 0, node_count)?;
            let r = input_idx(node, 1, node_count)?;
            Ok((CompiledOp::Or(l, r), NodeState::Stateless, init))
        }
        IrOp::IfElse => {
            let c = input_idx(node, 0, node_count)?;
            let t = input_idx(node, 1, node_count)?;
            let e = input_idx(node, 2, node_count)?;
            Ok((CompiledOp::IfElse { c, t, e }, NodeState::Stateless, init))
        }
        IrOp::Lag(k) => {
            let src = input_idx(node, 0, node_count)?;
            match usize::try_from(*k) {
                Ok(lag) if lag <= MAX_BUFFER_LEN => Ok((
                    CompiledOp::Lag {
                        src,
                        always_missing: false,
                    },
                    NodeState::Lag(LagRing::new(lag, init)),
                    init,
                )),
                _ => Ok((
                    CompiledOp::Lag {
                        src,
                        always_missing: true,
                    },
                    NodeState::Stateless,
                    init,
                )),
            }
        }
        IrOp::Builtin(name) => compile_builtin(name, node, node_count, init),
    }
}

fn compile_builtin(
    name: &str,
    node: &IrNode,
    node_count: usize,
    init: NodeVal,
) -> Result<(CompiledOp, NodeState, NodeVal), ExecError> {
    match name {
        "abs" => Ok((
            CompiledOp::Abs(input_idx(node, 0, node_count)?),
            NodeState::Stateless,
            init,
        )),
        "sqrt" => Ok((
            CompiledOp::Sqrt(input_idx(node, 0, node_count)?),
            NodeState::Stateless,
            init,
        )),
        "log" => Ok((
            CompiledOp::Log(input_idx(node, 0, node_count)?),
            NodeState::Stateless,
            init,
        )),
        "exp" => Ok((
            CompiledOp::Exp(input_idx(node, 0, node_count)?),
            NodeState::Stateless,
            init,
        )),
        "min" => Ok((
            CompiledOp::Min(
                input_idx(node, 0, node_count)?,
                input_idx(node, 1, node_count)?,
            ),
            NodeState::Stateless,
            init,
        )),
        "max" => Ok((
            CompiledOp::Max(
                input_idx(node, 0, node_count)?,
                input_idx(node, 1, node_count)?,
            ),
            NodeState::Stateless,
            init,
        )),
        "where" => Ok((
            CompiledOp::Where {
                c: input_idx(node, 0, node_count)?,
                a: input_idx(node, 1, node_count)?,
                b: input_idx(node, 2, node_count)?,
            },
            NodeState::Stateless,
            init,
        )),
        "clamp" => Ok((
            CompiledOp::Clamp {
                x: input_idx(node, 0, node_count)?,
                lo: input_idx(node, 1, node_count)?,
                hi: input_idx(node, 2, node_count)?,
            },
            NodeState::Stateless,
            init,
        )),
        "mod" => Ok((
            CompiledOp::Mod(
                input_idx(node, 0, node_count)?,
                input_idx(node, 1, node_count)?,
            ),
            NodeState::Stateless,
            init,
        )),
        "diff" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 0)?;
            Ok((
                CompiledOp::Diff(x),
                NodeState::Diff(LagRing::new(n, f64::NAN)),
                init,
            ))
        }
        "pct_change" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::PctChange(x),
                NodeState::PctChange(LagRing::new(n, f64::NAN)),
                init,
            ))
        }
        "cumsum" => Ok((
            CompiledOp::Cumsum(input_idx(node, 0, node_count)?),
            NodeState::Cumsum(CumsumState::new()),
            init,
        )),
        "sum" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Sum(x),
                NodeState::Sum(TwoStackWindow::new(n)),
                init,
            ))
        }
        "mean" | "sma" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Mean(x),
                NodeState::Sum(TwoStackWindow::new(n)),
                init,
            ))
        }
        "var" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Var(x),
                NodeState::Stats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "std" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Std(x),
                NodeState::Stats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "bollinger" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 2)?;
            let mult = const_f64(node, 1)?;
            Ok((
                CompiledOp::Bollinger { x, mult },
                NodeState::Stats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "ema" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            let alpha = 2.0 / ((n as f64) + 1.0);
            Ok((
                CompiledOp::Ema(x),
                NodeState::Ewm(EwmState::new(alpha, n)),
                init,
            ))
        }
        "rma" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            let alpha = 1.0 / (n as f64);
            Ok((
                CompiledOp::Ema(x),
                NodeState::Ewm(EwmState::new(alpha, n)),
                init,
            ))
        }
        "wma" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            let nf = n as f64;
            let denom = nf * (nf + 1.0) / 2.0;
            Ok((
                CompiledOp::Wma { x, denom },
                NodeState::Wma(TwoStackWindow::new(n)),
                init,
            ))
        }
        "highest" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Highest(x),
                NodeState::Extrema(TwoStackWindow::new(n)),
                init,
            ))
        }
        "lowest" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Lowest(x),
                NodeState::Extrema(TwoStackWindow::new(n)),
                init,
            ))
        }
        "rsi" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((CompiledOp::Rsi(x), NodeState::Rsi(RsiState::new(n)), init))
        }
        "tr" => Ok((
            CompiledOp::Tr {
                h: input_idx(node, 0, node_count)?,
                l: input_idx(node, 1, node_count)?,
                c: input_idx(node, 2, node_count)?,
            },
            NodeState::Tr(TrState::new()),
            init,
        )),
        "atr" => {
            let h = input_idx(node, 0, node_count)?;
            let l = input_idx(node, 1, node_count)?;
            let c = input_idx(node, 2, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Atr { h, l, c },
                NodeState::Atr(AtrState::new(n)),
                init,
            ))
        }
        "macd" => {
            let x = input_idx(node, 0, node_count)?;
            let fast = const_usize(node, 0, 1)?;
            let slow = const_usize(node, 1, 1)?;
            let sig = const_usize(node, 2, 1)?;
            Ok((
                CompiledOp::Macd(x),
                NodeState::Macd(MacdState::new(fast, slow, sig)),
                init,
            ))
        }
        "cov" => {
            let x = input_idx(node, 0, node_count)?;
            let y = input_idx(node, 1, node_count)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Cov(x, y),
                NodeState::PairStats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "corr" => {
            let x = input_idx(node, 0, node_count)?;
            let y = input_idx(node, 1, node_count)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Corr(x, y),
                NodeState::PairStats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "linreg_slope" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::LinregSlope(x),
                NodeState::Linreg(TwoStackWindow::new(n)),
                init,
            ))
        }
        "linreg_intercept" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::LinregIntercept(x),
                NodeState::Linreg(TwoStackWindow::new(n)),
                init,
            ))
        }
        "percentile_rank" => {
            let x = input_idx(node, 0, node_count)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::PercentileRank(x),
                NodeState::Percentile(PercentileWindow::new(n)),
                init,
            ))
        }
        "crossover" => {
            let a = input_idx(node, 0, node_count)?;
            let b = input_idx(node, 1, node_count)?;
            Ok((
                CompiledOp::Crossover(a, b),
                NodeState::Cross(CrossState::new(true)),
                init,
            ))
        }
        "crossunder" => {
            let a = input_idx(node, 0, node_count)?;
            let b = input_idx(node, 1, node_count)?;
            Ok((
                CompiledOp::Crossunder(a, b),
                NodeState::Cross(CrossState::new(false)),
                init,
            ))
        }
        "since" => {
            let c = input_idx(node, 0, node_count)?;
            Ok((
                CompiledOp::Since(c),
                NodeState::Since(SinceState::new()),
                init,
            ))
        }
        _ => Err(ExecError::UnsupportedOp(name.to_string())),
    }
}

fn fingerprint_ir(ir: &IrGraph) -> u64 {
    let mut h = DefaultHasher::new();
    ir.base_clock.hash(&mut h);
    ir.entry.hash(&mut h);
    ir.exit.hash(&mut h);
    ir.nodes.len().hash(&mut h);
    for node in &ir.nodes {
        node.id.hash(&mut h);
        node.op.hash(&mut h);
        node.inputs.hash(&mut h);
        node.const_args.hash(&mut h);
        node.clock.hash(&mut h);
        node.ty.hash(&mut h);
    }
    h.finish()
}
