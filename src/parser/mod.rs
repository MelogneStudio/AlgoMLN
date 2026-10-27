use crate::{
    ast::*,
    diagnostics::FrontendError,
    lexer::{Token, TokenKind},
    source::{NumericLiteral, Span},
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
    fn nesting(&self) -> usize {
        let mut depth = 0usize;
        let mut max = 0usize;
        for t in self.t[..self.i.min(self.t.len())].iter() {
            match t.kind {
                TokenKind::LParen => {
                    depth += 1;
                    max = max.max(depth)
                }
                TokenKind::RParen => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        max
    }
    fn check_depth(&self) -> Result<(), FrontendError> {
        if self.nesting() > 128 {
            let p = self.span();
            Err(FrontendError::Parse {
                code: "PARSE_DEPTH",
                message: "nesting exceeds 128 levels".into(),
                line: p.line,
                column: p.column,
                span: p,
            })
        } else {
            Ok(())
        }
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
            span: p,
        })
    }
    fn fail_at<T>(&self, msg: &str, span: Span) -> Result<T, FrontendError> {
        Err(FrontendError::Parse {
            code: "PARSE_SYNTAX",
            message: msg.into(),
            line: span.line,
            column: span.column,
            span,
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
                TokenKind::Word(w) | TokenKind::String(w) => {
                    if !matches!(self.k(), TokenKind::Newline) {
                        return self.fail(
                            "strategy name is one identifier and header must end at newline",
                        );
                    }
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
        if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
            return self.fail("TRADE_IN must end at newline");
        }
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
            if self.take_word("STRATEGY") {
                return self.fail("STRATEGY is only allowed before TRADE_IN");
            }
            if self.take_word("TRADE_IN") {
                return self.fail("duplicate TRADE_IN declaration");
            }
            if self.word("STOP_LOSS") {
                let declaration_span = self.bump().span;
                if stop_loss.is_some() {
                    return self.fail_at("duplicate STOP_LOSS", declaration_span);
                }
                stop_loss = Some(self.percent()?);
                if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
                    return self.fail("unexpected token after declaration");
                }
                self.newlines();
                continue;
            }
            if self.word("TAKE_PROFIT") {
                let declaration_span = self.bump().span;
                if take_profit.is_some() {
                    return self.fail_at("duplicate TAKE_PROFIT", declaration_span);
                }
                take_profit = Some(self.percent()?);
                if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
                    return self.fail("unexpected token after declaration");
                }
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
                if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
                    return self.fail("unexpected token after risk declaration");
                }
                self.newlines();
                continue;
            }
            if self.take_word("WHEN") {
                let start = self.t[self.i - 1].span;
                let condition_start = self.span();
                let condition = self.condition()?;
                let condition_end = self.t[self.i - 1].span;
                if !matches!(self.k(), TokenKind::Newline) {
                    return self.fail("rule condition must end at newline");
                }
                self.newlines();
                if matches!(self.k(), TokenKind::Eof) {
                    return self.fail("expected action after condition");
                }
                let action_start = self.span();
                let action = self.action()?;
                let end = self.t[self.i - 1].span;
                let n = rules.len() + 1;
                rules.push(Rule {
                    id: format!("rule_{n}"),
                    condition,
                    action,
                    span: Span::new(start.start, end.end, start.line, start.column),
                    condition_span: Span::new(
                        condition_start.start,
                        condition_end.end,
                        condition_start.line,
                        condition_start.column,
                    ),
                    action_span: Span::new(
                        action_start.start,
                        end.end,
                        action_start.line,
                        action_start.column,
                    ),
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
                TokenKind::Word(w) if !w.eq_ignore_ascii_case("INDEX") => xs.push(w),
                TokenKind::String(w) => xs.push(w),
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
        let token_start = self.span();
        let n = self.number()?;
        if !matches!(self.k(), TokenKind::Percent) {
            return self.fail("expected %");
        };
        if self.i > 0 && self.t[self.i - 1].span.end != self.span().start {
            return self.fail("% must immediately follow number");
        }
        self.bump();
        let end = self.t[self.i - 1].span.end;
        NumericLiteral::parse(
            n,
            Span::new(token_start.start, end, token_start.line, token_start.column),
        )
        .ok_or_else(|| FrontendError::Parse {
            code: "PARSE_NUMBER",
            message: "invalid decimal".into(),
            line: token_start.line,
            column: token_start.column,
            span: token_start,
        })
    }
    fn decimal(&self, n: String) -> Result<Decimal, FrontendError> {
        let span = self.t[self.i.saturating_sub(1)].span;
        NumericLiteral::parse(n, span).ok_or_else(|| FrontendError::Parse {
            code: "PARSE_NUMBER",
            message: "invalid decimal".into(),
            line: span.line,
            column: span.column,
            span,
        })
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
            span: self.span(),
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
            if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
                return self.fail("SELL ALL cannot have an amount or suffix");
            }
            return Ok(Action::SellAll);
        }
        let n = self.number()?;
        let number_end = self.span().start;
        let amount = if self.take_word("WORTH") {
            if n.starts_with(['-', '+']) {
                return self.fail("WORTH requires an unsigned decimal amount");
            }
            TradeAmount::Worth(self.decimal(n)?)
        } else if matches!(self.k(), TokenKind::Percent) {
            if number_end != self.span().start {
                return self.fail("% must immediately follow number");
            }
            self.bump();
            if buy {
                TradeAmount::BuyCapitalPercentage(self.decimal(n)?)
            } else {
                TradeAmount::SellPositionPercentage(self.decimal(n)?)
            }
        } else {
            let q = n.parse::<u64>().map_err(|_| FrontendError::Parse {
                code: "PARSE_INTEGER",
                message: "quantity must be a nonnegative integer".into(),
                line: self.span().line,
                column: self.span().column,
                span: self.span(),
            })?;
            TradeAmount::Quantity(q)
        };
        if !matches!(self.k(), TokenKind::Newline | TokenKind::Eof) {
            return self.fail("unexpected token after action");
        }
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
            let mut count = 1;
            while self.take_word("NOT") {
                count += 1;
            }
            let mut c = self.not()?;
            for _ in 0..count {
                c = Condition::Not(Box::new(c));
            }
            return Ok(c);
        }
        if matches!(self.k(), TokenKind::LParen) {
            self.bump();
            self.check_depth()?;
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
            self.check_depth()?;
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
            if !matches!(self.k(), TokenKind::LParen) {
                return self.fail("in_position requires parentheses");
            }
            self.bump();
            if !matches!(self.k(), TokenKind::RParen) {
                return self.fail("in_position takes no arguments");
            }
            self.bump();
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
        if matches!(
            self.k(),
            TokenKind::Lt
                | TokenKind::Le
                | TokenKind::Gt
                | TokenKind::Ge
                | TokenKind::Eq
                | TokenKind::Ne
        ) {
            return self.fail("chained comparisons are not supported");
        }
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
            let n = self.number()?;
            return Ok(NumericExpr::Literal(self.decimal(n)?));
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
        let period_token = self.bump();
        let period = match period_token.kind {
            TokenKind::Number(n) if !n.contains('.') => {
                n.parse::<u32>().map_err(|_| FrontendError::Parse {
                    code: "PARSE_PERIOD",
                    message: "indicator period must be a positive integer".into(),
                    line: self.span().line,
                    column: self.span().column,
                    span: self.span(),
                })?
            }
            _ => return self.fail("indicator period must be an integer"),
        };
        let multiplier = if matches!(self.k(), TokenKind::Comma) {
            self.bump();
            let n = self.number()?;
            Some(self.decimal(n)?)
        } else {
            None
        };
        let multiplier = if matches!(
            kind,
            IndicatorKind::BbUpper | IndicatorKind::BbLower | IndicatorKind::BbMid
        ) && multiplier.is_none()
        {
            Some(NumericLiteral::parse("2.0".into(), self.span()).unwrap())
        } else {
            multiplier
        };
        if !matches!(
            kind,
            IndicatorKind::BbUpper | IndicatorKind::BbLower | IndicatorKind::BbMid
        ) && multiplier.is_some()
        {
            return self.fail("this indicator accepts only a period");
        }
        if multiplier.is_some() && matches!(self.k(), TokenKind::Comma) {
            return self.fail("too many indicator arguments");
        }
        if !matches!(self.k(), TokenKind::RParen) {
            return self.fail("expected closing indicator parenthesis");
        }
        self.bump();
        if period == 0 {
            return self.fail("indicator period must be positive");
        }
        if let Some(m) = &multiplier {
            if !m.cmp_integer("0").is_some_and(|v| v.is_gt()) {
                return self.fail("Bollinger multiplier must be positive");
            }
        }
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
            Some(NumericLiteral::parse("5".into(), Span::new(44, 46, 4, 11)).unwrap())
        );
        assert!(parse(&lex("TRADE_IN ABC\nSTOP_LOSS 2%\nSTOP_LOSS 3%\n").unwrap()).is_err());
    }
    #[test]
    fn trailing_input_and_missing_rule_newline_are_rejected() {
        assert!(parse(&lex("TRADE_IN ABC\nWHEN close > 1 BUY 2\n").unwrap()).is_err());
        assert!(lex("TRADE_IN ABC\nWHEN close > 1\nBUY 2\n@\n").is_err());
    }
    #[test]
    fn headers_actions_suffixes_and_eof_are_strict() {
        for src in [
            "TRADE_IN ABC\nTRADE_IN XYZ\n",
            "TRADE_IN ABC\nSTRATEGY late\n",
            "TRADE_IN ABC\nSTOP_LOSS 2 %\n",
            "TRADE_IN ABC\nWHEN close > 1\nBUY 1%WORTH\n",
            "TRADE_IN ABC\nWHEN close > 1\nSELL ALL 1\n",
            "TRADE_IN ABC\nWHEN 1 < close < 10\nBUY 1\n",
            "TRADE_IN ABC,\n",
            "TRADE_IN ABC\nWHEN close > 1\nBUY 1\nGARBAGE\n",
        ] {
            assert!(compile_err(src), "{src}");
        }
        assert!(!compile_err(
            "STRATEGY Momentum\nTRADE_IN INDEX NIFTY\n\nWHEN SMA(14) > 1\n\nBUY 00010\n"
        ));
    }
    #[test]
    fn quoted_symbols_fractional_worth_and_price_crossovers_parse() {
        let s = crate::compile("TRADE_IN \"M&M\", \"BAJAJ-AUTO\", \"3MINDIA\", \"360ONE\"\nWHEN CROSS_ABOVE(close, ema(20))\nBUY 10.50 WORTH\nWHEN close > 0\nSELL 12.5%\n").unwrap();
        assert_eq!(
            s.trade_in,
            TradeIn::Symbols(vec![
                "M&M".into(),
                "BAJAJ-AUTO".into(),
                "3MINDIA".into(),
                "360ONE".into()
            ])
        );
        assert!(matches!(s.rules[0].condition, Condition::Cross { .. }));
        assert!(matches!(
            s.rules[0].action,
            Action::Buy(TradeAmount::Worth(_))
        ));
        assert!(matches!(
            s.rules[1].action,
            Action::Sell(TradeAmount::SellPositionPercentage(_))
        ));
    }
    fn compile_err(s: &str) -> bool {
        let t = lex(s);
        match t {
            Err(_) => true,
            Ok(t) => parse(&t).is_err(),
        }
    }
}
