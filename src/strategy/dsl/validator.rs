use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::ast::{
    ActionNode, ConditionNode, ExprNode, IndicatorCall, IndicatorKind, QuantitySpec, RuleNode,
    StrategyNode, TradeIn,
};

pub struct AstValidator;

impl AstValidator {
    pub fn validate(strategy: &StrategyNode) -> Vec<ValidationError> {
        let mut errors = Vec::new();

        if strategy.rules.is_empty() {
            errors.push(ValidationError {
                rule_id: String::new(),
                message: "strategy must contain at least one rule".to_string(),
                kind: ValidationErrorKind::EmptyStrategy,
            });
        }

        let mut seen = HashSet::new();
        let mut duplicates = HashSet::new();
        for rule in &strategy.rules {
            if !seen.insert(rule.id.clone()) {
                duplicates.insert(rule.id.clone());
            }
            validate_rule(rule, &mut errors);
        }

        for rule_id in duplicates {
            errors.push(ValidationError {
                rule_id,
                message: "duplicate rule id".to_string(),
                kind: ValidationErrorKind::DuplicateRuleIds,
            });
        }

        if strategy.trade_in.is_some() {
            validate_trade_in(strategy.trade_in.as_ref().unwrap(), &mut errors);
        }

        validate_percent_threshold(strategy.stop_loss, "STOP_LOSS", &mut errors);
        validate_percent_threshold(strategy.take_profit, "TAKE_PROFIT", &mut errors);

        if let Some(risk) = &strategy.risk {
            validate_risk(risk, &mut errors);
        }

        errors
    }
}

