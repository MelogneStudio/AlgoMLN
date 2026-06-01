use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::ast::{
    ActionNode, ConditionNode, ExprNode, IndicatorCall, IndicatorKind, RuleNode, StrategyNode,
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

        errors
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
    match rule.action {
        ActionNode::Buy { quantity } | ActionNode::Sell { quantity } if quantity == 0 => {
            errors.push(ValidationError {
                rule_id: rule.id.clone(),
                message: "order quantity must be greater than zero".to_string(),
                kind: ValidationErrorKind::InvalidQuantity,
            });
        }
        _ => {}
    }
}

fn indicator_name(kind: &IndicatorKind) -> &'static str {
    match kind {
        IndicatorKind::Ema => "ema",
        IndicatorKind::Ma => "ma",
        IndicatorKind::Rsi => "rsi",
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
    use crate::strategy::dsl::ast::{CompareOp, PriceField};

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
            ActionNode::Buy { quantity: 1 },
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
            ActionNode::Buy { quantity: 0 },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidQuantity)));
    }

    #[test]
    fn rejects_cross_with_literal() {
        let errors = AstValidator::validate(&strategy(vec![rule(
            "rule_0",
            ConditionNode::CrossAbove {
                fast: ExprNode::Literal(30.0),
                slow: ExprNode::PriceField(PriceField::Close),
            },
            ActionNode::Buy { quantity: 1 },
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
            ActionNode::Buy { quantity: 1 },
        )]));
        assert!(errors
            .iter()
            .any(|err| matches!(err.kind, ValidationErrorKind::InvalidTimeRange)));
    }

    #[test]
    fn collects_all_errors() {
        let errors = AstValidator::validate(&strategy(vec![
            rule("same", price_condition(), ActionNode::Buy { quantity: 0 }),
            rule(
                "same",
                ConditionNode::CrossBelow {
                    fast: ExprNode::Literal(1.0),
                    slow: ExprNode::Literal(2.0),
                },
                ActionNode::Sell { quantity: 0 },
            ),
        ]));
        assert!(errors.len() >= 4);
    }

    fn price_condition() -> ConditionNode {
        ConditionNode::Comparison {
            left: ExprNode::PriceField(PriceField::Close),
            op: CompareOp::Gt,
            right: ExprNode::Literal(10.0),
        }
    }
}
