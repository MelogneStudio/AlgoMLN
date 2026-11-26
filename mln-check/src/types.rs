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

/// Checked statement in a program or `def` instance.
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
    Error(Span),
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
    /// Leading NaN bars in the expression's own clock (or dominant non-zero clock).
    pub own_clock_bars: u64,
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
