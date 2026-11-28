use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use mln_check::Tf;
use mln_lang::{BinOp, CmpOp, UnOp};
use mln_std::eval::{clean, safe_div};
use mln_std::TriBool;
use rust_decimal::prelude::ToPrimitive;

use crate::oracle::{ExecError, MAX_BUFFER_LEN};
use crate::rolling::{
    ExtremaSummary, LagRing, PairStatsSummary, PercentileWindow, StatsSummary, SumSummary,
    TwoStackWindow, WmaSummary,
};
use crate::state::{
    validate_bar, validate_bar_allowing_nans, AtrState, Bar, BarOutput, CrossState, CumsumState,
    EwmState, ExecutorSnapshot, MacdState, NodeState, NodeVal, PositionSide, PositionView,
    RsiState, SessionCalendar, SinceState, SymbolState, TfTracker, TrState,
};
use mln_ir::{
    IrClock, IrConst, IrExecStep, IrGraph, IrNode, IrOp, IrReset, IrStateVar, IrTrigger, IrTy,
    NodeId, PositionField, SeriesField, SymbolField,
};

#[derive(Debug, Clone, Copy)]
enum CompiledOp {
    Series(SeriesField),
    Position(PositionField),
    SymbolSeries { sym_idx: usize, field: SymbolField },
    StateRead(usize),
    Const(NodeVal),
    Neg(usize),
    Pos(usize),
    Bin { op: BinOp, l: usize, r: usize },
    Cmp { op: CmpOp, l: usize, r: usize },
    Not(usize),
    And(usize, usize),
    Or(usize, usize),
    IfElse { c: usize, t: usize, e: usize },
    Lag(usize),
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

#[derive(Debug, Clone, Copy)]
enum CompiledReset {
    None,
    Enter,
    Exit,
    Expr(usize),
}

#[derive(Debug, Clone, Copy)]
struct CompiledStateVar {
    init_idx: usize,
    reset: CompiledReset,
}

#[derive(Debug, Clone, Copy)]
enum CompiledTrigger {
    OnEnter,
    OnExit,
    OnExpr(usize),
    WhileLong,
    WhileShort,
    WhileFlat,
    WhileExpr(usize),
}

#[derive(Debug, Clone)]
struct CompiledStateBlock {
    trigger: CompiledTrigger,
    nodes: Vec<usize>,
    updates: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy)]
enum CompiledStep {
    EvalNode(usize),
    ResetState(usize),
    ExecBlock(usize),
}

/// Incremental bar-by-bar and chunked executor for a lowered [`IrGraph`].
#[derive(Debug, Clone)]
pub struct Executor {
    ir_fingerprint: u64,
    calendar: SessionCalendar,
    ops: Vec<CompiledOp>,
    states: Vec<NodeState>,
    vals: Vec<NodeVal>,
    node_is_base: Vec<bool>,
    strings: Vec<String>,
    state_vars: Vec<CompiledStateVar>,
    state_slots: Vec<NodeVal>,
    state_initialized: Vec<bool>,
    state_names: Vec<String>,
    state_blocks: Vec<CompiledStateBlock>,
    schedule: Vec<CompiledStep>,
    symbol_states: Vec<SymbolState>,
    tf_trackers: Vec<TfTracker>,
    entry_idx: usize,
    exit_idx: usize,
    prev_side: PositionSide,
    last_timestamp: Option<i64>,
    bar_count: u64,
}

impl Executor {
    /// Compiles and allocates all per-node buffers for `ir` using UTC calendar bucketing.
    ///
    /// No heap allocations happen inside [`Self::step`] or [`Self::step_with_context`]
    /// after construction.
    pub fn new(ir: &IrGraph) -> Result<Self, ExecError> {
        Self::new_with_calendar(ir, SessionCalendar::utc())
    }

