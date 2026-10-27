//! Typed, serializable syntax tree. Decimal values are stored as their exact
//! source spelling to avoid floating point rounding in the frontend.
use crate::source::Span;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strategy {
    pub name: Option<String>,
    pub trade_in: TradeIn,
    pub stop_loss: Option<Decimal>,
    pub take_profit: Option<Decimal>,
    pub rules: Vec<Rule>,
    pub risk: RiskConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeIn {
    Symbols(Vec<String>),
    Index(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_orders: Option<u64>,
    pub max_positions: Option<u64>,
    pub max_daily_loss: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub condition: Condition,
    pub action: Action,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Condition {
    Compare {
        left: NumericExpr,
        op: CompareOp,
        right: NumericExpr,
    },
    Cross {
        direction: CrossDirection,
        left: NumericExpr,
        right: NumericExpr,
    },
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
    InPosition,
    Between {
        start: Time,
        end: Time,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumericExpr {
    Literal(Decimal),
    Price(PriceField),
    Indicator(IndicatorCall),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decimal(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorCall {
    pub kind: IndicatorKind,
    pub period: u32,
    pub multiplier: Option<Decimal>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndicatorKind {
    Sma,
    Ema,
    Rsi,
    RelVol,
    Atr,
    Vwap,
    BbUpper,
    BbLower,
    BbMid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrossDirection {
    Above,
    Below,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Buy(TradeAmount),
    Sell(TradeAmount),
    SellAll,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeAmount {
    Quantity(u64),
    Worth(Decimal),
    Percentage(Decimal),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serde_round_trip_preserves_distinctions_and_rule_order() {
        let strategy = Strategy {
            name: Some("demo".into()),
            trade_in: TradeIn::Symbols(vec!["ABC".into()]),
            stop_loss: None,
            take_profit: None,
            rules: vec![Rule {
                id: "rule_1".into(),
                condition: Condition::InPosition,
                action: Action::Buy(TradeAmount::Percentage(Decimal("2.5".into()))),
                span: Span::default(),
            }],
            risk: RiskConfig::default(),
        };
        let value = serde_json::to_value(&strategy).unwrap();
        assert_eq!(serde_json::from_value::<Strategy>(value).unwrap(), strategy);
    }
}
