//! Typed, serializable syntax tree. Decimal values are stored as their exact
//! source spelling to avoid floating point rounding in the frontend.
use crate::source::{NumericLiteral, Span};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Strategy {
    pub name: Option<String>,
    pub trade_in: TradeIn,
    pub stop_loss: Option<NumericLiteral>,
    pub take_profit: Option<NumericLiteral>,
    pub rules: Vec<Rule>,
    pub risk: RiskConfig,
}

// Domain-explicit names for consumers that prefer the `*Node` vocabulary.
pub type StrategyNode = Strategy;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeIn {
    Symbols(Vec<String>),
    Index(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_orders: Option<u64>,
    pub max_positions: Option<u64>,
    pub max_daily_loss: Option<NumericLiteral>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub condition: Condition,
    pub action: Action,
    pub span: Span,
    pub condition_span: Span,
    pub action_span: Span,
}
pub type RuleNode = Rule;

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
pub type ConditionNode = Condition;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumericExpr {
    Literal(Decimal),
    Price(PriceField),
    Indicator(IndicatorCall),
}

pub type Decimal = NumericLiteral;

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
pub type ActionNode = Action;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeAmount {
    Quantity(u64),
    Worth(Decimal),
    BuyCapitalPercentage(Decimal),
    SellPositionPercentage(Decimal),
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
                action: Action::Buy(TradeAmount::BuyCapitalPercentage(
                    Decimal::parse("2.5".into(), Span::default()).unwrap(),
                )),
                span: Span::default(),
                condition_span: Span::default(),
                action_span: Span::default(),
            }],
            risk: RiskConfig::default(),
        };
        let value = serde_json::to_value(&strategy).unwrap();
        assert_eq!(serde_json::from_value::<Strategy>(value).unwrap(), strategy);
    }
    #[test]
    fn exact_decimal_keeps_spelling_and_normalizes_value() {
        let a = crate::source::NumericLiteral::parse("001.500".into(), Span::default()).unwrap();
        let b = crate::source::NumericLiteral::parse("1.5".into(), Span::default()).unwrap();
        assert_eq!(a.spelling, "001.500");
        assert_eq!(a.value, b.value);
        assert_eq!(a.cmp_integer("2"), Some(std::cmp::Ordering::Less));
    }
}

#[cfg(test)]
mod property_tests {
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parsed_ast_survives_serde_round_trip_and_reparse(period in 1u32..=1000, value in 1u32..=10000) {
            let source = format!("TRADE_IN ABC\nWHEN ema({period}) > {value}\nBUY {value}\n");
            let parsed = crate::compile(&source).unwrap();
            let encoded = serde_json::to_vec(&parsed).unwrap();
            let decoded: crate::ast::Strategy = serde_json::from_slice(&encoded).unwrap();
            prop_assert_eq!(&parsed, &decoded);
            prop_assert_eq!(crate::compile(&source).unwrap(), decoded);
        }
    }
}
