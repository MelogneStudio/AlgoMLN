use crate::{
    ast::*,
    diagnostics::{Diagnostic, FrontendError},
};
pub fn validate(s: &Strategy) -> Result<(), FrontendError> {
    let mut d = Vec::new();
    if s.rules.is_empty() {
        d.push(diag("VAL_EMPTY", "strategy must contain at least one rule"));
    }
    for (name, v) in [
        ("STOP_LOSS", s.stop_loss.as_ref()),
        ("TAKE_PROFIT", s.take_profit.as_ref()),
        ("MAX_DAILY_LOSS", s.risk.max_daily_loss.as_ref()),
    ] {
        if v.is_some_and(|x| !percent_valid(x)) {
            let value = v.unwrap();
            d.push(Diagnostic {
                code: "VAL_PERCENT",
                message: format!("{name} must be in (0, 100]"),
                span: Some(value.span),
            });
        }
    }
    if s.risk.max_orders == Some(0) || s.risk.max_positions == Some(0) {
        d.push(diag(
            "VAL_RISK_INTEGER",
            "risk integer limits must be positive",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for r in &s.rules {
        if !ids.insert(&r.id) {
            d.push(Diagnostic {
                code: "VAL_DUPLICATE_RULE_ID",
                message: format!("duplicate rule id {}", r.id),
                span: Some(r.span),
            });
        }
        if let Some(amount) = match &r.action {
            Action::Buy(x) | Action::Sell(x) => Some(x),
            Action::SellAll => None,
        } {
            match amount {
                TradeAmount::Quantity(q) if *q == 0 => d.push(Diagnostic {
                    code: "VAL_AMOUNT",
                    message: "quantity must be positive".into(),
                    span: Some(r.span),
                }),
                TradeAmount::Worth(x)
                | TradeAmount::BuyCapitalPercentage(x)
                | TradeAmount::SellPositionPercentage(x)
                    if !x.cmp_integer("0").is_some_and(|v| v.is_gt())
                        || (matches!(
                            amount,
                            TradeAmount::BuyCapitalPercentage(_)
                                | TradeAmount::SellPositionPercentage(_)
                        ) && !x.cmp_integer("100").is_some_and(|v| v.is_le())) =>
                {
                    d.push(Diagnostic {
                        code: "VAL_AMOUNT",
                        message: "amount and percentage must be positive".into(),
                        span: Some(r.span),
                    })
                }
                _ => {}
            }
        }
        if let Some(value) = match &r.action {
            Action::Buy(TradeAmount::BuyCapitalPercentage(x))
            | Action::Sell(TradeAmount::SellPositionPercentage(x)) => Some(x),
            _ => None,
        } {
            if !percent_valid(value) {
                d.push(Diagnostic {
                    code: "VAL_PERCENT",
                    message: "action percentage must be in (0, 100]".into(),
                    span: Some(r.action_span),
                });
            }
        }
        condition(&r.condition, &mut d);
    }
    if d.is_empty() {
        Ok(())
    } else {
        Err(FrontendError::Validation(d))
    }
}
fn diag(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        code,
        message: message.into(),
        span: None,
    }
}
fn percent_valid(x: &Decimal) -> bool {
    let hundred = crate::source::NumericLiteral::parse("100".into(), x.span).unwrap();
    x.value.coefficient != "0" && !x.value.negative && x.compare_value(&hundred).is_le()
}
fn condition(c: &Condition, d: &mut Vec<Diagnostic>) {
    match c {
        Condition::And(a, b) | Condition::Or(a, b) => {
            condition(a, d);
            condition(b, d)
        }
        Condition::Not(a) => condition(a, d),
        Condition::Cross { left, right, .. } => {
            let a = matches!(left, NumericExpr::Indicator(_));
            let b = matches!(right, NumericExpr::Indicator(_));
            if (!a && !matches!(left, NumericExpr::Price(_)))
                || (!b && !matches!(right, NumericExpr::Price(_)))
                || (!a && !b)
            {
                d.push(diag(
                    "VAL_CROSS_TYPES",
                    "crossovers require at least one indicator and otherwise price fields",
                ));
            }
            validate_expr(left, d);
            validate_expr(right, d);
        }
        Condition::Compare { left, right, .. } => {
            validate_expr(left, d);
            validate_expr(right, d);
        }
        Condition::Between { start, end }
            if start.hour > 23 || end.hour > 23 || start.minute > 59 || end.minute > 59 =>
        {
            d.push(diag("VAL_TIME", "time must be within 00:00..23:59"))
        }
        Condition::Between { start, end }
            if start.hour <= 23
                && end.hour <= 23
                && start.minute <= 59
                && end.minute <= 59
                && (start.hour, start.minute) > (end.hour, end.minute) =>
        {
            d.push(diag(
                "VAL_TIME_ORDER",
                "start time must not follow end time",
            ))
        }
        _ => {}
    }
}
fn validate_expr(e: &NumericExpr, d: &mut Vec<Diagnostic>) {
    if let NumericExpr::Indicator(i) = e {
        if i.period == 0 {
            d.push(diag(
                "VAL_INDICATOR_PERIOD",
                "indicator period must be positive",
            ));
        }
        if matches!(
            i.kind,
            IndicatorKind::BbUpper | IndicatorKind::BbLower | IndicatorKind::BbMid
        ) {
            if !i
                .multiplier
                .as_ref()
                .is_some_and(|m| m.cmp_integer("0").is_some_and(|v| v.is_gt()))
            {
                d.push(diag(
                    "VAL_BB_MULTIPLIER",
                    "Bollinger multiplier must be positive",
                ));
            }
        } else if i.multiplier.is_some() {
            d.push(diag(
                "VAL_INDICATOR_ARITY",
                "this indicator takes exactly one argument",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::lex, parser::parse};
    #[test]
    fn percentage_bounds_are_exact() {
        for x in ["0", "-0.1", "100.01"] {
            let s = parse(
                &lex(&format!(
                    "TRADE_IN ABC\nSTOP_LOSS {x}%\nWHEN close > 1\nBUY 1\n"
                ))
                .unwrap(),
            )
            .unwrap();
            assert!(validate(&s).is_err(), "{x}");
        }
        let s =
            parse(&lex("TRADE_IN ABC\nSTOP_LOSS 100%\nWHEN close > 1\nBUY 1\n").unwrap()).unwrap();
        assert!(validate(&s).is_ok());
    }
    #[test]
    fn action_zero_and_crossover_literal_are_rejected() {
        for src in [
            "TRADE_IN ABC\nWHEN close > 1\nBUY 0\n",
            "TRADE_IN ABC\nWHEN CROSS_ABOVE(ema(3), 1)\nBUY 1\n",
        ] {
            let s = parse(&lex(src).unwrap()).unwrap();
            assert!(validate(&s).is_err());
        }
    }
    #[test]
    fn extreme_exact_percentages_do_not_round_through_float() {
        for x in [
            "0.00000000000000000000000000000001",
            "100.0000000000000000000000000001",
        ] {
            let literal =
                crate::source::NumericLiteral::parse(x.to_string(), crate::source::Span::default())
                    .unwrap();
            assert_eq!(percent_valid(&literal), x.starts_with("0."), "{x}");
        }
    }
}