    /// Compiles and allocates all per-node buffers for `ir` with a custom [`SessionCalendar`].
    pub fn new_with_calendar(ir: &IrGraph, calendar: SessionCalendar) -> Result<Self, ExecError> {
        let mut ops = Vec::with_capacity(ir.nodes.len());
        let mut states = Vec::with_capacity(ir.nodes.len());
        let mut vals = Vec::with_capacity(ir.nodes.len());
        let mut node_is_base = Vec::with_capacity(ir.nodes.len());
        let mut strings = vec!["flat".to_string(), "long".to_string(), "short".to_string()];
        let mut tf_map: BTreeMap<Tf, Vec<usize>> = BTreeMap::new();

        for (i, node) in ir.nodes.iter().enumerate() {
            if node.id.0 as usize != i {
                return Err(ExecError::InvalidNode(format!(
                    "node at index {i} has mismatched id {}",
                    node.id
                )));
            }
            let (op, state, init_val) = compile_node(
                node,
                i,
                &ir.nodes,
                ir.symbols.len(),
                &ir.states,
                &mut strings,
            )?;
            ops.push(op);
            states.push(state);
            vals.push(init_val);
            if let IrClock::Tf(tf) = node.clock {
                node_is_base.push(false);
                tf_map.entry(tf).or_default().push(i);
            } else {
                node_is_base.push(true);
            }
        }

        let mut tf_trackers: Vec<TfTracker> = tf_map
            .into_iter()
            .map(|(tf, node_indices)| TfTracker::new(tf, ir.symbols.len(), node_indices))
            .collect();
        // Evaluate coarsest timeframe buckets before finer ones if both complete on the same bar.
        tf_trackers.sort_by_key(|a| std::cmp::Reverse(a.tf.minutes()));

        let mut state_vars = Vec::with_capacity(ir.states.len());
        let mut state_slots = Vec::with_capacity(ir.states.len());
        let mut state_names = Vec::with_capacity(ir.states.len());
        for (sid, st) in ir.states.iter().enumerate() {
            if st.id.0 as usize != sid {
                return Err(ExecError::InvalidNode(format!(
                    "state variable at index {sid} has mismatched id {}",
                    st.id
                )));
            }
            let init_idx = st.init.0 as usize;
            let init_node = ir
                .nodes
                .get(init_idx)
                .ok_or_else(|| ExecError::InvalidNode("invalid state init node".into()))?;
            if !same_value_kind(init_node.ty, st.ty) {
                return Err(ExecError::InvalidNode(format!(
                    "state `{}` init type {:?} does not match {:?}",
                    st.name, init_node.ty, st.ty
                )));
            }
            let reset = match st.reset {
                IrReset::None => CompiledReset::None,
                IrReset::Enter => CompiledReset::Enter,
                IrReset::Exit => CompiledReset::Exit,
                IrReset::Expr(id) => {
                    let cidx = id.0 as usize;
                    match ir.nodes.get(cidx) {
                        Some(n) if n.ty == IrTy::Bool => CompiledReset::Expr(cidx),
                        _ => {
                            return Err(ExecError::InvalidNode(
                                "state reset condition must be a valid Bool node".into(),
                            ))
                        }
                    }
                }
            };
            state_vars.push(CompiledStateVar { init_idx, reset });
            state_slots.push(init_val_for_ty(st.ty));
            state_names.push(st.name.clone());
        }

        let mut state_blocks = Vec::with_capacity(ir.state_blocks.len());
        for blk in &ir.state_blocks {
            let trigger = match blk.trigger {
                IrTrigger::OnEnter => CompiledTrigger::OnEnter,
                IrTrigger::OnExit => CompiledTrigger::OnExit,
                IrTrigger::WhileLong => CompiledTrigger::WhileLong,
                IrTrigger::WhileShort => CompiledTrigger::WhileShort,
                IrTrigger::WhileFlat => CompiledTrigger::WhileFlat,
                IrTrigger::OnExpr(id) => {
                    let cidx = id.0 as usize;
                    match ir.nodes.get(cidx) {
                        Some(n) if n.ty == IrTy::Bool => CompiledTrigger::OnExpr(cidx),
                        _ => {
                            return Err(ExecError::InvalidNode(
                                "`on` condition must be a valid Bool node".into(),
                            ))
                        }
                    }
                }
                IrTrigger::WhileExpr(id) => {
                    let cidx = id.0 as usize;
                    match ir.nodes.get(cidx) {
                        Some(n) if n.ty == IrTy::Bool => CompiledTrigger::WhileExpr(cidx),
                        _ => {
                            return Err(ExecError::InvalidNode(
                                "`while` guard must be a valid Bool node".into(),
                            ))
                        }
                    }
                }
            };
            let mut nodes = Vec::with_capacity(blk.nodes.len());
            for id in &blk.nodes {
                let nidx = id.0 as usize;
                if nidx >= ir.nodes.len() {
                    return Err(ExecError::InvalidNode("invalid state block node".into()));
                }
                nodes.push(nidx);
            }
            let mut updates = Vec::with_capacity(blk.updates.len());
            for &(sid, vid) in &blk.updates {
                let sidx = sid.0 as usize;
                let vidx = vid.0 as usize;
                if sidx >= ir.states.len() || vidx >= ir.nodes.len() {
                    return Err(ExecError::InvalidNode("invalid state update target".into()));
                }
                if !same_value_kind(ir.states[sidx].ty, ir.nodes[vidx].ty) {
                    return Err(ExecError::InvalidNode(
                        "state update value type mismatch".into(),
                    ));
                }
                updates.push((sidx, vidx));
            }
            state_blocks.push(CompiledStateBlock {
                trigger,
                nodes,
                updates,
            });
        }

        let schedule = if ir.schedule.is_empty() && !ir.nodes.is_empty() {
            (0..ir.nodes.len()).map(CompiledStep::EvalNode).collect()
        } else {
            let mut out = Vec::with_capacity(ir.schedule.len());
            for step in &ir.schedule {
                match *step {
                    IrExecStep::EvalNode(id) => {
                        let idx = id.0 as usize;
                        if idx >= ir.nodes.len() {
                            return Err(ExecError::InvalidNode("invalid schedule node".into()));
                        }
                        out.push(CompiledStep::EvalNode(idx));
                    }
                    IrExecStep::ResetState(sid) => {
                        let idx = sid.0 as usize;
                        if idx >= ir.states.len() {
                            return Err(ExecError::InvalidNode("invalid schedule state".into()));
                        }
                        out.push(CompiledStep::ResetState(idx));
                    }
                    IrExecStep::ExecBlock(bidx) => {
                        if bidx >= ir.state_blocks.len() {
                            return Err(ExecError::InvalidNode("invalid schedule block".into()));
                        }
                        out.push(CompiledStep::ExecBlock(bidx));
                    }
                }
            }
            out
        };

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

        let state_count = ir.states.len();
        let sym_count = ir.symbols.len();
        Ok(Self {
            ir_fingerprint: fingerprint_ir(ir),
            calendar,
            ops,
            states,
            vals,
            node_is_base,
            strings,
            state_vars,
            state_slots,
            state_initialized: vec![false; state_count],
            state_names,
            state_blocks,
            schedule,
            symbol_states: vec![SymbolState::default(); sym_count],
            tf_trackers,
            entry_idx,
            exit_idx,
            prev_side: PositionSide::Flat,
            last_timestamp: None,
            bar_count: 0,
        })
    }

