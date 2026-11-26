use std::collections::HashMap;
use std::fmt;

use mln_check::Tf;
use mln_lang::{BinOp, CmpOp, UnOp};
use rust_decimal::Decimal;

/// Index of a node in an [`IrGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub u32);

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "n{}", self.0)
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

impl IrClock {
    /// Converts a checker [`mln_check::Clock`] into an [`IrClock`] relative to `base_clock`.
    pub fn from_check(clock: mln_check::Clock, base_clock: Tf) -> Self {
        match clock {
            mln_check::Clock::Constant => IrClock::Constant,
            mln_check::Clock::Tf(tf) if tf == base_clock => IrClock::Base,
            mln_check::Clock::Tf(tf) => IrClock::Tf(tf),
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
            entry: NodeId(0),
            exit: NodeId(0),
            bindings: Vec::new(),
            cse: HashMap::new(),
        }
    }

    /// Interns a node in the DAG, returning an existing [`NodeId`] if an identical
    /// `(op, inputs, const_args, clock, ty)` node already exists.
    pub fn intern(
        &mut self,
        op: IrOp,
        inputs: Vec<NodeId>,
        const_args: Vec<IrConst>,
        clock: IrClock,
        ty: IrTy,
    ) -> NodeId {
        let key = CseKey {
            op: op.clone(),
            inputs: inputs.clone(),
            const_args: const_args.clone(),
            clock,
            ty,
        };
        if let Some(&id) = self.cse.get(&key) {
            return id;
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
        id
    }
}
