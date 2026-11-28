use std::collections::BTreeMap;
use std::fmt;

use mln_lang::{Span, TfUnit, Timeframe};
use rust_decimal::Decimal;

/// A timeframe clock value (`5m`, `1h`, `1d`, `2w`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Tf {
    pub n: u32,
    pub unit: TfUnit,
}

impl Tf {
    /// Creates a new `Tf`.
    pub const fn new(n: u32, unit: TfUnit) -> Self {
        Self { n, unit }
    }

    /// Converts this timeframe to minutes for relative coarseness ordering and bar-ratio estimates.
    pub const fn minutes(self) -> u64 {
        let mult: u64 = match self.unit {
            TfUnit::Minute => 1,
            TfUnit::Hour => 60,
            TfUnit::Day => 1440,
            TfUnit::Week => 10_080,
        };
        (self.n as u64) * mult
    }
}

impl From<Timeframe> for Tf {
    fn from(tf: Timeframe) -> Self {
        Self {
            n: tf.n,
            unit: tf.unit,
        }
    }
}

impl Ord for Tf {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.minutes()
            .cmp(&other.minutes())
            .then_with(|| self.n.cmp(&other.n))
            .then_with(|| (self.unit as u8).cmp(&(other.unit as u8)))
    }
}

impl PartialOrd for Tf {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Tf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.n, self.unit)
    }
}

/// Static type of an expression in `mln-check`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ty {
    Num,
    Bool,
    Dec,
    Tf,
    Str,
    /// Built-in `position` execution context object (`position.side`, etc.).
    Position,
    /// Secondary symbol namespace returned by `symbol("NAME")`.
    Symbol,
    /// Poison type propagated from Phase 1 `Error` nodes or earlier check errors.
    Error,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Num => f.write_str("Num"),
            Ty::Bool => f.write_str("Bool"),
            Ty::Dec => f.write_str("Dec"),
            Ty::Tf => f.write_str("Tf"),
            Ty::Str => f.write_str("Str"),
            Ty::Position => f.write_str("Position"),
            Ty::Symbol => f.write_str("Symbol"),
            Ty::Error => f.write_str("<error>"),
        }
    }
}

/// Clock domain of an expression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Clock {
    Constant,
    Tf(Tf),
}

/// Compile-time constant value obtained from constant folding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstVal {
    Dec(Decimal, bool),
    Bool(bool),
    Tf(Tf),
    Str(String),
}

impl ConstVal {
    /// Extracts a `Decimal` if this constant is numeric (`Dec`).
    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            ConstVal::Dec(d, _) => Some(*d),
            _ => None,
        }
    }

    /// Extracts an exact `i64` if this constant is an exact integer `Dec` (non-percent).
    pub fn as_exact_int(&self) -> Option<i64> {
        match self {
            ConstVal::Dec(d, false) => {
                let norm = d.normalize();
                if norm.scale() == 0 {
                    i64::try_from(norm.mantissa()).ok()
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

/// Configuration for [`crate::check`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckConfig {
    pub base_clock: Tf,
}

impl Default for CheckConfig {
    fn default() -> Self {
        Self {
            base_clock: Tf::new(1, TfUnit::Minute),
        }
    }
}

impl CheckConfig {
    /// Creates a config with the given base clock timeframe.
    pub const fn with_base_clock(n: u32, unit: TfUnit) -> Self {
        Self {
            base_clock: Tf::new(n, unit),
        }
    }
}

/// Checked expression metadata produced for Phase 3 IR lowering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedExpr {
    pub ty: Ty,
    pub clock: Clock,
    pub const_val: Option<ConstVal>,
    pub warmups: BTreeMap<Tf, u64>,
    pub span: Span,
}

/// Checked statement in a program or block body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedStmt {
    Assign {
        name: String,
        expr: CheckedExpr,
        span: Span,
    },
    DefDecl {
        name: String,
        span: Span,
    },
    State {
        name: String,
        init: CheckedExpr,
        reset: Option<CheckedStateReset>,
        span: Span,
    },
    On {
        event: CheckedEvent,
        body: Vec<CheckedStmt>,
        span: Span,
    },
    While {
        guard: CheckedGuard,
        body: Vec<CheckedStmt>,
        span: Span,
    },
    Error(Span),
}

/// Checked reset specification on a `state` variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedStateReset {
    Enter,
    Exit,
    Expr(CheckedExpr),
}

/// Checked event trigger on an `on` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedEvent {
    Enter,
    Exit,
    Expr(CheckedExpr),
}

/// Checked guard condition on a `while` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckedGuard {
    Long,
    Short,
    Flat,
    Expr(CheckedExpr),
}

/// A per-call-site instantiation of a `def` helper (gets its own state instance).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefInstance {
    pub id: usize,
    pub def_name: String,
    pub call_span: Span,
    pub clock: Clock,
    pub arg_types: Vec<Ty>,
    pub ret_ty: Ty,
    pub warmups: BTreeMap<Tf, u64>,
}

/// Warm-up summary for a single required output (`entry` or `exit`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputWarmup {
    pub name: String,
    pub clock: Tf,
    /// Leading NaN bars in the expression's own clock.
    pub own_clock_bars: u64,
    /// Clock contributing the largest base-bar warm-up estimate.
    pub primary_clock: Tf,
    /// Leading NaN bars in `primary_clock`.
    pub primary_clock_bars: u64,
    /// Per-clock warm-up bars across every clock feeding this output.
    pub by_clock: BTreeMap<Tf, u64>,
    /// Conservative estimate in base-clock bars.
    pub base_bars_estimate: u64,
}

/// Program-level warm-up report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmupReport {
    pub base_clock: Tf,
    pub entry: Option<OutputWarmup>,
    pub exit: Option<OutputWarmup>,
    pub max_base_bars_estimate: u64,
}

/// Checked program returned by [`crate::check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedProgram {
    pub base_clock: Tf,
    pub stmts: Vec<CheckedStmt>,
    pub def_instances: Vec<DefInstance>,
    pub warmup: WarmupReport,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn tf_ord_is_consistent_with_eq_for_equivalent_durations() {
        let m60 = Tf::new(60, TfUnit::Minute);
        let h1 = Tf::new(1, TfUnit::Hour);
        assert_ne!(m60, h1);
        assert_eq!(m60.minutes(), h1.minutes());
        assert_ne!(m60.cmp(&h1), Ordering::Equal);
        assert_eq!(m60.cmp(&h1), h1.cmp(&m60).reverse());
        assert_eq!(m60.cmp(&m60), Ordering::Equal);
        assert_eq!(h1.cmp(&h1), Ordering::Equal);

        let mut map = BTreeMap::new();
        map.insert(m60, 1);
        map.insert(h1, 2);
        assert_eq!(map.get(&m60), Some(&1));
        assert_eq!(map.get(&h1), Some(&2));
    }
}