    /// Returns the number of secondary symbols expected by [`Self::step_with_context`].
    pub fn symbol_count(&self) -> usize {
        self.symbol_states.len()
    }

    /// Processes one validated bar using flat position context and no secondary symbols.
    pub fn step(&mut self, bar: Bar) -> Result<BarOutput, ExecError> {
        self.step_with_context(bar, PositionView::default(), &[])
    }

    /// Processes one validated bar with pure `PositionView` and aligned secondary symbol bars.
    ///
    /// Rejects bad bars, symbol count mismatches, and backwards timestamps with `Err`
    /// while leaving all internal state completely unchanged.
    pub fn step_with_context(
        &mut self,
        bar: Bar,
        pos: PositionView,
        symbols: &[Option<Bar>],
    ) -> Result<BarOutput, ExecError> {
        if symbols.len() != self.symbol_states.len() {
            return Err(ExecError::SymbolCountMismatch {
                expected: self.symbol_states.len(),
                got: symbols.len(),
            });
        }
        if !self.tf_trackers.is_empty() {
            if let Some(prev) = self.last_timestamp {
                if bar.timestamp < prev {
                    return Err(ExecError::NonMonotonicTimestamp {
                        prev,
                        cur: bar.timestamp,
                    });
                }
            }
        }
        validate_bar(&bar)?;
        for sym_bar in symbols.iter().flatten() {
            validate_bar(sym_bar)?;
        }
        Ok(self.step_inner(bar, pos, symbols))
    }

    /// Processes one bar while permitting `NaN` holes in OHLCV fields (for differential testing).
    ///
    /// Still rejects infinities, `high < low`, and negative `volume` without mutating state.
    pub fn step_allowing_nans(&mut self, bar: Bar) -> Result<BarOutput, ExecError> {
        if !self.symbol_states.is_empty() {
            return Err(ExecError::SymbolCountMismatch {
                expected: self.symbol_states.len(),
                got: 0,
            });
        }
        if !self.tf_trackers.is_empty() {
            if let Some(prev) = self.last_timestamp {
                if bar.timestamp < prev {
                    return Err(ExecError::NonMonotonicTimestamp {
                        prev,
                        cur: bar.timestamp,
                    });
                }
            }
        }
        validate_bar_allowing_nans(&bar)?;
        Ok(self.step_inner(bar, PositionView::default(), &[]))
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
        if !self.symbol_states.is_empty() {
            return Err(ExecError::SymbolCountMismatch {
                expected: self.symbol_states.len(),
                got: 0,
            });
        }
        let check_ts = !self.tf_trackers.is_empty();
        let mut prev_ts = self.last_timestamp;
        for bar in bars {
            validate_bar(bar)?;
            if check_ts {
                if let Some(prev) = prev_ts {
                    if bar.timestamp < prev {
                        return Err(ExecError::NonMonotonicTimestamp {
                            prev,
                            cur: bar.timestamp,
                        });
                    }
                }
            }
            prev_ts = Some(bar.timestamp);
        }
        for &bar in bars {
            out.push(self.step_inner(bar, PositionView::default(), &[]));
        }
        Ok(())
    }

    /// Chunked execution with explicit `PositionView` and row-major secondary symbol inputs.
    ///
    /// `positions.len()` must equal `bars.len()`, and `symbols_flat.len()` must equal
    /// `bars.len() * self.symbol_count()`. Zero heap allocation when `out` has capacity.
    pub fn run_chunk_with_context_into(
        &mut self,
        bars: &[Bar],
        positions: &[PositionView],
        symbols_flat: &[Option<Bar>],
        out: &mut Vec<BarOutput>,
    ) -> Result<(), ExecError> {
        if positions.len() != bars.len() {
            return Err(ExecError::LengthMismatch);
        }
        let sym_count = self.symbol_states.len();
        if symbols_flat.len() != bars.len().saturating_mul(sym_count) {
            return Err(ExecError::SymbolCountMismatch {
                expected: sym_count,
                got: symbols_flat.len(),
            });
        }
        let check_ts = !self.tf_trackers.is_empty();
        let mut prev_ts = self.last_timestamp;
        for (i, bar) in bars.iter().enumerate() {
            validate_bar(bar)?;
            if check_ts {
                if let Some(prev) = prev_ts {
                    if bar.timestamp < prev {
                        return Err(ExecError::NonMonotonicTimestamp {
                            prev,
                            cur: bar.timestamp,
                        });
                    }
                }
            }
            prev_ts = Some(bar.timestamp);
            for sb in symbols_flat[i * sym_count..(i + 1) * sym_count]
                .iter()
                .flatten()
            {
                validate_bar(sb)?;
            }
        }
        for (i, &bar) in bars.iter().enumerate() {
            let syms = &symbols_flat[i * sym_count..(i + 1) * sym_count];
            out.push(self.step_inner(bar, positions[i], syms));
        }
        Ok(())
    }