/// Validate a `RiskConfig`. `max_daily_loss_pct` must be in `(0.0, 100.0]`
/// (zero means "disabled" — use `None` instead). `max_open_positions` and
/// `max_orders` must be `>= 1` (zero would mean "never trade").
fn validate_risk(risk: &super::ast::RiskConfig, errors: &mut Vec<ValidationError>) {
    if let Some(pct) = risk.max_daily_loss_pct {
        if !pct.is_finite() || pct <= 0.0 || pct > 100.0 {
            errors.push(ValidationError {
                rule_id: String::new(),
                message: format!("RISK MAX_DAILY_LOSS must be in (0, 100] (got {pct})"),
                kind: ValidationErrorKind::InvalidMaxDailyLoss,
            });
        }
    }
    if let Some(positions) = risk.max_open_positions {
        if positions == 0 {
            errors.push(ValidationError {
                rule_id: String::new(),
                message: "RISK MAX_POSITIONS must be >= 1".to_string(),
                kind: ValidationErrorKind::InvalidMaxPositions,
            });
        }
    }
    if let Some(orders) = risk.max_orders {
        if orders == 0 {
            errors.push(ValidationError {
                rule_id: String::new(),
                message: "RISK MAX_ORDERS must be >= 1".to_string(),
                kind: ValidationErrorKind::InvalidMaxOrders,
            });
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub rule_id: String,
    pub message: String,
    pub kind: ValidationErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationErrorKind {
    EmptyStrategy,
    InvalidPeriod { indicator: String, period: usize },
    InvalidQuantity,
    CrossWithLiteral,
    DuplicateRuleIds,
    InvalidTimeRange,
    InvalidTradeIn,
    InvalidStopLoss,
    InvalidTakeProfit,
    InvalidMaxDailyLoss,
    InvalidMaxPositions,
    InvalidMaxOrders,
}

/// Validate an optional percentage threshold (STOP_LOSS / TAKE_PROFIT). Both
/// must be in `(0.0, 100.0]` — zero means "disabled" (use `None`) and over 100
/// is nonsensical. The two thresholds are validated independently; they don't
/// need to sum to anything specific.
fn validate_percent_threshold(value: Option<f64>, label: &str, errors: &mut Vec<ValidationError>) {
    if let Some(pct) = value {
        if !pct.is_finite() || pct <= 0.0 || pct > 100.0 {
            let kind = if label == "STOP_LOSS" {
                ValidationErrorKind::InvalidStopLoss
            } else {
                ValidationErrorKind::InvalidTakeProfit
            };
            errors.push(ValidationError {
                rule_id: String::new(),
                message: format!("{label} must be in (0, 100] (got {pct})"),
                kind,
            });
        }
    }
}

fn validate_trade_in(trade_in: &TradeIn, errors: &mut Vec<ValidationError>) {
    match trade_in {
        TradeIn::Symbols(syms) => {
            if syms.is_empty() {
                errors.push(ValidationError {
                    rule_id: String::new(),
                    message: "TRADE_IN symbol list is empty".to_string(),
                    kind: ValidationErrorKind::InvalidTradeIn,
                });
            }
            if syms.len() > 500 {
                errors.push(ValidationError {
                    rule_id: String::new(),
                    message:
                        "TRADE_IN explicit symbol list exceeds 500 symbols; use an index alias instead"
                            .to_string(),
                    kind: ValidationErrorKind::InvalidTradeIn,
                });
            }
            for sym in syms {
                if sym.is_empty() {
                    errors.push(ValidationError {
                        rule_id: String::new(),
                        message: "TRADE_IN contains an empty symbol".to_string(),
                        kind: ValidationErrorKind::InvalidTradeIn,
                    });
                }
                if sym.len() > 30 {
                    errors.push(ValidationError {
                        rule_id: String::new(),
                        message: format!("TRADE_IN symbol '{}' is too long (max 30 chars)", sym),
                        kind: ValidationErrorKind::InvalidTradeIn,
                    });
                }
            }
        }
        TradeIn::Index(_) => {
            // Valid — resolved at runtime from IndexRegistry.
        }
    }
}

fn validate_rule(rule: &RuleNode, errors: &mut Vec<ValidationError>) {
    validate_condition(&rule.id, &rule.condition, errors);
    validate_action(rule, errors);
}

fn validate_condition(rule_id: &str, condition: &ConditionNode, errors: &mut Vec<ValidationError>) {
    match condition {
        ConditionNode::Comparison { left, right, .. } => {
            validate_expr(rule_id, left, errors);
            validate_expr(rule_id, right, errors);
        }
        ConditionNode::CrossAbove { fast, slow } | ConditionNode::CrossBelow { fast, slow } => {
            validate_expr(rule_id, fast, errors);
            validate_expr(rule_id, slow, errors);
            if matches!(fast, ExprNode::Literal(_)) || matches!(slow, ExprNode::Literal(_)) {
                errors.push(ValidationError {
                    rule_id: rule_id.to_string(),
                    message: "cross conditions cannot use literal operands".to_string(),
                    kind: ValidationErrorKind::CrossWithLiteral,
                });
            }
        }
        ConditionNode::And(left, right) | ConditionNode::Or(left, right) => {
            validate_condition(rule_id, left, errors);
            validate_condition(rule_id, right, errors);
        }
        ConditionNode::Not(inner) => validate_condition(rule_id, inner, errors),
        ConditionNode::InPosition => {}
        ConditionNode::TimeWindow { start, end } => {
            if start >= end {
                errors.push(ValidationError {
                    rule_id: rule_id.to_string(),
                    message: "time window start must be before end".to_string(),
                    kind: ValidationErrorKind::InvalidTimeRange,
                });
            }
        }
    }
}

fn validate_expr(rule_id: &str, expr: &ExprNode, errors: &mut Vec<ValidationError>) {
    if let ExprNode::Indicator(call) = expr {
        validate_indicator(rule_id, call, errors);
    }
}

fn validate_indicator(rule_id: &str, call: &IndicatorCall, errors: &mut Vec<ValidationError>) {
    if call.period == 0 {
        errors.push(ValidationError {
            rule_id: rule_id.to_string(),
            message: "indicator period must be greater than zero".to_string(),
            kind: ValidationErrorKind::InvalidPeriod {
                indicator: indicator_name(&call.kind).to_string(),
                period: call.period,
            },
        });
    }
}

fn validate_action(rule: &RuleNode, errors: &mut Vec<ValidationError>) {
    let quantity = match &rule.action {
        ActionNode::Buy { quantity } | ActionNode::Sell { quantity } => quantity,
        ActionNode::SellAll => return,
    };

    match quantity {
        QuantitySpec::Fixed(value) if *value == 0 => {
            push_invalid_quantity(rule, errors, "order quantity must be greater than zero");
        }
        QuantitySpec::PercentCapital(value) if *value <= 0.0 || *value > 100.0 => {
            push_invalid_quantity(
                rule,
                errors,
                &format!("percent quantity must be between 0 and 100 (got {value})"),
            );
        }
        QuantitySpec::ValueBased(value) if *value <= 0.0 => {
            push_invalid_quantity(
                rule,
                errors,
                &format!("WORTH quantity must be positive (got {value})"),
            );
        }
        _ => {}
    }
}

fn push_invalid_quantity(rule: &RuleNode, errors: &mut Vec<ValidationError>, message: &str) {
    errors.push(ValidationError {
        rule_id: rule.id.clone(),
        message: message.to_string(),
        kind: ValidationErrorKind::InvalidQuantity,
    });
}

fn indicator_name(kind: &IndicatorKind) -> &'static str {
    match kind {
        IndicatorKind::Ema => "ema",
        IndicatorKind::Ma => "ma",
        IndicatorKind::Rsi => "rsi",
        IndicatorKind::RelVol => "rel_vol",
        IndicatorKind::Atr => "atr",
        IndicatorKind::Vwap => "vwap",
        IndicatorKind::BbUpper => "bb_upper",
        IndicatorKind::BbLower => "bb_lower",
        IndicatorKind::BbMid => "bb_mid",
    }
}

#[cfg(test)]
mod tests {
    use chrono::NaiveTime;

    use super::*;
    use crate::strategy::dsl::ast::{CompareOp, IndexAlias, PriceField};

    fn rule(id: &str, condition: ConditionNode, action: ActionNode) -> RuleNode {
        RuleNode {
            id: id.to_string(),
            condition,
            action,
        }
    }

    fn strategy(rules: Vec<RuleNode>) -> StrategyNode {
        StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: None,
            risk: None,
            rules,
        }
    }

    #[test]
    fn rejects_empty_strategy() {
        let errors = AstValidator::validate(&strategy(vec![]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::EmptyStrategy)));
    }

    #[test]
    fn valid_strategy_has_no_errors() {
        let errors = AstValidator::validate(&strategy(vec![make_rsi_rule(14, 5)]));
        assert!(errors.is_empty());
    }

    #[test]
    fn rejects_zero_indicator_period() {
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            ConditionNode::Comparison {
                left: ExprNode::Indicator(IndicatorCall {
                    kind: IndicatorKind::Ema,
                    period: 0,
                }),
                op: CompareOp::Gt,
                right: ExprNode::Literal(10.0),
            },
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(1),
            },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidPeriod { .. })));
    }

    #[test]
    fn rejects_zero_period() {
        let errors = AstValidator::validate(&strategy(vec![make_rsi_rule(0, 5)]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidPeriod { .. })));
    }

    #[test]
    fn rejects_rel_vol_with_zero_period() {
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            ConditionNode::Comparison {
                left: ExprNode::Indicator(IndicatorCall {
                    kind: IndicatorKind::RelVol,
                    period: 0,
                }),
                op: CompareOp::Gt,
                right: ExprNode::Literal(1.5),
            },
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(1),
            },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidPeriod { .. })));
    }

    #[test]
    fn rejects_zero_quantity() {
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            price_condition(),
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(0),
            },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidQuantity)));
    }

    #[test]
    fn rejects_zero_quantity_from_rsi_rule() {
        let errors = AstValidator::validate(&strategy(vec![make_rsi_rule(14, 0)]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidQuantity)));
    }

    #[test]
    fn rejects_invalid_percent_and_worth_quantities() {
        let errors = AstValidator::validate(&strategy(vec![
            rule(
                "rule_0",
                price_condition(),
                ActionNode::Buy {
                    quantity: QuantitySpec::PercentCapital(101.0),
                },
            ),
            rule(
                "rule_1",
                price_condition(),
                ActionNode::Buy {
                    quantity: QuantitySpec::ValueBased(0.0),
                },
            ),
        ]));
        assert!(errors
            .iter()
            .any(|err| err.message == "percent quantity must be between 0 and 100 (got 101)"));
        assert!(errors
            .iter()
            .any(|err| err.message == "WORTH quantity must be positive (got 0)"));
    }

    #[test]
    fn rejects_cross_with_literal() {
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            ConditionNode::CrossAbove {
                fast: ExprNode::Literal(30.0),
                slow: ExprNode::PriceField(PriceField::Close),
            },
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(1),
            },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::CrossWithLiteral)));
    }

    #[test]
    fn rejects_cross_with_literal_operand() {
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            ConditionNode::CrossAbove {
                fast: ExprNode::Literal(30.0),
                slow: ExprNode::Indicator(IndicatorCall {
                    kind: IndicatorKind::Ema,
                    period: 20,
                }),
            },
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(1),
            },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::CrossWithLiteral)));
    }

    #[test]
    fn rejects_invalid_time_range() {
        let start = NaiveTime::from_hms_opt(10, 0, 0).unwrap();
        let end = NaiveTime::from_hms_opt(9, 30, 0).unwrap();
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            ConditionNode::TimeWindow { start, end },
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(1),
            },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidTimeRange)));
    }

    #[test]
    fn collects_all_errors() {
        let errors = AstValidator::validate(&strategy(vec![
            rule(
                "same",
                price_condition(),
                ActionNode::Buy {
                    quantity: QuantitySpec::Fixed(0),
                },
            ),
            rule(
                "same",
                ConditionNode::CrossBelow {
                    fast: ExprNode::Literal(1.0),
                    slow: ExprNode::Literal(2.0),
                },
                ActionNode::Sell {
                    quantity: QuantitySpec::Fixed(0),
                },
            ),
        ]));
        assert!(errors.len() >= 4);
    }

    #[test]
    fn collects_all_errors_not_just_first() {
        let errors = AstValidator::validate(&strategy(vec![make_rsi_rule(0, 0)]));
        assert!(errors.len() >= 2);
    }

    #[test]
    fn accepts_valid_index_alias() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: Some(TradeIn::Index(IndexAlias::NiftyBank)),
            stop_loss: None,
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        let errors = AstValidator::validate(&strat);
        assert!(!errors
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidTradeIn)));
    }

    #[test]
    fn rejects_over_500_symbols() {
        let big_list: Vec<String> = (0..=500).map(|i| format!("SYM{}", i)).collect();
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: Some(TradeIn::Symbols(big_list)),
            stop_loss: None,
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        let errors = AstValidator::validate(&strat);
        assert!(errors
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidTradeIn)
                && e.message.contains("exceeds 500")));
    }

    // ---------- STOP_LOSS / TAKE_PROFIT ----------

    #[test]
    fn accepts_valid_stop_loss() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: Some(2.0),
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        let errors = AstValidator::validate(&strat);
        assert!(!errors
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidStopLoss)));
    }

    #[test]
    fn accepts_valid_take_profit() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: Some(5.0),
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        let errors = AstValidator::validate(&strat);
        assert!(!errors
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidTakeProfit)));
    }

    #[test]
    fn accepts_stop_loss_at_100() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: Some(100.0),
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        assert!(!AstValidator::validate(&strat)
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidStopLoss)));
    }

    #[test]
    fn rejects_zero_stop_loss() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: Some(0.0),
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        assert!(AstValidator::validate(&strat)
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidStopLoss)));
    }

    #[test]
    fn rejects_over_100_stop_loss() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: Some(150.0),
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        assert!(AstValidator::validate(&strat)
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidStopLoss)));
    }

    #[test]
    fn rejects_zero_take_profit() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: Some(0.0),
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        assert!(AstValidator::validate(&strat)
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidTakeProfit)));
    }

    #[test]
    fn rejects_negative_stop_loss() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: Some(-1.0),
            take_profit: None,
            risk: None,
            rules: vec![make_rsi_rule(14, 5)],
        };
        assert!(AstValidator::validate(&strat)
            .iter()
            .any(|e| matches!(e.kind, ValidationErrorKind::InvalidStopLoss)));
    }

    // ---------- RISK ----------

    #[test]
    fn accepts_valid_risk_config() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: None,
            risk: Some(crate::strategy::dsl::ast::RiskConfig {
                max_daily_loss_pct: Some(5.0),
                max_open_positions: Some(3),
                max_orders: Some(20),
            }),
            rules: vec![make_rsi_rule(14, 5)],
        };
        let errors = AstValidator::validate(&strat);
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    }

    #[test]
    fn accepts_empty_risk_config() {
        let strat = StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: None,
            risk: Some(crate::strategy::dsl::ast::RiskConfig::default()),
            rules: vec![make_rsi_rule(14, 5)],
        };
        assert!(!errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    #[test]
    fn accepts_max_daily_loss_at_100() {
        let strat = make_risk_strategy(Some(100.0), None, None);
        assert!(!errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    #[test]
    fn rejects_zero_max_daily_loss() {
        let strat = make_risk_strategy(Some(0.0), None, None);
        assert!(errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    #[test]
    fn rejects_negative_max_daily_loss() {
        let strat = make_risk_strategy(Some(-1.0), None, None);
        assert!(errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    #[test]
    fn rejects_over_100_max_daily_loss() {
        let strat = make_risk_strategy(Some(150.0), None, None);
        assert!(errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    #[test]
    fn rejects_zero_max_positions() {
        let strat = make_risk_strategy(None, Some(0), None);
        assert!(errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    #[test]
    fn rejects_zero_max_orders() {
        let strat = make_risk_strategy(None, None, Some(0));
        assert!(errors_have_risk_errors(&AstValidator::validate(&strat)));
    }

    fn make_risk_strategy(
        max_loss: Option<f64>,
        max_pos: Option<u32>,
        max_ord: Option<u32>,
    ) -> StrategyNode {
        StrategyNode {
            name: "test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: None,
            risk: Some(crate::strategy::dsl::ast::RiskConfig {
                max_daily_loss_pct: max_loss,
                max_open_positions: max_pos,
                max_orders: max_ord,
            }),
            rules: vec![make_rsi_rule(14, 5)],
        }
    }

    fn errors_have_risk_errors(errors: &[ValidationError]) -> bool {
        errors.iter().any(|e| {
            matches!(
                e.kind,
                ValidationErrorKind::InvalidMaxDailyLoss
                    | ValidationErrorKind::InvalidMaxPositions
                    | ValidationErrorKind::InvalidMaxOrders
            )
        })
    }

    fn make_rsi_rule(period: usize, quantity: u64) -> RuleNode {
        rule(
            "rule_0",
            ConditionNode::Comparison {
                left: ExprNode::Indicator(IndicatorCall {
                    kind: IndicatorKind::Rsi,
                    period,
                }),
                op: CompareOp::Lt,
                right: ExprNode::Literal(30.0),
            },
            ActionNode::Buy {
                quantity: QuantitySpec::Fixed(quantity),
            },
        )
    }

    fn price_condition() -> ConditionNode {
        ConditionNode::Comparison {
            left: ExprNode::PriceField(PriceField::Close),
            op: CompareOp::Gt,
            right: ExprNode::Literal(10.0),
        }
    }
}
