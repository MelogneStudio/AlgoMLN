use crate::spec::{
    CostClass, NanRule, OpRegistry, OpSpec, ParamDefault, ParamSpec, ParamTy, ReturnTy, WarmupRule,
};

const P_X: &[ParamSpec] = &[ParamSpec {
    name: "x",
    ty: ParamTy::Num,
    default: None,
}];

const P_AB: &[ParamSpec] = &[
    ParamSpec {
        name: "a",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "b",
        ty: ParamTy::Num,
        default: None,
    },
];

const P_WHERE: &[ParamSpec] = &[
    ParamSpec {
        name: "cond",
        ty: ParamTy::Bool,
        default: None,
    },
    ParamSpec {
        name: "a",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "b",
        ty: ParamTy::Num,
        default: None,
    },
];

const P_CLAMP: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "lo",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "hi",
        ty: ParamTy::Num,
        default: None,
    },
];

const P_DIFF: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstNonNegInt,
        default: Some(ParamDefault::Int(1)),
    },
];

const P_PCT_CHANGE: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstPosInt,
        default: Some(ParamDefault::Int(1)),
    },
];

const P_XN: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstPosInt,
        default: None,
    },
];

const P_XN_MIN2: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstIntMin(2),
        default: None,
    },
];

const P_RSI: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstPosInt,
        default: Some(ParamDefault::Int(14)),
    },
];

const P_XYN: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "y",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstIntMin(2),
        default: None,
    },
];

const P_TR: &[ParamSpec] = &[
    ParamSpec {
        name: "high",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "low",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "close",
        ty: ParamTy::Num,
        default: None,
    },
];

const P_ATR: &[ParamSpec] = &[
    ParamSpec {
        name: "high",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "low",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "close",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstPosInt,
        default: Some(ParamDefault::Int(14)),
    },
];

const P_MACD: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "fast",
        ty: ParamTy::ConstPosInt,
        default: Some(ParamDefault::Int(12)),
    },
    ParamSpec {
        name: "slow",
        ty: ParamTy::ConstPosInt,
        default: Some(ParamDefault::Int(26)),
    },
    ParamSpec {
        name: "signal",
        ty: ParamTy::ConstPosInt,
        default: Some(ParamDefault::Int(9)),
    },
];

const P_BOLLINGER: &[ParamSpec] = &[
    ParamSpec {
        name: "x",
        ty: ParamTy::Num,
        default: None,
    },
    ParamSpec {
        name: "n",
        ty: ParamTy::ConstIntMin(2),
        default: Some(ParamDefault::Int(20)),
    },
    ParamSpec {
        name: "mult",
        ty: ParamTy::ConstDec,
        default: Some(ParamDefault::Dec(20, 1)),
    },
];

const P_SINCE: &[ParamSpec] = &[ParamSpec {
    name: "cond",
    ty: ParamTy::Bool,
    default: None,
}];

macro_rules! op {
    ($name:expr, $params:expr, $ret:expr, $warmup:expr, $state:expr, $cost:expr, $nan:expr, $ex:expr, $fix:expr) => {
        OpSpec {
            name: $name,
            params: $params,
            ret: $ret,
            warmup: $warmup,
            state_bytes: $state,
            cost: $cost,
            nan_rule: $nan,
            doc_example: $ex,
            fixture_path: $fix,
        }
    };
}

