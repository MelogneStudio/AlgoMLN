use std::fmt;

use mln_check::Tf;
use mln_ir::{lower_default, IrClock, IrConst, IrGraph, IrNode, IrOp, NodeId, SeriesField};
use mln_lang::{parse, BinOp, CmpOp, UnOp};
use mln_std::eval::{
    clean, op_abs, op_atr, op_bollinger, op_clamp, op_corr, op_cov, op_crossover, op_crossunder,
    op_cumsum, op_diff, op_ema, op_exp, op_highest, op_linreg_intercept, op_linreg_slope, op_log,
    op_lowest, op_macd, op_max, op_mean, op_min, op_mod, op_pct_change, op_percentile_rank, op_rma,
    op_rsi, op_since, op_sma, op_sqrt, op_std, op_sum, op_tr, op_var, op_where, op_wma, safe_div,
};
use mln_std::TriBool;
use rust_decimal::prelude::ToPrimitive;

/// Bar series inputs supplied to the oracle interpreter.
#[derive(Debug, Clone)]
pub struct SeriesInputs {
    pub open: Vec<f64>,
    pub high: Vec<f64>,
    pub low: Vec<f64>,
    pub close: Vec<f64>,
    pub volume: Vec<f64>,
}

impl SeriesInputs {
    /// Returns the number of bars if all OHLCV columns have equal length.
    pub fn len(&self) -> Result<usize, ExecError> {
        let n = self.close.len();
        if self.open.len() != n
            || self.high.len() != n
            || self.low.len() != n
            || self.volume.len() != n
        {
            return Err(ExecError::LengthMismatch);
        }
        Ok(n)
    }

    /// Returns `true` when there are zero bars.
    pub fn is_empty(&self) -> Result<bool, ExecError> {
        Ok(self.len()? == 0)
    }
}

/// Evaluated series value for an IR node.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueSeries {
    Num(Vec<f64>),
    Bool(Vec<TriBool>),
    Tf(Vec<Tf>),
    Str(Vec<String>),
}

/// Errors produced during oracle evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    LengthMismatch,
    ParseOrCheckFailed(String),
    UnsupportedOp(String),
    InvalidNode(String),
}

