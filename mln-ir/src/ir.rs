use std::collections::HashMap;
use std::fmt;

use mln_check::Tf;
use mln_lang::{BinOp, CmpOp, TfUnit, UnOp};
use rust_decimal::Decimal;

/// Index of a node in an [`IrGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
    }
}

/// Index of a persistent `state` variable in an [`IrGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub u32);

impl fmt::Display for StateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "s{}", self.0)
    }
}

/// Fields exposed by the pure `position.*` input context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PositionField {
    Side,
    EntryPrice,
    BarsHeld,
    Pnl,
}

impl PositionField {
    pub fn as_str(self) -> &'static str {
        match self {
            PositionField::Side => "side",
            PositionField::EntryPrice => "entry_price",
            PositionField::BarsHeld => "bars_held",
            PositionField::Pnl => "pnl",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "side" => Some(PositionField::Side),
            "entry_price" => Some(PositionField::EntryPrice),
            "bars_held" => Some(PositionField::BarsHeld),
            "pnl" => Some(PositionField::Pnl),
            _ => None,
        }
    }
}

/// Fields exposed by secondary symbol inputs (`symbol("NAME").<field>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolField {
    Open,
    High,
    Low,
    Close,
    Volume,
    Stale,
}

impl SymbolField {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolField::Open => "open",
            SymbolField::High => "high",
            SymbolField::Low => "low",
            SymbolField::Close => "close",
            SymbolField::Volume => "volume",
            SymbolField::Stale => "stale",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "open" => Some(SymbolField::Open),
            "high" => Some(SymbolField::High),
            "low" => Some(SymbolField::Low),
            "close" => Some(SymbolField::Close),
            "volume" => Some(SymbolField::Volume),
            "stale" => Some(SymbolField::Stale),
            _ => None,
        }
    }
}

/// Built-in market data series field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeriesField {
    Open,
    High,
    Low,
    Close,
    Volume,
}

impl SeriesField {
    pub fn as_str(self) -> &'static str {
        match self {
            SeriesField::Open => "open",
            SeriesField::High => "high",
            SeriesField::Low => "low",
            SeriesField::Close => "close",
            SeriesField::Volume => "volume",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "open" => Some(SeriesField::Open),
            "high" => Some(SeriesField::High),
            "low" => Some(SeriesField::Low),
            "close" => Some(SeriesField::Close),
            "volume" => Some(SeriesField::Volume),
            _ => None,
        }
    }
}

/// Value type of an IR node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrTy {
    Num,
    Bool,
    Dec,
    Tf,
    Str,
}

/// Clock domain attached to an IR node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrClock {
    Constant,
    Base,
    Tf(Tf),
}

/// Canonicalizes equivalent timeframe durations (for example `60m` -> `1h`).
pub fn canonical_tf(tf: Tf) -> Tf {
    let mins = tf.minutes();
    if mins == 0 {
        return Tf::new(0, TfUnit::Minute);
    }
    for (mult, unit) in [
        (10_080u64, TfUnit::Week),
        (1_440u64, TfUnit::Day),
        (60u64, TfUnit::Hour),
        (1u64, TfUnit::Minute),
    ] {
        if mins % mult == 0 {
            if let Ok(n) = u32::try_from(mins / mult) {
                return Tf::new(n, unit);
            }
        }
    }
    tf
}

impl IrClock {
    /// Converts a checker [`mln_check::Clock`] into an [`IrClock`] relative to `base_clock`.
    pub fn from_check(clock: mln_check::Clock, base_clock: Tf) -> Self {
        match clock {
            mln_check::Clock::Constant => IrClock::Constant,
            mln_check::Clock::Tf(tf) if tf.minutes() == base_clock.minutes() => IrClock::Base,
            mln_check::Clock::Tf(tf) => IrClock::Tf(canonical_tf(tf)),
        }
    }

    /// Resolves to a concrete [`Tf`] (None for constant).
    pub fn resolve_tf(self, base_clock: Tf) -> Option<Tf> {
        match self {
            IrClock::Constant => None,
            IrClock::Base => Some(base_clock),
            IrClock::Tf(tf) => Some(tf),
        }
    }
}

/// Constant value embedded in an IR node or operator constant argument.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IrConst {
    Dec { value: Decimal, pct: bool },
    Int(i64),
    Bool(bool),
    Tf(Tf),
    Str(String),
}

/// Operation performed by an [`IrNode`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IrOp {
    Series(SeriesField),
    Position(PositionField),
    SymbolSeries { sym_idx: u32, field: SymbolField },
    StateRead(StateId),
    Const(IrConst),
    Unary(UnOp),
    Binary(BinOp),
    Compare(CmpOp),
    Not,
    And,
    Or,
    IfElse,
    Lag(u64),
    Builtin(&'static str),
}

/// Reset condition for a lowered persistent `state` variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrReset {
    None,
    Enter,
    Exit,
    Expr(NodeId),
}