/// Complete Tier 1 operator specification table.
pub const TIER1_OPS: &[OpSpec] = &[
    op!(
        "abs",
        P_X,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "abs(close - open)",
        "tests/fixtures/ref/abs.csv"
    ),
    op!(
        "sqrt",
        P_X,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "sqrt(var(close, 20))",
        "tests/fixtures/ref/sqrt.csv"
    ),
    op!(
        "log",
        P_X,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "log(close)",
        "tests/fixtures/ref/log.csv"
    ),
    op!(
        "exp",
        P_X,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "exp(ret)",
        "tests/fixtures/ref/exp.csv"
    ),
    op!(
        "min",
        P_AB,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "min(open, close)",
        "tests/fixtures/ref/min.csv"
    ),
    op!(
        "max",
        P_AB,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "max(open, close)",
        "tests/fixtures/ref/max.csv"
    ),
    op!(
        "where",
        P_WHERE,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Select,
        "where(close > open, high, low)",
        "tests/fixtures/ref/where.csv"
    ),
    op!(
        "clamp",
        P_CLAMP,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "clamp(rsi(close, 14), 20, 80)",
        "tests/fixtures/ref/clamp.csv"
    ),
    op!(
        "mod",
        P_AB,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        0,
        CostClass::Constant,
        NanRule::Propagate,
        "mod(volume, 5)",
        "tests/fixtures/ref/mod.csv"
    ),
    op!(
        "diff",
        P_DIFF,
        ReturnTy::Num,
        WarmupRule::Param(1),
        8,
        CostClass::Constant,
        NanRule::Propagate,
        "diff(close, 1)",
        "tests/fixtures/ref/diff.csv"
    ),
    op!(
        "pct_change",
        P_PCT_CHANGE,
        ReturnTy::Num,
        WarmupRule::Param(1),
        8,
        CostClass::Constant,
        NanRule::Propagate,
        "pct_change(close, 1)",
        "tests/fixtures/ref/pct_change.csv"
    ),
    op!(
        "cumsum",
        P_X,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        16,
        CostClass::Constant,
        NanRule::Stateful,
        "cumsum(volume)",
        "tests/fixtures/ref/cumsum.csv"
    ),
    op!(
        "sum",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        24,
        CostClass::Constant,
        NanRule::Propagate,
        "sum(volume, 20)",
        "tests/fixtures/ref/sum.csv"
    ),
    op!(
        "mean",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        24,
        CostClass::Constant,
        NanRule::Propagate,
        "mean(close, 20)",
        "tests/fixtures/ref/mean.csv"
    ),
    op!(
        "sma",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        24,
        CostClass::Constant,
        NanRule::Propagate,
        "sma(close, 20)",
        "tests/fixtures/ref/sma.csv"
    ),
    op!(
        "std",
        P_XN_MIN2,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Constant,
        NanRule::Propagate,
        "std(close, 20)",
        "tests/fixtures/ref/std.csv"
    ),
    op!(
        "var",
        P_XN_MIN2,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Constant,
        NanRule::Propagate,
        "var(close, 20)",
        "tests/fixtures/ref/var.csv"
    ),
    op!(
        "ema",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        24,
        CostClass::Constant,
        NanRule::Stateful,
        "ema(close, 12)",
        "tests/fixtures/ref/ema.csv"
    ),
    op!(
        "rma",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        24,
        CostClass::Constant,
        NanRule::Stateful,
        "rma(close, 14)",
        "tests/fixtures/ref/rma.csv"
    ),
    op!(
        "wma",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        24,
        CostClass::Constant,
        NanRule::Propagate,
        "wma(close, 10)",
        "tests/fixtures/ref/wma.csv"
    ),
    op!(
        "highest",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Constant,
        NanRule::Propagate,
        "highest(high, 20)",
        "tests/fixtures/ref/highest.csv"
    ),
    op!(
        "lowest",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Constant,
        NanRule::Propagate,
        "lowest(low, 20)",
        "tests/fixtures/ref/lowest.csv"
    ),
    op!(
        "rsi",
        P_RSI,
        ReturnTy::Num,
        WarmupRule::Param(1),
        40,
        CostClass::Constant,
        NanRule::Stateful,
        "rsi(close, 14)",
        "tests/fixtures/ref/rsi.csv"
    ),
    op!(
        "tr",
        P_TR,
        ReturnTy::Num,
        WarmupRule::Fixed(1),
        16,
        CostClass::Constant,
        NanRule::Propagate,
        "tr(high, low, close)",
        "tests/fixtures/ref/tr.csv"
    ),
    op!(
        "atr",
        P_ATR,
        ReturnTy::Num,
        WarmupRule::TrPlusParamMinusOne(3),
        48,
        CostClass::Constant,
        NanRule::Stateful,
        "atr(high, low, close, 14)",
        "tests/fixtures/ref/atr.csv"
    ),
    op!(
        "macd",
        P_MACD,
        ReturnTy::Num,
        WarmupRule::Macd {
            fast: 1,
            slow: 2,
            signal: 3,
        },
        72,
        CostClass::Constant,
        NanRule::Stateful,
        "macd(close, 12, 26, 9)",
        "tests/fixtures/ref/macd.csv"
    ),
    op!(
        "bollinger",
        P_BOLLINGER,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        40,
        CostClass::Constant,
        NanRule::Propagate,
        "bollinger(close, 20, 2.0)",
        "tests/fixtures/ref/bollinger.csv"
    ),
    op!(
        "corr",
        P_XYN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(2),
        64,
        CostClass::Constant,
        NanRule::Propagate,
        "corr(close, volume, 20)",
        "tests/fixtures/ref/corr.csv"
    ),
    op!(
        "cov",
        P_XYN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(2),
        56,
        CostClass::Constant,
        NanRule::Propagate,
        "cov(close, volume, 20)",
        "tests/fixtures/ref/cov.csv"
    ),
    op!(
        "linreg_slope",
        P_XN_MIN2,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Constant,
        NanRule::Propagate,
        "linreg_slope(close, 20)",
        "tests/fixtures/ref/linreg_slope.csv"
    ),
    op!(
        "linreg_intercept",
        P_XN_MIN2,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Constant,
        NanRule::Propagate,
        "linreg_intercept(close, 20)",
        "tests/fixtures/ref/linreg_intercept.csv"
    ),
    op!(
        "percentile_rank",
        P_XN,
        ReturnTy::Num,
        WarmupRule::ParamMinusOne(1),
        32,
        CostClass::Linear,
        NanRule::Propagate,
        "percentile_rank(close, 252)",
        "tests/fixtures/ref/percentile_rank.csv"
    ),
    op!(
        "crossover",
        P_AB,
        ReturnTy::Bool,
        WarmupRule::Fixed(1),
        16,
        CostClass::Constant,
        NanRule::Propagate,
        "crossover(fast, slow)",
        "tests/fixtures/ref/crossover.csv"
    ),
    op!(
        "crossunder",
        P_AB,
        ReturnTy::Bool,
        WarmupRule::Fixed(1),
        16,
        CostClass::Constant,
        NanRule::Propagate,
        "crossunder(fast, slow)",
        "tests/fixtures/ref/crossunder.csv"
    ),
    op!(
        "since",
        P_SINCE,
        ReturnTy::Num,
        WarmupRule::Fixed(0),
        16,
        CostClass::Constant,
        NanRule::Stateful,
        "since(close > open)",
        "tests/fixtures/ref/since.csv"
    ),
];

/// Returns the default Tier 1 operator registry.
pub fn tier1_registry() -> OpRegistry {
    OpRegistry::from_specs(TIER1_OPS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tier1_ops_are_unique_and_complete() {
        let mut names = HashSet::new();
        for op in TIER1_OPS {
            assert!(names.insert(op.name), "duplicate op {}", op.name);
            assert!(!op.doc_example.is_empty());
            assert!(
                !op.doc_example.contains("bar_idx"),
                "doc_example for {} references undefined bar_idx",
                op.name
            );
            assert!(!op.fixture_path.is_empty());
        }
        let reg = tier1_registry();
        assert_eq!(reg.all().len(), TIER1_OPS.len());
        assert_eq!(
            reg.get("ema").unwrap().warmup.compute(&[None, Some(50)]),
            49
        );
        assert_eq!(
            reg.get("rsi").unwrap().warmup.compute(&[None, Some(14)]),
            14
        );
    }
}
