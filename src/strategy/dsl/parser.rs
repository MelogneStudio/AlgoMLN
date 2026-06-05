use chrono::NaiveTime;
use serde::{Deserialize, Serialize};

use super::ast::{
    ActionNode, CompareOp, ConditionNode, ExprNode, IndicatorCall, IndicatorKind, PriceField,
    RuleNode, StrategyNode,
};
use super::lexer::{Token, TokenKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse(&mut self) -> Result<StrategyNode, ParseError> {
        let mut rules = Vec::new();
        self.skip_newlines();

        while !self.is_at_end() {
            let mut rule = self.parse_rule()?;
            rule.id = format!("rule_{}", rules.len());
            rules.push(rule);
            self.skip_newlines();
        }

        Ok(StrategyNode {
            name: "Untitled Strategy".to_string(),
            rules,
        })
    }

    fn parse_rule(&mut self) -> Result<RuleNode, ParseError> {
        self.expect_simple(TokenKind::When)?;
        let condition = self.parse_condition()?;
        self.expect_simple(TokenKind::Newline)?;
        let action = self.parse_action()?;

        Ok(RuleNode {
            id: String::new(),
            condition,
            action,
        })
    }

    fn parse_condition(&mut self) -> Result<ConditionNode, ParseError> {
        let mut node = self.parse_primary_condition()?;

        loop {
            if self.matches_simple(TokenKind::And) {
                let right = self.parse_primary_condition()?;
                node = ConditionNode::And(Box::new(node), Box::new(right));
            } else if self.matches_simple(TokenKind::Or) {
                let right = self.parse_primary_condition()?;
                node = ConditionNode::Or(Box::new(node), Box::new(right));
            } else {
                break;
            }
        }

        Ok(node)
    }

    fn parse_primary_condition(&mut self) -> Result<ConditionNode, ParseError> {
        match &self.peek().kind {
            TokenKind::CrossAbove | TokenKind::CrossBelow => self.parse_cross(),
            TokenKind::Not => self.parse_not(),
            TokenKind::InPosition => self.parse_position_expr(),
            TokenKind::Between => self.parse_time_window(),
            _ => self.parse_comparison(),
        }
    }

    fn parse_comparison(&mut self) -> Result<ConditionNode, ParseError> {
        let left = self.parse_expr()?;
        let op = self.parse_compare_op()?;
        let right = self.parse_expr()?;
        Ok(ConditionNode::Comparison { left, op, right })
    }

    fn parse_cross(&mut self) -> Result<ConditionNode, ParseError> {
        let is_above = self.matches_simple(TokenKind::CrossAbove);
        if !is_above {
            self.expect_simple(TokenKind::CrossBelow)?;
        }
        self.expect_simple(TokenKind::LParen)?;
        let fast = self.parse_expr()?;
        self.expect_simple(TokenKind::Comma)?;
        let slow = self.parse_expr()?;
        self.expect_simple(TokenKind::RParen)?;

        if is_above {
            Ok(ConditionNode::CrossAbove { fast, slow })
        } else {
            Ok(ConditionNode::CrossBelow { fast, slow })
        }
    }

    fn parse_not(&mut self) -> Result<ConditionNode, ParseError> {
        self.expect_simple(TokenKind::Not)?;
        self.expect_simple(TokenKind::LParen)?;
        let inner = self.parse_condition()?;
        self.expect_simple(TokenKind::RParen)?;
        Ok(ConditionNode::Not(Box::new(inner)))
    }

    fn parse_position_expr(&mut self) -> Result<ConditionNode, ParseError> {
        self.expect_simple(TokenKind::InPosition)?;
        self.expect_simple(TokenKind::LParen)?;
        self.expect_simple(TokenKind::RParen)?;
        Ok(ConditionNode::InPosition)
    }

    fn parse_time_window(&mut self) -> Result<ConditionNode, ParseError> {
        self.expect_simple(TokenKind::Between)?;
        self.expect_simple(TokenKind::LParen)?;
        let start = self.parse_time()?;
        self.expect_simple(TokenKind::Comma)?;
        let end = self.parse_time()?;
        self.expect_simple(TokenKind::RParen)?;
        Ok(ConditionNode::TimeWindow { start, end })
    }

    fn parse_expr(&mut self) -> Result<ExprNode, ParseError> {
        match self.peek().kind.clone() {
            TokenKind::Integer(value) => {
                self.advance();
                Ok(ExprNode::Literal(value as f64))
            }
            TokenKind::Number(value) => {
                self.advance();
                Ok(ExprNode::Literal(value))
            }
            TokenKind::Close => {
                self.advance();
                Ok(ExprNode::PriceField(PriceField::Close))
            }
            TokenKind::Open => {
                self.advance();
                Ok(ExprNode::PriceField(PriceField::Open))
            }
            TokenKind::High => {
                self.advance();
                Ok(ExprNode::PriceField(PriceField::High))
            }
            TokenKind::Low => {
                self.advance();
                Ok(ExprNode::PriceField(PriceField::Low))
            }
            TokenKind::Volume => {
                self.advance();
                Ok(ExprNode::PriceField(PriceField::Volume))
            }
            TokenKind::PrevClose => {
                self.advance();
                Ok(ExprNode::PriceField(PriceField::PrevClose))
            }
            TokenKind::Ema => self.parse_indicator(IndicatorKind::Ema),
            TokenKind::Ma => self.parse_indicator(IndicatorKind::Ma),
            TokenKind::Rsi => self.parse_indicator(IndicatorKind::Rsi),
            TokenKind::RelVol => self.parse_indicator(IndicatorKind::RelVol),
            TokenKind::Atr => self.parse_indicator(IndicatorKind::Atr),
            TokenKind::Vwap => self.parse_indicator(IndicatorKind::Vwap),
            TokenKind::BbUpper => self.parse_indicator(IndicatorKind::BbUpper),
            TokenKind::BbLower => self.parse_indicator(IndicatorKind::BbLower),
            TokenKind::BbMid => self.parse_indicator(IndicatorKind::BbMid),
            _ => self.error_here("expected expression"),
        }
    }

    fn parse_indicator(&mut self, kind: IndicatorKind) -> Result<ExprNode, ParseError> {
        self.advance();
        self.expect_simple(TokenKind::LParen)?;
        let period = match self.advance().kind.clone() {
            TokenKind::Integer(value) => value,
            _ => return self.error_previous("expected integer indicator period"),
        };
        self.expect_simple(TokenKind::RParen)?;
        Ok(ExprNode::Indicator(IndicatorCall { kind, period }))
    }

    fn parse_action(&mut self) -> Result<ActionNode, ParseError> {
        if self.matches_simple(TokenKind::Buy) {
            let quantity = self.parse_quantity()?;
            return Ok(ActionNode::Buy { quantity });
        }

        if self.matches_simple(TokenKind::Sell) {
            if self.matches_simple(TokenKind::All) {
                return Ok(ActionNode::SellAll);
            }
            let quantity = self.parse_quantity()?;
            return Ok(ActionNode::Sell { quantity });
        }

        self.error_here("expected BUY or SELL action")
    }

    fn parse_compare_op(&mut self) -> Result<CompareOp, ParseError> {
        let token = self.advance();
        match token.kind {
            TokenKind::Lt => Ok(CompareOp::Lt),
            TokenKind::Gt => Ok(CompareOp::Gt),
            TokenKind::Lte => Ok(CompareOp::Lte),
            TokenKind::Gte => Ok(CompareOp::Gte),
            TokenKind::Eq => Ok(CompareOp::Eq),
            TokenKind::Neq => Ok(CompareOp::Neq),
            _ => self.error_previous("expected comparison operator"),
        }
    }

    fn parse_quantity(&mut self) -> Result<usize, ParseError> {
        match self.advance().kind.clone() {
            TokenKind::Integer(value) => Ok(value),
            _ => self.error_previous("expected integer quantity"),
        }
    }

    fn parse_time(&mut self) -> Result<NaiveTime, ParseError> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::TimeStr(value) => {
                NaiveTime::parse_from_str(&value, "%H:%M").map_err(|_| ParseError {
                    message: format!("invalid time '{}'", value),
                    line: token.line,
                    col: token.col,
                })
            }
            _ => Err(ParseError {
                message: "expected HH:MM time".to_string(),
                line: token.line,
                col: token.col,
            }),
        }
    }

    fn skip_newlines(&mut self) {
        while self.matches_simple(TokenKind::Newline) {}
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.pos += 1;
        }
        &self.tokens[self.pos - 1]
    }

    fn expect_simple(&mut self, kind: TokenKind) -> Result<&Token, ParseError> {
        if same_variant(&self.peek().kind, &kind) {
            Ok(self.advance())
        } else {
            self.error_here(&format!("expected {}", token_name(&kind)))
        }
    }

    fn matches_simple(&mut self, kind: TokenKind) -> bool {
        if same_variant(&self.peek().kind, &kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn error_here<T>(&self, message: &str) -> Result<T, ParseError> {
        let token = self.peek();
        Err(ParseError {
            message: message.to_string(),
            line: token.line,
            col: token.col,
        })
    }

    fn error_previous<T>(&self, message: &str) -> Result<T, ParseError> {
        let token = &self.tokens[self.pos.saturating_sub(1)];
        Err(ParseError {
            message: message.to_string(),
            line: token.line,
            col: token.col,
        })
    }
}

fn same_variant(left: &TokenKind, right: &TokenKind) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

fn token_name(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::When => "WHEN",
        TokenKind::Newline => "newline",
        TokenKind::LParen => "'('",
        TokenKind::RParen => "')'",
        TokenKind::Comma => "','",
        _ => "token",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::dsl::lexer::Lexer;

    fn parse(source: &str) -> StrategyNode {
        let tokens = Lexer::tokenize(source).unwrap();
        Parser::new(tokens).parse().unwrap()
    }

    #[test]
    fn parses_examples() {
        let source = r#"
WHEN cross_above(ema(20), ema(50))
BUY 10

WHEN cross_below(ema(20), ema(50))
SELL ALL

WHEN rsi(14) < 30
BUY 5

WHEN rsi(14) > 70
SELL ALL

WHEN close > bb_upper(20)
SELL 10

WHEN close < bb_lower(20)
BUY 10

WHEN ema(9) > ema(21) AND rsi(14) < 60
BUY 5

WHEN cross_above(ema(20), ema(50)) AND NOT (in_position())
BUY 10
"#;
        assert_eq!(parse(source).rules.len(), 8);
    }

    #[test]
    fn parses_simple_rsi_rule() {
        let strategy = parse("WHEN rsi(14) < 30\nBUY 5");
        assert_eq!(strategy.rules.len(), 1);
        assert_eq!(strategy.rules[0].id, "rule_0");
        assert!(matches!(
            strategy.rules[0].action,
            ActionNode::Buy { quantity: 5 }
        ));
    }

    #[test]
    fn assigns_rule_ids_in_order() {
        let strategy = parse("WHEN close > 10\nBUY 1\nWHEN close < 5\nSELL 1");
        assert_eq!(strategy.rules[0].id, "rule_0");
        assert_eq!(strategy.rules[1].id, "rule_1");
    }

    #[test]
    fn assigns_sequential_rule_ids() {
        let strategy = parse("WHEN rsi(14) < 30\nBUY 5\n\nWHEN rsi(14) > 70\nSELL ALL");
        assert_eq!(strategy.rules[0].id, "rule_0");
        assert_eq!(strategy.rules[1].id, "rule_1");
    }

    #[test]
    fn errors_on_missing_action() {
        let tokens = Lexer::tokenize("WHEN close > 10").unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.message.contains("action"));
    }

    #[test]
    fn parse_error_on_missing_action() {
        let tokens = Lexer::tokenize("WHEN rsi(14) < 30").unwrap();
        assert!(Parser::new(tokens).parse().is_err());
    }

    #[test]
    fn parse_error_on_incomplete_comparison() {
        let tokens = Lexer::tokenize("WHEN rsi(14) <\nBUY 5").unwrap();
        assert!(Parser::new(tokens).parse().is_err());
    }

    #[test]
    fn errors_on_malformed_condition() {
        let tokens = Lexer::tokenize("WHEN close\nBUY 1").unwrap();
        let err = Parser::new(tokens).parse().unwrap_err();
        assert!(err.message.contains("comparison"));
    }

    #[test]
    fn nests_and_conditions() {
        let strategy = parse("WHEN close > 10 AND rsi(14) < 60\nBUY 1");
        assert!(matches!(
            strategy.rules[0].condition,
            ConditionNode::And(_, _)
        ));
    }

    #[test]
    fn parses_and_condition() {
        let strategy = parse("WHEN ema(9) > ema(21) AND rsi(14) < 60\nBUY 5");
        assert!(matches!(
            strategy.rules[0].condition,
            ConditionNode::And(_, _)
        ));
    }

    #[test]
    fn parses_not_condition() {
        let strategy = parse("WHEN NOT (in_position())\nBUY 1");
        assert!(matches!(strategy.rules[0].condition, ConditionNode::Not(_)));
    }

    #[test]
    fn parses_not_wrapping_comparison() {
        let strategy = parse("WHEN NOT (rsi(14) > 70)\nBUY 5");
        assert!(matches!(strategy.rules[0].condition, ConditionNode::Not(_)));
    }

    #[test]
    fn parses_sell_all_distinctly() {
        let strategy = parse("WHEN close > 10\nSELL ALL");
        assert!(matches!(strategy.rules[0].action, ActionNode::SellAll));
    }

    #[test]
    fn sell_all_is_sell_all_not_sell_quantity() {
        let strategy = parse("WHEN close > 100\nSELL ALL");
        assert!(matches!(strategy.rules[0].action, ActionNode::SellAll));
    }

    #[test]
    fn parses_cross_above() {
        let strategy = parse("WHEN cross_above(ema(20), ema(50))\nBUY 10");
        assert!(matches!(
            strategy.rules[0].condition,
            ConditionNode::CrossAbove { .. }
        ));
    }

    #[test]
    fn parses_rel_vol_indicator() {
        let node = parse("WHEN rel_vol(20) > 2.0\nBUY 5");
        assert_eq!(node.rules.len(), 1);
        match &node.rules[0].condition {
            ConditionNode::Comparison { left, .. } => {
                assert!(matches!(
                    left,
                    ExprNode::Indicator(IndicatorCall {
                        kind: IndicatorKind::RelVol,
                        period: 20
                    })
                ));
            }
            _ => panic!("expected comparison"),
        }
    }

    #[test]
    fn parses_prev_close() {
        let node = parse("WHEN prev_close < 100\nBUY 1");
        match &node.rules[0].condition {
            ConditionNode::Comparison { left, .. } => {
                assert!(matches!(left, ExprNode::PriceField(PriceField::PrevClose)));
            }
            _ => panic!("expected comparison"),
        }
    }

    #[test]
    fn deterministic_ids_across_reparses() {
        let source = "WHEN rsi(14) < 30\nBUY 5";
        let first = parse(source);
        let second = parse(source);
        assert_eq!(first.rules[0].id, second.rules[0].id);
    }

    #[test]
    fn print_ast_for_inspection() {
        let tokens = Lexer::tokenize("WHEN rsi(14) < 30\nBUY 5").unwrap();
        let strategy = Parser::new(tokens).parse().unwrap();
        println!("{:#?}", strategy);
    }

    #[test]
    fn print_parse_error_for_inspection() {
        let tokens = Lexer::tokenize("WHEN rsi(14) <\nBUY 5").unwrap();
        let result = Parser::new(tokens).parse();
        println!("{:#?}", result);
        assert!(result.is_err());
    }
}