/// Lowered persistent `state` variable metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrStateVar {
    pub id: StateId,
    pub name: String,
    pub ty: IrTy,
    pub init: NodeId,
    pub reset: IrReset,
}

/// Trigger condition for a lowered `on` or `while` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrTrigger {
    OnEnter,
    OnExit,
    OnExpr(NodeId),
    WhileLong,
    WhileShort,
    WhileFlat,
    WhileExpr(NodeId),
}

/// Conditional state mutation block lowered from `on` or `while`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrStateBlock {
    pub trigger: IrTrigger,
    pub nodes: Vec<NodeId>,
    pub updates: Vec<(StateId, NodeId)>,
}

/// Top-level execution schedule step inside an [`IrGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IrExecStep {
    EvalNode(NodeId),
    ResetState(StateId),
    ExecBlock(usize),
}

/// A single node in the stream DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrNode {
    pub id: NodeId,
    pub op: IrOp,
    pub inputs: Vec<NodeId>,
    pub const_args: Vec<IrConst>,
    pub clock: IrClock,
    pub ty: IrTy,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CseKey {
    scope: u64,
    op: IrOp,
    inputs: Vec<NodeId>,
    const_args: Vec<IrConst>,
    clock: IrClock,
    ty: IrTy,
}

/// Lowered stream DAG with deduplicated common subexpressions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrGraph {
    pub base_clock: Tf,
    pub nodes: Vec<IrNode>,
    pub symbols: Vec<String>,
    pub states: Vec<IrStateVar>,
    pub state_blocks: Vec<IrStateBlock>,
    pub schedule: Vec<IrExecStep>,
    pub entry: NodeId,
    pub exit: NodeId,
    /// Named top-level bindings (excluding `def`s) in source order for inspection/testing.
    pub bindings: Vec<(String, NodeId)>,
    cse: HashMap<CseKey, NodeId>,
}

impl IrGraph {
    /// Creates an empty graph for `base_clock`.
    pub fn new(base_clock: Tf) -> Self {
        Self {
            base_clock,
            nodes: Vec::new(),
            symbols: Vec::new(),
            states: Vec::new(),
            state_blocks: Vec::new(),
            schedule: Vec::new(),
            entry: NodeId(0),
            exit: NodeId(0),
            bindings: Vec::new(),
            cse: HashMap::new(),
        }
    }

    /// Interns a top-level node in the DAG, returning an existing [`NodeId`] if an
    /// identical `(op, inputs, const_args, clock, ty)` node already exists at global scope.
    pub fn intern(
        &mut self,
        op: IrOp,
        inputs: Vec<NodeId>,
        const_args: Vec<IrConst>,
        clock: IrClock,
        ty: IrTy,
    ) -> NodeId {
        let (id, created) = self.intern_with_scope(0, op, inputs, const_args, clock, ty);
        if created {
            self.schedule.push(IrExecStep::EvalNode(id));
        }
        id
    }

    /// Interns a node under an explicit CSE `scope` tag without automatically scheduling it
    /// at the top level. Returns `(id, newly_created)`.
    pub fn intern_scoped(
        &mut self,
        scope: u64,
        op: IrOp,
        inputs: Vec<NodeId>,
        const_args: Vec<IrConst>,
        clock: IrClock,
        ty: IrTy,
    ) -> (NodeId, bool) {
        self.intern_with_scope(scope, op, inputs, const_args, clock, ty)
    }

    fn intern_with_scope(
        &mut self,
        scope: u64,
        op: IrOp,
        inputs: Vec<NodeId>,
        const_args: Vec<IrConst>,
        clock: IrClock,
        ty: IrTy,
    ) -> (NodeId, bool) {
        let key = CseKey {
            scope,
            op: op.clone(),
            inputs: inputs.clone(),
            const_args: const_args.clone(),
            clock,
            ty,
        };
        if let Some(&id) = self.cse.get(&key) {
            return (id, false);
        }
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(IrNode {
            id,
            op,
            inputs,
            const_args,
            clock,
            ty,
        });
        self.cse.insert(key, id);
        (id, true)
    }

    /// Interns a secondary symbol name and returns its dense index.
    pub fn intern_symbol(&mut self, sym: &str) -> u32 {
        if let Some(idx) = self.symbols.iter().position(|s| s == sym) {
            return idx as u32;
        }
        let idx = self.symbols.len() as u32;
        self.symbols.push(sym.to_string());
        idx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ir_clock_canonicalizes_equivalent_durations() {
        let m60 = Tf::new(60, TfUnit::Minute);
        let h1 = Tf::new(1, TfUnit::Hour);
        let m1 = Tf::new(1, TfUnit::Minute);

        assert_eq!(
            IrClock::from_check(mln_check::Clock::Tf(h1), m60),
            IrClock::Base
        );
        assert_eq!(
            IrClock::from_check(mln_check::Clock::Tf(m60), m1),
            IrClock::Tf(h1)
        );
    }
}
