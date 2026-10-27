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
            d.push(diag("VAL_PERCENT", format!("{name} must be in (0, 100]")));
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
                TradeAmount::Worth(x) | TradeAmount::Percentage(x)
                    if !decimal_cmp(&x.0, "0").is_some_and(|v| v > 0) =>
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
    decimal_cmp(&x.0, "0").is_some_and(|v| v > 0)
        && decimal_cmp(&x.0, "100").is_some_and(|v| v <= 0)
}
fn decimal_cmp(a: &str, b: &str) -> Option<i8> {
    fn parts(s: &str) -> Option<(bool, String, String)> {
        let (neg, s) = if let Some(x) = s.strip_prefix('-') {
            (true, x)
        } else {
            (false, s.strip_prefix('+').unwrap_or(s))
        };
        let mut split = s.split('.');
        let whole = split.next()?;
        let frac = split.next().unwrap_or("");
        if split.next().is_some()
            || whole.is_empty()
            || !whole.bytes().all(|x| x.is_ascii_digit())
            || !frac.bytes().all(|x| x.is_ascii_digit())
        {
            return None;
        }
        let whole = whole.trim_start_matches('0');
        let frac = frac.trim_end_matches('0');
        Some((
            neg,
            if whole.is_empty() { "0" } else { whole }.to_owned(),
            frac.to_owned(),
        ))
    }
    let (an, aw, af) = parts(a)?;
    let (bn, bw, bf) = parts(b)?;
    if an != bn {
        return Some(if an { -1 } else { 1 });
    }
    let mag = aw
        .len()
        .cmp(&bw.len())
        .then_with(|| aw.cmp(&bw))
        .then_with(|| {
            let n = af.len().max(bf.len());
            af.chars()
                .chain(std::iter::repeat('0'))
                .take(n)
                .cmp(bf.chars().chain(std::iter::repeat('0')).take(n))
        });
    Some(match (an, mag) {
        (true, std::cmp::Ordering::Less) => 1,
        (true, std::cmp::Ordering::Greater) => -1,
        (false, std::cmp::Ordering::Less) => -1,
        (false, std::cmp::Ordering::Greater) => 1,
        _ => 0,
    })
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
            if !a || !b {
                d.push(diag(
                    "VAL_CROSS_TYPES",
                    "crossovers require two indicator expressions",
                ));
            }
        }
        Condition::Between { start, end }
            if start.hour > 23 || end.hour > 23 || start.minute > 59 || end.minute > 59 =>
        {
            d.push(diag("VAL_TIME", "time must be within 00:00..23:59"))
        }
        _ => {}
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
}
