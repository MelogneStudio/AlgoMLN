use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyNode {
    pub name: String,
    pub rules: Vec<RuleNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleNode {
    pub id: String,
    pub condition: ConditionNode,
    pub action: ActionNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConditionNode {
    Comparison {
        left: ExprNode,
        op: CompareOp,
        right: ExprNode,
    },
    CrossAbove {
        fast: ExprNode,
        slow: ExprNode,
    },
    CrossBelow {
        fast: ExprNode,
        slow: ExprNode,
    },
    And(Box<ConditionNode>, Box<ConditionNode>),
    Or(Box<ConditionNode>, Box<ConditionNode>),
    Not(Box<ConditionNode>),
    InPosition,
    TimeWindow {
        start: NaiveTime,
        end: NaiveTime,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExprNode {
    Indicator(IndicatorCall),
    PriceField(PriceField),
    Literal(f64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndicatorCall {
    pub kind: IndicatorKind,
    pub period: usize,
}

/// Indicator functions available in DSL expressions:
/// ema(N), ma(N), rsi(N), rel_vol(N), atr(N), vwap(N), bb_upper(N),
/// bb_lower(N), bb_mid(N).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndicatorKind {
    Ema,
    Ma,
    Rsi,
    RelVol,
    Atr,
    Vwap,
    BbUpper,
    BbLower,
    BbMid,
}

/// Candle price fields available in DSL expressions:
/// close, open, high, low, volume, prev_close.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PriceField {
    Close,
    Open,
    High,
    Low,
    Volume,
    PrevClose,
    PrevOpen,
    PrevHigh,
    PrevLow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    Neq,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionNode {
    Buy { quantity: usize },
    Sell { quantity: usize },
    SellAll,
}
