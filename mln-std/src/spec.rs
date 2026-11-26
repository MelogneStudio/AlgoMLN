use std::collections::HashMap;

use rust_decimal::Decimal;

/// Expected type/constness of an operator parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamTy {
    /// Numeric series (`Num` or `Dec` constant promoted to series).
    Num,
    /// Three-valued boolean series (`Bool`).
    Bool,
    /// Constant positive integer (`>= 1`, e.g. rolling window `n`).
    ConstPosInt,
    /// Constant non-negative integer (`>= 0`, e.g. lag `k`).
    ConstNonNegInt,
    /// Constant decimal number (`Dec`).
    ConstDec,
    /// Constant string (`Str`).
    ConstStr,
}

/// Default value for an optional operator parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamDefault {
    Int(i64),
    Dec(i64, u32), // mantissa, scale (e.g. 2.0 => (20, 1))
    Bool(bool),
    Str(&'static str),
}

impl ParamDefault {
    /// Converts a numeric default into a `Decimal`, if numeric.
    pub fn as_decimal(self) -> Option<Decimal> {
        match self {
            ParamDefault::Int(v) => Some(Decimal::from(v)),
            ParamDefault::Dec(m, s) => Some(Decimal::new(m, s)),
            ParamDefault::Bool(_) | ParamDefault::Str(_) => None,
        }
    }
}

/// Specification of a single operator parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: &'static str,
    pub ty: ParamTy,
    pub default: Option<ParamDefault>,
}

/// Return type of an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnTy {
    Num,
    Bool,
}

/// Warm-up bar count rule for an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarmupRule {
    /// Fixed number of leading NaN bars.
    Fixed(u64),
    /// `param[idx]` bars (e.g. `rsi(x, n)` has warm-up `n`, `diff(x, n)` has `n`).
    Param(usize),
    /// `param[idx] - 1` bars (e.g. `ema(x, n)`, `sma(x, n)` have warm-up `n - 1`).
    ParamMinusOne(usize),
    /// `param[idx]` bars (used when combining 1-bar diff/tr + `n - 1` smoothing, e.g. `atr`).
    TrPlusParamMinusOne(usize),
    /// `max(slow - 1, slow - 1 + signal - 1)` = `max(param[a], param[b]) + param[c] - 2`.
    Macd {
        fast: usize,
        slow: usize,
        signal: usize,
    },
}

impl WarmupRule {
    /// Computes the operator's intrinsic warm-up bars given resolved integer parameter values.
    pub fn compute(self, int_args: &[Option<u64>]) -> u64 {
        match self {
            WarmupRule::Fixed(k) => k,
            WarmupRule::Param(i) => int_args.get(i).and_then(|v| *v).unwrap_or(0),
            WarmupRule::ParamMinusOne(i) => int_args
                .get(i)
                .and_then(|v| *v)
                .unwrap_or(1)
                .saturating_sub(1),
            WarmupRule::TrPlusParamMinusOne(i) => int_args.get(i).and_then(|v| *v).unwrap_or(1),
            WarmupRule::Macd { fast, slow, signal } => {
                let f = int_args.get(fast).and_then(|v| *v).unwrap_or(12);
                let s = int_args.get(slow).and_then(|v| *v).unwrap_or(26);
                let sig = int_args.get(signal).and_then(|v| *v).unwrap_or(9);
                f.max(s).saturating_sub(1) + sig.saturating_sub(1)
            }
        }
    }
}

/// Incremental per-bar update time complexity class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostClass {
    Constant,
    Logarithmic,
    Linear,
}

/// How an operator handles NaN / unknown inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NanRule {
    /// Any NaN/unknown input produces NaN/unknown output at that bar.
    Propagate,
    /// `where(cond, a, b)`: unknown `cond` gives NaN; otherwise only the selected branch matters.
    Select,
    /// Stateful accumulator resets or skips/carries according to its operator contract.
    Stateful,
}

/// Full metadata specification for a built-in (or plugin) operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpSpec {
    pub name: &'static str,
    pub params: &'static [ParamSpec],
    pub ret: ReturnTy,
    pub warmup: WarmupRule,
    pub state_bytes: usize,
    pub cost: CostClass,
    pub nan_rule: NanRule,
    pub doc_example: &'static str,
    pub fixture_path: &'static str,
}

/// Lookup table of registered operators.
#[derive(Debug, Clone)]
pub struct OpRegistry {
    ops: Vec<OpSpec>,
    by_name: HashMap<&'static str, usize>,
}

impl OpRegistry {
    /// Builds a registry from a slice of [`OpSpec`] entries.
    pub fn from_specs(specs: &[OpSpec]) -> Self {
        let mut ops = Vec::with_capacity(specs.len());
        let mut by_name = HashMap::with_capacity(specs.len());
        for spec in specs {
            let idx = ops.len();
            ops.push(*spec);
            by_name.insert(spec.name, idx);
        }
        Self { ops, by_name }
    }

    /// Looks up an operator by exact name.
    pub fn get(&self, name: &str) -> Option<&OpSpec> {
        self.by_name.get(name).map(|&idx| &self.ops[idx])
    }

    /// Returns all registered operators.
    pub fn all(&self) -> &[OpSpec] {
        &self.ops
    }
}
