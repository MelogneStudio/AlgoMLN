use crate::{
    ast::*,
    diagnostics::FrontendError,
    lexer::{Token, TokenKind},
    source::Span,
};

pub fn parse(tokens: &[Token]) -> Result<Strategy, FrontendError> {
    Parser { t: tokens, i: 0 }.strategy()
}
struct Parser<'a> {
    t: &'a [Token],
    i: usize,
}
impl<'a> Parser<'a> {
    fn k(&self) -> &TokenKind {
        &self.t[self.i].kind
    }
    fn span(&self) -> Span {
        self.t[self.i].span
    }
    fn bump(&mut self) -> Token {
        let x = self.t[self.i].clone();
        if !matches!(x.kind, TokenKind::Eof) {
            self.i += 1;
        }
        x
    }
    fn word(&self, s: &str) -> bool {
        matches!(self.k(),TokenKind::Word(w) if w.eq_ignore_ascii_case(s))
    }
    fn take_word(&mut self, s: &str) -> bool {
        if self.word(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn fail<T>(&self, msg: &str) -> Result<T, FrontendError> {
        let p = self.span();
        Err(FrontendError::Parse {
            code: "PARSE_SYNTAX",
            message: msg.into(),
            line: p.line,
            column: p.column,
        })
    }
    fn newlines(&mut self) {
        while matches!(self.k(), TokenKind::Newline) {
            self.bump();
        }
    }
    fn strategy(mut self) -> Result<Strategy, FrontendError> {
        self.newlines();
        let name = if self.take_word("STRATEGY") {
            match self.bump().kind {
                TokenKind::Word(w) => {
                    self.newlines();
                    Some(w)
                }
                _ => return self.fail("expected strategy name"),
            }
        } else {
            None
        };
        if !self.take_word("TRADE_IN") {
            return self.fail("expected TRADE_IN declaration");
        }
        let trade_in = self.trade_in()?;
        self.newlines();
        let mut stop_loss = None;
        let mut take_profit = None;
        let mut risk = RiskConfig::default();
        let mut rules = Vec::new();
        loop {
            self.newlines();
            if matches!(self.k(), TokenKind::Eof) {
                break;
            }
            if self.take_word("STOP_LOSS") {
                if stop_loss.is_some() {
                    return self.fail("duplicate STOP_LOSS");
                }
                stop_loss = Some(self.percent()?);
                self.newlines();
                continue;
            }
            if self.take_word("TAKE_PROFIT") {
                if take_profit.is_some() {
                    return self.fail("duplicate TAKE_PROFIT");
                }
                take_profit = Some(self.percent()?);
                self.newlines();
                continue;
            }
            if self.take_word("RISK") {
                let key = match self.bump().kind {
                    TokenKind::Word(w) => w.to_ascii_uppercase(),
                    _ => return self.fail("expected risk setting"),
                };
                match key.as_str() {
                    "MAX_ORDERS" => {
                        if risk.max_orders.is_some() {
                            return self.fail("duplicate MAX_ORDERS");
                        }
                        risk.max_orders = Some(self.uint()?);
                    }
                    "MAX_POSITIONS" => {
                        if risk.max_positions.is_some() {
                            return self.fail("duplicate MAX_POSITIONS");
                        }
                        risk.max_positions = Some(self.uint()?);
                    }
                    "MAX_DAILY_LOSS" => {
                        if risk.max_daily_loss.is_some() {
                            return self.fail("duplicate MAX_DAILY_LOSS");
                        }
                        risk.max_daily_loss = Some(self.percent()?);
                    }
                    _ => return self.fail("unknown risk setting"),
                };
                self.newlines();
                continue;
            }
            if self.take_word("WHEN") {
                let start = self.t[self.i - 1].span;
                let condition = self.condition()?;
                if !matches!(self.k(), TokenKind::Newline) {
                    return self.fail("rule condition must end at newline");
                }
                self.newlines();
                let action = self.action()?;
                let end = self.t[self.i - 1].span;
                let n = rules.len() + 1;
                rules.push(Rule {
                    id: format!("rule_{n}"),
                    condition,
                    action,
                    span: Span::new(start.start, end.end, start.line, start.column),
                });
                if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
                    return self.fail("expected newline after action");
                }
                self.newlines();
                continue;
            }
            return self.fail("expected declaration or WHEN rule");
        }
        Ok(Strategy {
            name,
            trade_in,
            stop_loss,
            take_profit,
            rules,
            risk,
        })
    }
    fn trade_in(&mut self) -> Result<TradeIn, FrontendError> {
        if self.take_word("INDEX") {
            return match self.bump().kind {
                TokenKind::Word(w) => Ok(TradeIn::Index(w)),
                _ => self.fail("expected index alias"),
            };
        }
        let mut xs = Vec::new();
        loop {
            match self.bump().kind {
                TokenKind::Word(w) => xs.push(w),
                _ => return self.fail("expected symbol or INDEX"),
            };
            if !matches!(self.k(), TokenKind::Comma) {
                break;
            }
            self.bump();
        }
        if xs.is_empty() {
            self.fail("TRADE_IN requires symbol(s)")
        } else {
            Ok(TradeIn::Symbols(xs))
        }
    }
    fn percent(&mut self) -> Result<Decimal, FrontendError> {
        let n = self.number()?;
        if !matches!(self.k(), TokenKind::Percent) {
            return self.fail("expected %");
        };
        self.bump();
        Ok(Decimal(n))
    }
    fn number(&mut self) -> Result<String, FrontendError> {
        let sign = if matches!(self.k(), TokenKind::Minus | TokenKind::Plus) {
            let s = if matches!(self.k(), TokenKind::Minus) {
                "-"
            } else {
                "+"
            };
            self.bump();
            s
        } else {
            ""
        };
        match self.bump().kind {
            TokenKind::Number(n) => Ok(format!("{sign}{n}")),
            _ => self.fail("expected number"),
        }
    }
    fn uint(&mut self) -> Result<u64, FrontendError> {
        let n = self.number()?;
        if n.starts_with(['-', '+']) || n.contains('.') {
            return self.fail("expected unsigned integer");
        }
        n.parse().map_err(|_| FrontendError::Parse {
            code: "PARSE_INTEGER",
            message: "integer out of range".into(),
            line: self.span().line,
            column: self.span().column,
        })
    }
    fn action(&mut self) -> Result<Action, FrontendError> {
        let buy = if self.take_word("BUY") {
            true
        } else if self.take_word("SELL") {
            false
        } else {
            return self.fail("expected BUY or SELL action");
        };
        if !buy && self.take_word("ALL") {
            return Ok(Action::SellAll);
        }
        let n = self.number()?;
        let amount = if self.take_word("WORTH") {
            TradeAmount::Worth(Decimal(n))
        } else if matches!(self.k(), TokenKind::Percent) {
            self.bump();
            TradeAmount::Percentage(Decimal(n))
        } else {
            let q = n.parse::<u64>().map_err(|_| FrontendError::Parse {
                code: "PARSE_INTEGER",
                message: "quantity must be a nonnegative integer".into(),
                line: self.span().line,
                column: self.span().column,
            })?;
            TradeAmount::Quantity(q)
        };
        Ok(if buy {
            Action::Buy(amount)
        } else {
            Action::Sell(amount)
        })
    }
    fn condition(&mut self) -> Result<Condition, FrontendError> {
        self.or()
    }
    fn or(&mut self) -> Result<Condition, FrontendError> {
        let mut x = self.and()?;
        while self.take_word("OR") {
            x = Condition::Or(Box::new(x), Box::new(self.and()?));
        }
        Ok(x)
    }
    fn and(&mut self) -> Result<Condition, FrontendError> {
        let mut x = self.not()?;
        while self.take_word("AND") {
            x = Condition::And(Box::new(x), Box::new(self.not()?));
        }
        Ok(x)
    }
    fn not(&mut self) -> Result<Condition, FrontendError> {
        if self.take_word("NOT") {
            return Ok(Condition::Not(Box::new(self.not()?)));
        }
        if matches!(self.k(), TokenKind::LParen) {
            self.bump();
            let x = self.condition()?;
            if !matches!(self.k(), TokenKind::RParen) {
                return self.fail("expected closing parenthesis");
            }
            self.bump();
            return Ok(x);
        }
        self.predicate()
    }
    fn predicate(&mut self) -> Result<Condition, FrontendError> {
        if self.word("CROSS_ABOVE") || self.word("CROSS_BELOW") {
            let direction = if self.take_word("CROSS_ABOVE") {
                CrossDirection::Above
            } else {
                self.bump();
                CrossDirection::Below
            };
            if !matches!(self.k(), TokenKind::LParen) {
                return self.fail("cross predicate requires parentheses");
            }
            self.bump();
            let left = self.numeric()?;
            if !matches!(self.k(), TokenKind::Comma) {
                return self.fail("cross predicate expects two arguments");
            }
            self.bump();
            let right = self.numeric()?;
            if !matches!(self.k(), TokenKind::RParen) {
                return self.fail("expected closing parenthesis");
            }
            self.bump();
            return Ok(Condition::Cross {
                direction,
                left,
                right,
            });
        }
        if self.take_word("IN_POSITION") {
            if matches!(self.k(), TokenKind::LParen) {
                self.bump();
                if !matches!(self.k(), TokenKind::RParen) {
                    return self.fail("in_position takes no arguments");
                }
                self.bump();
            }
            return Ok(Condition::InPosition);
        }
        if self.take_word("BETWEEN") {
            if !matches!(self.k(), TokenKind::LParen) {
                return self.fail("between requires parentheses");
            }
            self.bump();
            let a = self.time()?;
            if !matches!(self.k(), TokenKind::Comma) {
                return self.fail("between expects two times");
            }
            self.bump();
            let b = self.time()?;
            if !matches!(self.k(), TokenKind::RParen) {
                return self.fail("expected closing parenthesis");
            }
            self.bump();
            return Ok(Condition::Between { start: a, end: b });
        }
        let left = self.numeric()?;
        let op = match self.k() {
            TokenKind::Lt => CompareOp::Lt,
            TokenKind::Le => CompareOp::Le,
            TokenKind::Gt => CompareOp::Gt,
            TokenKind::Ge => CompareOp::Ge,
            TokenKind::Eq => CompareOp::Eq,
            TokenKind::Ne => CompareOp::Ne,
            _ => return self.fail("expected comparison operator"),
        };
        self.bump();
        let right = self.numeric()?;
        Ok(Condition::Compare { left, op, right })
    }
    fn time(&mut self) -> Result<Time, FrontendError> {
        let s = match self.bump().kind {
            TokenKind::Time(s) => s,
            _ => return self.fail("expected HH:MM time"),
        };
        let mut p = s.split(':');
        let h = p.next().unwrap_or("").parse::<u8>().unwrap_or(255);
        let m = p.next().unwrap_or("").parse::<u8>().unwrap_or(255);
        Ok(Time { hour: h, minute: m })
    }
    fn numeric(&mut self) -> Result<NumericExpr, FrontendError> {
        if matches!(
            self.k(),
            TokenKind::Number(_) | TokenKind::Minus | TokenKind::Plus
        ) {
            return Ok(NumericExpr::Literal(Decimal(self.number()?)));
        }
        let name = match self.bump().kind {
            TokenKind::Word(w) => w,
            _ => return self.fail("expected numeric expression"),
        };
        let price = match name.to_ascii_lowercase().as_str() {
            "close" => Some(PriceField::Close),
            "open" => Some(PriceField::Open),
            "high" => Some(PriceField::High),
            "low" => Some(PriceField::Low),
            "volume" => Some(PriceField::Volume),
            "prev_close" => Some(PriceField::PrevClose),
            "prev_open" => Some(PriceField::PrevOpen),
            "prev_high" => Some(PriceField::PrevHigh),
            "prev_low" => Some(PriceField::PrevLow),
            _ => None,
        };
        if let Some(p) = price {
            return Ok(NumericExpr::Price(p));
        }
        let kind = match name.to_ascii_lowercase().as_str() {
            "sma" => IndicatorKind::Sma,
            "ema" => IndicatorKind::Ema,
            "rsi" => IndicatorKind::Rsi,
            "rel_vol" => IndicatorKind::RelVol,
            "atr" => IndicatorKind::Atr,
            "vwap" => IndicatorKind::Vwap,
            "bb_upper" => IndicatorKind::BbUpper,
            "bb_lower" => IndicatorKind::BbLower,
            "bb_mid" => IndicatorKind::BbMid,
            _ => return self.fail("unknown price field or indicator"),
        };
        if !matches!(self.k(), TokenKind::LParen) {
            return self.fail("indicator requires parentheses");
        }
        self.bump();
        let period = match self.bump().kind {
            TokenKind::Number(n) => n.parse::<u32>().map_err(|_| FrontendError::Parse {
                code: "PARSE_PERIOD",
                message: "indicator period must be a positive integer".into(),
                line: self.span().line,
                column: self.span().column,
            })?,
            _ => return self.fail("indicator period must be an integer"),
        };
        let multiplier = if matches!(self.k(), TokenKind::Comma) {
            self.bump();
            Some(Decimal(self.number()?))
        } else {
            None
        };
        if matches!(
            kind,
            IndicatorKind::BbUpper | IndicatorKind::BbLower | IndicatorKind::BbMid
        ) && multiplier.is_none()
        {
            return self.fail("Bollinger indicator requires period and multiplier");
        }
        if !matches!(
            kind,
            IndicatorKind::BbUpper | IndicatorKind::BbLower | IndicatorKind::BbMid
        ) && multiplier.is_some()
        {
            return self.fail("this indicator accepts only a period");
        }
        if !matches!(self.k(), TokenKind::RParen) {
            return self.fail("expected closing indicator parenthesis");
        }
        self.bump();
        Ok(NumericExpr::Indicator(IndicatorCall {
            kind,
            period,
            multiplier,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    #[test]
    fn precedence_is_and_before_or_and_rules_are_ordered() {
        let src = "TRADE_IN ABC\nWHEN close > 1 AND open > 2 OR high > 3\nBUY 2\nWHEN low < 1\nSELL ALL\n";
        let ast = parse(&lex(src).unwrap()).unwrap();
        assert_eq!(ast.rules.len(), 2);
        assert_eq!(ast.rules[0].id, "rule_1");
        assert!(matches!(ast.rules[0].condition, Condition::Or(_, _)));
        assert!(matches!(ast.rules[1].action, Action::SellAll));
    }
    #[test]
    fn declarations_may_be_interleaved_and_duplicates_fail() {
        let src = "TRADE_IN ABC\nWHEN close > 1\nBUY 2\nSTOP_LOSS 5%\nWHEN low < 1\nSELL ALL\n";
        assert_eq!(
            parse(&lex(src).unwrap()).unwrap().stop_loss,
            Some(Decimal("5".into()))
        );
        assert!(parse(&lex("TRADE_IN ABC\nSTOP_LOSS 2%\nSTOP_LOSS 3%\n").unwrap()).is_err());
    }
    #[test]
    fn trailing_input_and_missing_rule_newline_are_rejected() {
        assert!(parse(&lex("TRADE_IN ABC\nWHEN close > 1 BUY 2\n").unwrap()).is_err());
        assert!(lex("TRADE_IN ABC\nWHEN close > 1\nBUY 2\n@\n").is_err());
    }
}