impl fmt::Display for ExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecError::LengthMismatch => f.write_str("input OHLCV series have unequal lengths"),
            ExecError::ParseOrCheckFailed(msg) => write!(f, "front-end failed: {msg}"),
            ExecError::UnsupportedOp(op) => write!(f, "unsupported operator `{op}`"),
            ExecError::InvalidNode(msg) => write!(f, "invalid IR node: {msg}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// Complete evaluation output from [`eval_oracle`].
#[derive(Debug, Clone)]
pub struct OracleOutput {
    pub values: Vec<ValueSeries>,
    pub entry: Vec<TriBool>,
    pub exit: Vec<TriBool>,
    pub entry_signals: Vec<u8>,
    pub exit_signals: Vec<u8>,
}

impl OracleOutput {
    /// Returns the numeric series for `id`, if `id` evaluates to `Num`/`Dec`.
    pub fn node_num(&self, id: NodeId) -> Option<&[f64]> {
        match self.values.get(id.0 as usize) {
            Some(ValueSeries::Num(v)) => Some(v),
            _ => None,
        }
    }

    /// Returns the boolean series for `id`, if `id` evaluates to `Bool`.
    pub fn node_bool(&self, id: NodeId) -> Option<&[TriBool]> {
        match self.values.get(id.0 as usize) {
            Some(ValueSeries::Bool(v)) => Some(v),
            _ => None,
        }
    }
}

/// Convenience helper: parses, checks, lowers, and evaluates `src` with the oracle.
pub fn eval_source_oracle(
    src: &str,
    inputs: &SeriesInputs,
) -> Result<(IrGraph, OracleOutput), ExecError> {
    let parsed = parse(src);
    if !parsed.diags.is_empty() {
        return Err(ExecError::ParseOrCheckFailed(format!(
            "{} parse error(s)",
            parsed.diags.len()
        )));
    }
    let graph =
        lower_default(&parsed.program).map_err(|e| ExecError::ParseOrCheckFailed(e.to_string()))?;
    let out = eval_oracle(&graph, inputs)?;
    Ok((graph, out))
}

/// Evaluates an [`IrGraph`] over whole-array [`SeriesInputs`].
pub fn eval_oracle(graph: &IrGraph, inputs: &SeriesInputs) -> Result<OracleOutput, ExecError> {
    let len = inputs.len()?;
    let mut values: Vec<ValueSeries> = Vec::with_capacity(graph.nodes.len());

    for node in &graph.nodes {
        if let IrClock::Tf(tf) = node.clock {
            return Err(ExecError::UnsupportedOp(format!(
                "non-base clock `{tf}` is not supported in Phase 3 oracle"
            )));
        }
        let val = eval_node(node, &values, inputs, len)?;
        values.push(val);
    }

    let entry = match values.get(graph.entry.0 as usize) {
        Some(ValueSeries::Bool(v)) => v.clone(),
        _ => return Err(ExecError::InvalidNode("entry is not Bool".into())),
    };
    let exit = match values.get(graph.exit.0 as usize) {
        Some(ValueSeries::Bool(v)) => v.clone(),
        _ => return Err(ExecError::InvalidNode("exit is not Bool".into())),
    };
    let entry_signals = entry.iter().map(|b| b.to_signal()).collect();
    let exit_signals = exit.iter().map(|b| b.to_signal()).collect();

    Ok(OracleOutput {
        values,
        entry,
        exit,
        entry_signals,
        exit_signals,
    })
}

fn input(node: &IrNode, idx: usize) -> Result<NodeId, ExecError> {
    node.inputs
        .get(idx)
        .copied()
        .ok_or_else(|| ExecError::InvalidNode(format!("missing input {idx} on {}", node.id)))
}

fn get_val(values: &[ValueSeries], id: NodeId) -> Result<&ValueSeries, ExecError> {
    values
        .get(id.0 as usize)
        .ok_or_else(|| ExecError::InvalidNode(format!("missing value at {id}")))
}

fn get_num(values: &[ValueSeries], id: NodeId) -> Result<&[f64], ExecError> {
    match get_val(values, id)? {
        ValueSeries::Num(v) => Ok(v),
        _ => Err(ExecError::InvalidNode(format!("expected Num at {id}"))),
    }
}

fn get_bool(values: &[ValueSeries], id: NodeId) -> Result<&[TriBool], ExecError> {
    match get_val(values, id)? {
        ValueSeries::Bool(v) => Ok(v),
        _ => Err(ExecError::InvalidNode(format!("expected Bool at {id}"))),
    }
}

fn const_usize(node: &IrNode, idx: usize) -> Result<usize, ExecError> {
    match node.const_args.get(idx) {
        Some(IrConst::Int(v)) => usize::try_from(*v).map_err(|_| {
            ExecError::InvalidNode(format!(
                "int const arg {idx} out of usize range on {}",
                node.id
            ))
        }),
        _ => Err(ExecError::InvalidNode(format!(
            "expected non-negative int const arg {idx} on {}",
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

fn eval_node(
    node: &IrNode,
    values: &[ValueSeries],
    inputs: &SeriesInputs,
    len: usize,
) -> Result<ValueSeries, ExecError> {
    match &node.op {
        IrOp::Series(field) => {
            let src = match field {
                SeriesField::Open => &inputs.open,
                SeriesField::High => &inputs.high,
                SeriesField::Low => &inputs.low,
                SeriesField::Close => &inputs.close,
                SeriesField::Volume => &inputs.volume,
            };
            Ok(ValueSeries::Num(src.iter().map(|&v| clean(v)).collect()))
        }
        IrOp::Const(c) => match c {
            IrConst::Dec { value, pct } => {
                let mut v = value.to_f64().unwrap_or(f64::NAN);
                if *pct {
                    v /= 100.0;
                }
                Ok(ValueSeries::Num(vec![clean(v); len]))
            }
            IrConst::Int(i) => Ok(ValueSeries::Num(vec![*i as f64; len])),
            IrConst::Bool(b) => Ok(ValueSeries::Bool(vec![
                if *b {
                    TriBool::True
                } else {
                    TriBool::False
                };
                len
            ])),
            IrConst::Tf(tf) => Ok(ValueSeries::Tf(vec![*tf; len])),
            IrConst::Str(s) => Ok(ValueSeries::Str(vec![s.clone(); len])),
        },
        IrOp::Unary(op) => {
            let x = get_num(values, input(node, 0)?)?;
            let out = x
                .iter()
                .map(|&v| {
                    if !v.is_finite() {
                        f64::NAN
                    } else {
                        match op {
                            UnOp::Neg => clean(-v),
                            UnOp::Pos => v,
                        }
                    }
                })
                .collect();
            Ok(ValueSeries::Num(out))
        }
        IrOp::Binary(op) => {
            let l = get_num(values, input(node, 0)?)?;
            let r = get_num(values, input(node, 1)?)?;
            let out = l
                .iter()
                .zip(r.iter())
                .map(|(&a, &b)| {
                    if !a.is_finite() || !b.is_finite() {
                        return f64::NAN;
                    }
                    match op {
                        BinOp::Add => clean(a + b),
                        BinOp::Sub => clean(a - b),
                        BinOp::Mul => clean(a * b),
                        BinOp::Div => safe_div(a, b),
                        BinOp::Pow => clean(a.powf(b)),
                    }
                })
                .collect();
            Ok(ValueSeries::Num(out))
        }
        IrOp::Compare(op) => {
            let lhs = get_val(values, input(node, 0)?)?;
            let rhs = get_val(values, input(node, 1)?)?;
            let out = match (lhs, rhs) {
                (ValueSeries::Num(l), ValueSeries::Num(r)) => l
                    .iter()
                    .zip(r.iter())
                    .map(|(&a, &b)| {
                        if !a.is_finite() || !b.is_finite() {
                            TriBool::Unknown
                        } else {
                            let res = match op {
                                CmpOp::Eq => a == b,
                                CmpOp::NotEq => a != b,
                                CmpOp::Lt => a < b,
                                CmpOp::LtEq => a <= b,
                                CmpOp::Gt => a > b,
                                CmpOp::GtEq => a >= b,
                            };
                            if res {
                                TriBool::True
                            } else {
                                TriBool::False
                            }
                        }
                    })
                    .collect(),
                (ValueSeries::Bool(l), ValueSeries::Bool(r)) => l
                    .iter()
                    .zip(r.iter())
                    .map(|(&a, &b)| match (a, b) {
                        (TriBool::Unknown, _) | (_, TriBool::Unknown) => TriBool::Unknown,
                        _ => {
                            let res = match op {
                                CmpOp::Eq => a == b,
                                CmpOp::NotEq => a != b,
                                _ => return TriBool::Unknown,
                            };
                            if res {
                                TriBool::True
                            } else {
                                TriBool::False
                            }
                        }
                    })
                    .collect(),
                (ValueSeries::Tf(l), ValueSeries::Tf(r)) => l
                    .iter()
                    .zip(r.iter())
                    .map(|(a, b)| {
                        let res = match op {
                            CmpOp::Eq => a.minutes() == b.minutes(),
                            CmpOp::NotEq => a.minutes() != b.minutes(),
                            CmpOp::Lt => a.minutes() < b.minutes(),
                            CmpOp::LtEq => a.minutes() <= b.minutes(),
                            CmpOp::Gt => a.minutes() > b.minutes(),
                            CmpOp::GtEq => a.minutes() >= b.minutes(),
                        };
                        if res {
                            TriBool::True
                        } else {
                            TriBool::False
                        }
                    })
                    .collect(),
                (ValueSeries::Str(l), ValueSeries::Str(r)) => {
                    if !matches!(op, CmpOp::Eq | CmpOp::NotEq) {
                        return Err(ExecError::InvalidNode(
                            "unsupported string comparison operator".into(),
                        ));
                    }
                    l.iter()
                        .zip(r.iter())
                        .map(|(a, b)| {
                            let res = match op {
                                CmpOp::Eq => a == b,
                                CmpOp::NotEq => a != b,
                                _ => unreachable!(),
                            };
                            if res {
                                TriBool::True
                            } else {
                                TriBool::False
                            }
                        })
                        .collect()
                }
                _ => {
                    return Err(ExecError::InvalidNode(
                        "compare operand type mismatch".into(),
                    ))
                }
            };
            Ok(ValueSeries::Bool(out))
        }
        IrOp::Not => {
            let x = get_bool(values, input(node, 0)?)?;
            Ok(ValueSeries::Bool(x.iter().map(|b| b.not()).collect()))
        }
        IrOp::And => {
            let l = get_bool(values, input(node, 0)?)?;
            let r = get_bool(values, input(node, 1)?)?;
            Ok(ValueSeries::Bool(
                l.iter().zip(r.iter()).map(|(&a, &b)| a.and(b)).collect(),
            ))
        }
        IrOp::Or => {
            let l = get_bool(values, input(node, 0)?)?;
            let r = get_bool(values, input(node, 1)?)?;
            Ok(ValueSeries::Bool(
                l.iter().zip(r.iter()).map(|(&a, &b)| a.or(b)).collect(),
            ))
        }
        IrOp::IfElse => {
            let c = get_bool(values, input(node, 0)?)?;
            let then_val = get_val(values, input(node, 1)?)?;
            let else_val = get_val(values, input(node, 2)?)?;
            match (then_val, else_val) {
                (ValueSeries::Num(t), ValueSeries::Num(e)) => {
                    if c.len() != len || t.len() != len || e.len() != len {
                        return Err(ExecError::InvalidNode("if_else length mismatch".into()));
                    }
                    Ok(ValueSeries::Num(op_where(c, t, e)))
                }
                (ValueSeries::Bool(t), ValueSeries::Bool(e)) => {
                    if c.len() != len || t.len() != len || e.len() != len {
                        return Err(ExecError::InvalidNode("if_else length mismatch".into()));
                    }
                    let out = c
                        .iter()
                        .zip(t.iter())
                        .zip(e.iter())
                        .map(|((&cond, &tv), &ev)| match cond {
                            TriBool::True => tv,
                            TriBool::False => ev,
                            TriBool::Unknown => TriBool::Unknown,
                        })
                        .collect();
                    Ok(ValueSeries::Bool(out))
                }
                (ValueSeries::Tf(t), ValueSeries::Tf(e)) => {
                    if c.len() != len || t.len() != len || e.len() != len {
                        return Err(ExecError::InvalidNode("if_else length mismatch".into()));
                    }
                    let out = c
                        .iter()
                        .zip(t.iter())
                        .zip(e.iter())
                        .map(|((&cond, &tv), &ev)| match cond {
                            TriBool::True => Ok(tv),
                            TriBool::False => Ok(ev),
                            TriBool::Unknown => Err(ExecError::InvalidNode(
                                "if_else on Tf cannot evaluate Unknown condition".into(),
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(ValueSeries::Tf(out))
                }
                (ValueSeries::Str(t), ValueSeries::Str(e)) => {
                    if c.len() != len || t.len() != len || e.len() != len {
                        return Err(ExecError::InvalidNode("if_else length mismatch".into()));
                    }
                    let out = c
                        .iter()
                        .zip(t.iter())
                        .zip(e.iter())
                        .map(|((&cond, tv), ev)| match cond {
                            TriBool::True => Ok(tv.clone()),
                            TriBool::False => Ok(ev.clone()),
                            TriBool::Unknown => Err(ExecError::InvalidNode(
                                "if_else on Str cannot evaluate Unknown condition".into(),
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(ValueSeries::Str(out))
                }
                _ => Err(ExecError::InvalidNode("if_else branch mismatch".into())),
            }
        }
        IrOp::Lag(k) => {
            let k_opt = usize::try_from(*k).ok().filter(|&k| k < len);
            match get_val(values, input(node, 0)?)? {
                ValueSeries::Num(x) => {
                    if x.len() != len {
                        return Err(ExecError::InvalidNode("lag input length mismatch".into()));
                    }
                    let out = match k_opt {
                        None => vec![f64::NAN; len],
                        Some(k) => (0..len)
                            .map(|t| if t < k { f64::NAN } else { clean(x[t - k]) })
                            .collect(),
                    };
                    Ok(ValueSeries::Num(out))
                }
                ValueSeries::Bool(x) => {
                    if x.len() != len {
                        return Err(ExecError::InvalidNode("lag input length mismatch".into()));
                    }
                    let out = match k_opt {
                        None => vec![TriBool::Unknown; len],
                        Some(k) => (0..len)
                            .map(|t| if t < k { TriBool::Unknown } else { x[t - k] })
                            .collect(),
                    };
                    Ok(ValueSeries::Bool(out))
                }
                ValueSeries::Tf(_) | ValueSeries::Str(_) => Err(ExecError::InvalidNode(
                    "lag is not supported on Tf or Str series".into(),
                )),
            }
        }
        IrOp::Builtin(name) => eval_builtin(name, node, values),
    }
}

fn eval_builtin(
    name: &str,
    node: &IrNode,
    values: &[ValueSeries],
) -> Result<ValueSeries, ExecError> {
    match name {
        "abs" => Ok(ValueSeries::Num(op_abs(get_num(values, input(node, 0)?)?))),
        "sqrt" => Ok(ValueSeries::Num(op_sqrt(get_num(values, input(node, 0)?)?))),
        "log" => Ok(ValueSeries::Num(op_log(get_num(values, input(node, 0)?)?))),
        "exp" => Ok(ValueSeries::Num(op_exp(get_num(values, input(node, 0)?)?))),
        "min" => Ok(ValueSeries::Num(op_min(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
        ))),
        "max" => Ok(ValueSeries::Num(op_max(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
        ))),
        "where" => Ok(ValueSeries::Num(op_where(
            get_bool(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
            get_num(values, input(node, 2)?)?,
        ))),
        "clamp" => Ok(ValueSeries::Num(op_clamp(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
            get_num(values, input(node, 2)?)?,
        ))),
        "mod" => Ok(ValueSeries::Num(op_mod(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
        ))),
        "diff" => Ok(ValueSeries::Num(op_diff(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "pct_change" => Ok(ValueSeries::Num(op_pct_change(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "cumsum" => Ok(ValueSeries::Num(op_cumsum(get_num(
            values,
            input(node, 0)?,
        )?))),
        "sum" => Ok(ValueSeries::Num(op_sum(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "mean" => Ok(ValueSeries::Num(op_mean(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "sma" => Ok(ValueSeries::Num(op_sma(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "std" => Ok(ValueSeries::Num(op_std(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "var" => Ok(ValueSeries::Num(op_var(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "ema" => Ok(ValueSeries::Num(op_ema(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "rma" => Ok(ValueSeries::Num(op_rma(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "wma" => Ok(ValueSeries::Num(op_wma(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "highest" => Ok(ValueSeries::Num(op_highest(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "lowest" => Ok(ValueSeries::Num(op_lowest(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "rsi" => Ok(ValueSeries::Num(op_rsi(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "tr" => Ok(ValueSeries::Num(op_tr(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
            get_num(values, input(node, 2)?)?,
        ))),
        "atr" => Ok(ValueSeries::Num(op_atr(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
            get_num(values, input(node, 2)?)?,
            const_usize(node, 0)?,
        ))),
        "macd" => Ok(ValueSeries::Num(op_macd(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
            const_usize(node, 1)?,
            const_usize(node, 2)?,
        ))),
        "bollinger" => Ok(ValueSeries::Num(op_bollinger(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
            const_f64(node, 1)?,
        ))),
        "corr" => Ok(ValueSeries::Num(op_corr(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
            const_usize(node, 0)?,
        ))),
        "cov" => Ok(ValueSeries::Num(op_cov(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
            const_usize(node, 0)?,
        ))),
        "linreg_slope" => Ok(ValueSeries::Num(op_linreg_slope(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "linreg_intercept" => Ok(ValueSeries::Num(op_linreg_intercept(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "percentile_rank" => Ok(ValueSeries::Num(op_percentile_rank(
            get_num(values, input(node, 0)?)?,
            const_usize(node, 0)?,
        ))),
        "crossover" => Ok(ValueSeries::Bool(op_crossover(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
        ))),
        "crossunder" => Ok(ValueSeries::Bool(op_crossunder(
            get_num(values, input(node, 0)?)?,
            get_num(values, input(node, 1)?)?,
        ))),
        "since" => Ok(ValueSeries::Num(op_since(get_bool(
            values,
            input(node, 0)?,
        )?))),
        _ => Err(ExecError::UnsupportedOp(name.to_string())),
    }
}