    /// Captures a point-in-time snapshot of all per-node, state-var, symbol, and TF state.
    pub fn snapshot(&self) -> ExecutorSnapshot {
        ExecutorSnapshot {
            ir_fingerprint: self.ir_fingerprint,
            calendar: self.calendar,
            bar_count: self.bar_count,
            last_timestamp: self.last_timestamp,
            prev_side: self.prev_side,
            node_states: self.states.clone(),
            node_vals: self.vals.clone(),
            state_slots: self.state_slots.clone(),
            state_initialized: self.state_initialized.clone(),
            symbol_states: self.symbol_states.clone(),
            tf_trackers: self.tf_trackers.clone(),
        }
    }

    /// Restores executor state from a previously taken [`ExecutorSnapshot`].
    pub fn restore(&mut self, snap: &ExecutorSnapshot) -> Result<(), ExecError> {
        if snap.ir_fingerprint != self.ir_fingerprint
            || snap.calendar != self.calendar
            || snap.node_states.len() != self.states.len()
            || snap.node_vals.len() != self.vals.len()
            || snap.state_slots.len() != self.state_slots.len()
            || snap.state_initialized.len() != self.state_initialized.len()
            || snap.symbol_states.len() != self.symbol_states.len()
            || snap.tf_trackers.len() != self.tf_trackers.len()
        {
            return Err(ExecError::SnapshotMismatch);
        }
        self.bar_count = snap.bar_count;
        self.last_timestamp = snap.last_timestamp;
        self.prev_side = snap.prev_side;
        self.states.clone_from(&snap.node_states);
        self.vals.clone_from(&snap.node_vals);
        self.state_slots.clone_from(&snap.state_slots);
        self.state_initialized.clone_from(&snap.state_initialized);
        self.symbol_states.clone_from(&snap.symbol_states);
        self.tf_trackers.clone_from(&snap.tf_trackers);
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

    /// Returns the current value of a declared `state` variable by name.
    pub fn state_value(&self, name: &str) -> Option<NodeVal> {
        let idx = self.state_names.iter().position(|n| n == name)?;
        self.state_slots.get(idx).copied()
    }

    /// Returns the string constant associated with `id`, if `id` evaluates to [`NodeVal::Str`].
    pub fn str_value(&self, id: NodeId) -> Option<&str> {
        match self.node_value(id) {
            Some(NodeVal::Str(idx)) => self.strings.get(idx as usize).map(String::as_str),
            _ => None,
        }
    }

    fn step_inner(&mut self, bar: Bar, pos: PositionView, symbols: &[Option<Bar>]) -> BarOutput {
        let entered = pos.side != PositionSide::Flat && self.prev_side != pos.side;
        let exited = self.prev_side != PositionSide::Flat && pos.side == PositionSide::Flat;

        // 1. Advance higher-timeframe trackers (coarsest first). In-progress higher-TF bars are
        //    never visible until a newer bucket opens and completes the previous bucket cleanly.
        {
            let calendar = self.calendar;
            let ops = &self.ops;
            let states = &mut self.states;
            let vals = &mut self.vals;
            let strings = &self.strings;
            let state_slots = &self.state_slots;
            for tracker in &mut self.tf_trackers {
                let bucket = calendar.bucket(bar.timestamp, tracker.tf);
                if let Some(prev_b) = tracker.current_bucket {
                    if bucket > prev_b {
                        if let Some(completed_primary) = tracker.primary.take_completed() {
                            for s_idx in 0..tracker.completed_symbols.len() {
                                let sym_bar = tracker.symbols[s_idx].take_completed();
                                tracker.completed_symbols[s_idx].update(sym_bar);
                            }
                            let eval_input = EvalInput {
                                bar: &completed_primary,
                                pos: &pos,
                                symbol_states: &tracker.completed_symbols,
                                state_slots,
                            };
                            for &nidx in &tracker.node_indices {
                                let val = eval_compiled(
                                    ops[nidx],
                                    &mut states[nidx],
                                    vals,
                                    strings,
                                    &eval_input,
                                );
                                vals[nidx] = val;
                            }
                        }
                        tracker.current_bucket = Some(bucket);
                    }
                } else {
                    tracker.current_bucket = Some(bucket);
                }
                tracker.primary.accumulate(bar);
                for (s_idx, sym_bar) in symbols.iter().enumerate() {
                    if let Some(sb) = *sym_bar {
                        tracker.symbols[s_idx].accumulate(sb);
                    }
                }
            }
        }

        // 2. Update base-clock aligned secondary symbols.
        for (s_idx, sym_bar) in symbols.iter().enumerate() {
            self.symbol_states[s_idx].update(*sym_bar);
        }

        // 3. Evaluate base-clock DAG nodes, state resets, and guarded on/while blocks in schedule order.
        for step in &self.schedule {
            match *step {
                CompiledStep::EvalNode(i) => {
                    if self.node_is_base[i] {
                        let eval_input = EvalInput {
                            bar: &bar,
                            pos: &pos,
                            symbol_states: &self.symbol_states,
                            state_slots: &self.state_slots,
                        };
                        let val = eval_compiled(
                            self.ops[i],
                            &mut self.states[i],
                            &self.vals,
                            &self.strings,
                            &eval_input,
                        );
                        self.vals[i] = val;
                    }
                }
                CompiledStep::ResetState(sid) => {
                    let st = &self.state_vars[sid];
                    let init_val = self.vals[st.init_idx];
                    if !self.state_initialized[sid] {
                        self.state_slots[sid] = init_val;
                        self.state_initialized[sid] = true;
                    } else {
                        let should_reset = match st.reset {
                            CompiledReset::None => false,
                            CompiledReset::Enter => entered,
                            CompiledReset::Exit => exited,
                            CompiledReset::Expr(cid) => bool_at(&self.vals, cid) == TriBool::True,
                        };
                        if should_reset {
                            self.state_slots[sid] = init_val;
                        }
                    }
                }
                CompiledStep::ExecBlock(bidx) => {
                    let blk = &self.state_blocks[bidx];
                    let active = match blk.trigger {
                        CompiledTrigger::OnEnter => entered,
                        CompiledTrigger::OnExit => exited,
                        CompiledTrigger::OnExpr(cid) | CompiledTrigger::WhileExpr(cid) => {
                            bool_at(&self.vals, cid) == TriBool::True
                        }
                        CompiledTrigger::WhileLong => pos.side == PositionSide::Long,
                        CompiledTrigger::WhileShort => pos.side == PositionSide::Short,
                        CompiledTrigger::WhileFlat => pos.side == PositionSide::Flat,
                    };
                    if active {
                        let eval_input = EvalInput {
                            bar: &bar,
                            pos: &pos,
                            symbol_states: &self.symbol_states,
                            state_slots: &self.state_slots,
                        };
                        for &nidx in &blk.nodes {
                            if self.node_is_base[nidx] {
                                let val = eval_compiled(
                                    self.ops[nidx],
                                    &mut self.states[nidx],
                                    &self.vals,
                                    &self.strings,
                                    &eval_input,
                                );
                                self.vals[nidx] = val;
                            }
                        }
                        for &(sid, vidx) in &blk.updates {
                            self.state_slots[sid] = self.vals[vidx];
                        }
                    }
                }
            }
        }

        self.prev_side = pos.side;
        self.last_timestamp = Some(bar.timestamp);
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

struct EvalInput<'a> {
    bar: &'a Bar,
    pos: &'a PositionView,
    symbol_states: &'a [SymbolState],
    state_slots: &'a [NodeVal],
}

fn eval_compiled(
    op: CompiledOp,
    state: &mut NodeState,
    vals: &[NodeVal],
    strings: &[String],
    input: &EvalInput<'_>,
) -> NodeVal {
    match op {
        CompiledOp::Series(field) => {
            let v = match field {
                SeriesField::Open => input.bar.open,
                SeriesField::High => input.bar.high,
                SeriesField::Low => input.bar.low,
                SeriesField::Close => input.bar.close,
                SeriesField::Volume => input.bar.volume,
            };
            NodeVal::Num(clean(v))
        }
        CompiledOp::Position(field) => match field {
            PositionField::Side => NodeVal::Str(input.pos.side.str_idx()),
            PositionField::EntryPrice => NodeVal::Num(clean(input.pos.entry_price)),
            PositionField::BarsHeld => NodeVal::Num(input.pos.bars_held as f64),
            PositionField::Pnl => NodeVal::Num(clean(input.pos.pnl)),
        },
        CompiledOp::SymbolSeries { sym_idx, field } => {
            let sym = &input.symbol_states[sym_idx];
            match field {
                SymbolField::Open => NodeVal::Num(sym.open),
                SymbolField::High => NodeVal::Num(sym.high),
                SymbolField::Low => NodeVal::Num(sym.low),
                SymbolField::Close => NodeVal::Num(sym.close),
                SymbolField::Volume => NodeVal::Num(sym.volume),
                SymbolField::Stale => NodeVal::Bool(sym.stale),
            }
        }
        CompiledOp::StateRead(sid) => input.state_slots[sid],
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
                _ => NodeVal::Num(f64::NAN),
            }
        }
        CompiledOp::Lag(src) => {
            let cur = vals[src];
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

fn input_idx(node: &IrNode, pos: usize, cur_idx: usize) -> Result<usize, ExecError> {
    let id = node
        .inputs
        .get(pos)
        .ok_or_else(|| ExecError::InvalidNode(format!("missing input {pos} on {}", node.id)))?;
    let idx = id.0 as usize;
    if idx >= cur_idx {
        return Err(ExecError::InvalidNode(format!(
            "non-topological input {id} on {}",
            node.id
        )));
    }
    Ok(idx)
}

fn is_numeric(ty: IrTy) -> bool {
    matches!(ty, IrTy::Num | IrTy::Dec)
}

fn same_value_kind(a: IrTy, b: IrTy) -> bool {
    (is_numeric(a) && is_numeric(b)) || a == b
}

fn typed_input(
    node: &IrNode,
    pos: usize,
    nodes: &[IrNode],
    cur_idx: usize,
    expected: IrTy,
) -> Result<usize, ExecError> {
    let idx = input_idx(node, pos, cur_idx)?;
    if !same_value_kind(nodes[idx].ty, expected) {
        return Err(ExecError::InvalidNode(format!(
            "input {pos} on {} expected {expected:?}, got {:?}",
            node.id, nodes[idx].ty
        )));
    }
    Ok(idx)
}

fn expect_ty(node: &IrNode, expected: IrTy) -> Result<(), ExecError> {
    if !same_value_kind(node.ty, expected) {
        return Err(ExecError::InvalidNode(format!(
            "node {} expected output type {expected:?}, got {:?}",
            node.id, node.ty
        )));
    }
    Ok(())
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
    cur_idx: usize,
    nodes: &[IrNode],
    symbol_count: usize,
    states: &[IrStateVar],
    strings: &mut Vec<String>,
) -> Result<(CompiledOp, NodeState, NodeVal), ExecError> {
    let init = init_val_for_ty(node.ty);
    match &node.op {
        IrOp::Series(field) => {
            expect_ty(node, IrTy::Num)?;
            Ok((CompiledOp::Series(*field), NodeState::Stateless, init))
        }
        IrOp::Position(field) => {
            let expected = match field {
                PositionField::Side => IrTy::Str,
                _ => IrTy::Num,
            };
            expect_ty(node, expected)?;
            Ok((CompiledOp::Position(*field), NodeState::Stateless, init))
        }
        IrOp::SymbolSeries { sym_idx, field } => {
            let sidx = *sym_idx as usize;
            if sidx >= symbol_count {
                return Err(ExecError::InvalidNode(format!(
                    "symbol index {sidx} out of bounds on {}",
                    node.id
                )));
            }
            let expected = match field {
                SymbolField::Stale => IrTy::Bool,
                _ => IrTy::Num,
            };
            expect_ty(node, expected)?;
            Ok((
                CompiledOp::SymbolSeries {
                    sym_idx: sidx,
                    field: *field,
                },
                NodeState::Stateless,
                init,
            ))
        }
        IrOp::StateRead(sid) => {
            let sidx = sid.0 as usize;
            let st = states.get(sidx).ok_or_else(|| {
                ExecError::InvalidNode(format!("state index {sidx} out of bounds on {}", node.id))
            })?;
            expect_ty(node, st.ty)?;
            Ok((CompiledOp::StateRead(sidx), NodeState::Stateless, init))
        }
        IrOp::Const(c) => {
            let (expected_ty, v) = match c {
                IrConst::Dec { value, pct } => {
                    let mut f = value.to_f64().unwrap_or(f64::NAN);
                    if *pct {
                        f /= 100.0;
                    }
                    (IrTy::Num, NodeVal::Num(clean(f)))
                }
                IrConst::Int(i) => (IrTy::Num, NodeVal::Num(*i as f64)),
                IrConst::Bool(b) => (
                    IrTy::Bool,
                    NodeVal::Bool(if *b { TriBool::True } else { TriBool::False }),
                ),
                IrConst::Tf(tf) => (IrTy::Tf, NodeVal::Tf(*tf)),
                IrConst::Str(s) => {
                    let sid = strings.len() as u32;
                    strings.push(s.clone());
                    (IrTy::Str, NodeVal::Str(sid))
                }
            };
            expect_ty(node, expected_ty)?;
            Ok((CompiledOp::Const(v), NodeState::Stateless, v))
        }
        IrOp::Unary(op) => {
            expect_ty(node, IrTy::Num)?;
            let x = typed_input(node, 0, nodes, cur_idx, IrTy::Num)?;
            let cop = match op {
                UnOp::Neg => CompiledOp::Neg(x),
                UnOp::Pos => CompiledOp::Pos(x),
            };
            Ok((cop, NodeState::Stateless, init))
        }
        IrOp::Binary(op) => {
            expect_ty(node, IrTy::Num)?;
            let l = typed_input(node, 0, nodes, cur_idx, IrTy::Num)?;
            let r = typed_input(node, 1, nodes, cur_idx, IrTy::Num)?;
            Ok((
                CompiledOp::Bin { op: *op, l, r },
                NodeState::Stateless,
                init,
            ))
        }
        IrOp::Compare(op) => {
            expect_ty(node, IrTy::Bool)?;
            let l = input_idx(node, 0, cur_idx)?;
            let r = input_idx(node, 1, cur_idx)?;
            if !same_value_kind(nodes[l].ty, nodes[r].ty) {
                return Err(ExecError::InvalidNode(format!(
                    "compare operand type mismatch on {}: {:?} vs {:?}",
                    node.id, nodes[l].ty, nodes[r].ty
                )));
            }
            if nodes[l].ty == IrTy::Str && !matches!(op, CmpOp::Eq | CmpOp::NotEq) {
                return Err(ExecError::InvalidNode(
                    "unsupported string comparison operator".into(),
                ));
            }
            Ok((
                CompiledOp::Cmp { op: *op, l, r },
                NodeState::Stateless,
                init,
            ))
        }
        IrOp::Not => {
            expect_ty(node, IrTy::Bool)?;
            let x = typed_input(node, 0, nodes, cur_idx, IrTy::Bool)?;
            Ok((CompiledOp::Not(x), NodeState::Stateless, init))
        }
        IrOp::And => {
            expect_ty(node, IrTy::Bool)?;
            let l = typed_input(node, 0, nodes, cur_idx, IrTy::Bool)?;
            let r = typed_input(node, 1, nodes, cur_idx, IrTy::Bool)?;
            Ok((CompiledOp::And(l, r), NodeState::Stateless, init))
        }
        IrOp::Or => {
            expect_ty(node, IrTy::Bool)?;
            let l = typed_input(node, 0, nodes, cur_idx, IrTy::Bool)?;
            let r = typed_input(node, 1, nodes, cur_idx, IrTy::Bool)?;
            Ok((CompiledOp::Or(l, r), NodeState::Stateless, init))
        }
        IrOp::IfElse => {
            let c = typed_input(node, 0, nodes, cur_idx, IrTy::Bool)?;
            let t = input_idx(node, 1, cur_idx)?;
            let e = input_idx(node, 2, cur_idx)?;
            if !same_value_kind(nodes[t].ty, nodes[e].ty) || !same_value_kind(node.ty, nodes[t].ty)
            {
                return Err(ExecError::InvalidNode(format!(
                    "if_else branch/output type mismatch on {}",
                    node.id
                )));
            }
            if matches!(nodes[t].ty, IrTy::Tf | IrTy::Str) {
                return Err(ExecError::InvalidNode(
                    "if_else is not supported on Tf or Str series".into(),
                ));
            }
            Ok((CompiledOp::IfElse { c, t, e }, NodeState::Stateless, init))
        }
        IrOp::Lag(k) => {
            let src = input_idx(node, 0, cur_idx)?;
            if matches!(nodes[src].ty, IrTy::Tf | IrTy::Str) {
                return Err(ExecError::InvalidNode(
                    "lag is not supported on Tf or Str series".into(),
                ));
            }
            if !same_value_kind(node.ty, nodes[src].ty) {
                return Err(ExecError::InvalidNode(format!(
                    "lag output type {:?} mismatches input {:?}",
                    node.ty, nodes[src].ty
                )));
            }
            let lag = usize::try_from(*k)
                .ok()
                .filter(|&lag| lag <= MAX_BUFFER_LEN)
                .ok_or_else(|| {
                    ExecError::InvalidNode(format!(
                        "lag {k} exceeds maximum buffer length {MAX_BUFFER_LEN} on {}",
                        node.id
                    ))
                })?;
            Ok((
                CompiledOp::Lag(src),
                NodeState::Lag(LagRing::new(lag, init)),
                init,
            ))
        }
        IrOp::Builtin(name) => compile_builtin(name, node, cur_idx, nodes, init),
    }
}

fn compile_builtin(
    name: &str,
    node: &IrNode,
    cur_idx: usize,
    nodes: &[IrNode],
    init: NodeVal,
) -> Result<(CompiledOp, NodeState, NodeVal), ExecError> {
    let num = |pos| typed_input(node, pos, nodes, cur_idx, IrTy::Num);
    let bool_in = |pos| typed_input(node, pos, nodes, cur_idx, IrTy::Bool);
    match name {
        "abs" => {
            expect_ty(node, IrTy::Num)?;
            Ok((CompiledOp::Abs(num(0)?), NodeState::Stateless, init))
        }
        "sqrt" => {
            expect_ty(node, IrTy::Num)?;
            Ok((CompiledOp::Sqrt(num(0)?), NodeState::Stateless, init))
        }
        "log" => {
            expect_ty(node, IrTy::Num)?;
            Ok((CompiledOp::Log(num(0)?), NodeState::Stateless, init))
        }
        "exp" => {
            expect_ty(node, IrTy::Num)?;
            Ok((CompiledOp::Exp(num(0)?), NodeState::Stateless, init))
        }
        "min" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Min(num(0)?, num(1)?),
                NodeState::Stateless,
                init,
            ))
        }
        "max" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Max(num(0)?, num(1)?),
                NodeState::Stateless,
                init,
            ))
        }
        "where" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Where {
                    c: bool_in(0)?,
                    a: num(1)?,
                    b: num(2)?,
                },
                NodeState::Stateless,
                init,
            ))
        }
        "clamp" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Clamp {
                    x: num(0)?,
                    lo: num(1)?,
                    hi: num(2)?,
                },
                NodeState::Stateless,
                init,
            ))
        }
        "mod" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Mod(num(0)?, num(1)?),
                NodeState::Stateless,
                init,
            ))
        }
        "diff" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 0)?;
            Ok((
                CompiledOp::Diff(x),
                NodeState::Diff(LagRing::new(n, f64::NAN)),
                init,
            ))
        }
        "pct_change" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::PctChange(x),
                NodeState::PctChange(LagRing::new(n, f64::NAN)),
                init,
            ))
        }
        "cumsum" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Cumsum(num(0)?),
                NodeState::Cumsum(CumsumState::new()),
                init,
            ))
        }
        "sum" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Sum(x),
                NodeState::Sum(TwoStackWindow::new(n)),
                init,
            ))
        }
        "mean" | "sma" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Mean(x),
                NodeState::Sum(TwoStackWindow::new(n)),
                init,
            ))
        }
        "var" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Var(x),
                NodeState::Stats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "std" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Std(x),
                NodeState::Stats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "bollinger" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 2)?;
            let mult = const_f64(node, 1)?;
            Ok((
                CompiledOp::Bollinger { x, mult },
                NodeState::Stats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "ema" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            let alpha = 2.0 / ((n as f64) + 1.0);
            Ok((
                CompiledOp::Ema(x),
                NodeState::Ewm(EwmState::new(alpha, n)),
                init,
            ))
        }
        "rma" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            let alpha = 1.0 / (n as f64);
            Ok((
                CompiledOp::Ema(x),
                NodeState::Ewm(EwmState::new(alpha, n)),
                init,
            ))
        }
        "wma" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
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
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Highest(x),
                NodeState::Extrema(TwoStackWindow::new(n)),
                init,
            ))
        }
        "lowest" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Lowest(x),
                NodeState::Extrema(TwoStackWindow::new(n)),
                init,
            ))
        }
        "rsi" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((CompiledOp::Rsi(x), NodeState::Rsi(RsiState::new(n)), init))
        }
        "tr" => {
            expect_ty(node, IrTy::Num)?;
            Ok((
                CompiledOp::Tr {
                    h: num(0)?,
                    l: num(1)?,
                    c: num(2)?,
                },
                NodeState::Tr(TrState::new()),
                init,
            ))
        }
        "atr" => {
            expect_ty(node, IrTy::Num)?;
            let h = num(0)?;
            let l = num(1)?;
            let c = num(2)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::Atr { h, l, c },
                NodeState::Atr(AtrState::new(n)),
                init,
            ))
        }
        "macd" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
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
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let y = num(1)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Cov(x, y),
                NodeState::PairStats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "corr" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let y = num(1)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::Corr(x, y),
                NodeState::PairStats(TwoStackWindow::new(n)),
                init,
            ))
        }
        "linreg_slope" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::LinregSlope(x),
                NodeState::Linreg(TwoStackWindow::new(n)),
                init,
            ))
        }
        "linreg_intercept" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 2)?;
            Ok((
                CompiledOp::LinregIntercept(x),
                NodeState::Linreg(TwoStackWindow::new(n)),
                init,
            ))
        }
        "percentile_rank" => {
            expect_ty(node, IrTy::Num)?;
            let x = num(0)?;
            let n = const_usize(node, 0, 1)?;
            Ok((
                CompiledOp::PercentileRank(x),
                NodeState::Percentile(PercentileWindow::new(n)),
                init,
            ))
        }
        "crossover" => {
            expect_ty(node, IrTy::Bool)?;
            let a = num(0)?;
            let b = num(1)?;
            Ok((
                CompiledOp::Crossover(a, b),
                NodeState::Cross(CrossState::new(true)),
                init,
            ))
        }
        "crossunder" => {
            expect_ty(node, IrTy::Bool)?;
            let a = num(0)?;
            let b = num(1)?;
            Ok((
                CompiledOp::Crossunder(a, b),
                NodeState::Cross(CrossState::new(false)),
                init,
            ))
        }
        "since" => {
            expect_ty(node, IrTy::Num)?;
            let c = bool_in(0)?;
            Ok((
                CompiledOp::Since(c),
                NodeState::Since(SinceState::new()),
                init,
            ))
        }
        _ => Err(ExecError::UnsupportedOp(name.to_string())),
    }
}

struct Fnv1a64(u64);

impl Fnv1a64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0100_0000_01b3;

    fn new() -> Self {
        Self(Self::OFFSET_BASIS)
    }
}

impl Hasher for Fnv1a64 {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

fn fingerprint_ir(ir: &IrGraph) -> u64 {
    let mut h = Fnv1a64::new();
    ir.base_clock.hash(&mut h);
    ir.entry.hash(&mut h);
    ir.exit.hash(&mut h);
    ir.symbols.hash(&mut h);
    ir.states.len().hash(&mut h);
    for st in &ir.states {
        st.id.hash(&mut h);
        st.name.hash(&mut h);
        st.ty.hash(&mut h);
        st.init.hash(&mut h);
        st.reset.hash(&mut h);
    }
    ir.state_blocks.len().hash(&mut h);
    for blk in &ir.state_blocks {
        blk.trigger.hash(&mut h);
        blk.nodes.hash(&mut h);
        blk.updates.hash(&mut h);
    }
    ir.schedule.hash(&mut h);
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
