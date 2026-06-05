use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    When,
    Buy,
    Sell,
    All,
    And,
    Or,
    Not,
    Between,
    InPosition,
    Ema,
    Ma,
    Rsi,
    RelVol,
    Atr,
    Vwap,
    BbUpper,
    BbLower,
    BbMid,
    Close,
    Open,
    High,
    Low,
    Volume,
    PrevClose,
    CrossAbove,
    CrossBelow,
    Lt,
    Gt,
    Lte,
    Gte,
    Eq,
    Neq,
    Number(f64),
    Integer(usize),
    TimeStr(String),
    LParen,
    RParen,
    Comma,
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LexError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

pub struct Lexer;

impl Lexer {
    pub fn tokenize(source: &str) -> Result<Vec<Token>, LexError> {
        let mut tokens = Vec::new();

        for (line_index, raw_line) in source.lines().enumerate() {
            let line_no = line_index + 1;
            let trimmed = raw_line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let chars: Vec<char> = raw_line.chars().collect();
            let mut pos = 0;
            while pos < chars.len() {
                let ch = chars[pos];
                let col = pos + 1;

                if ch.is_whitespace() {
                    pos += 1;
                    continue;
                }

                match ch {
                    '(' => {
                        tokens.push(token(TokenKind::LParen, line_no, col));
                        pos += 1;
                    }
                    ')' => {
                        tokens.push(token(TokenKind::RParen, line_no, col));
                        pos += 1;
                    }
                    ',' => {
                        tokens.push(token(TokenKind::Comma, line_no, col));
                        pos += 1;
                    }
                    '<' | '>' | '=' | '!' => {
                        let (kind, next_pos) = operator(&chars, pos).ok_or_else(|| LexError {
                            message: format!("unknown operator '{}'", ch),
                            line: line_no,
                            col,
                        })?;
                        tokens.push(token(kind, line_no, col));
                        pos = next_pos;
                    }
                    '0'..='9' => {
                        let (kind, next_pos) =
                            number_or_time(&chars, pos).map_err(|message| LexError {
                                message,
                                line: line_no,
                                col,
                            })?;
                        tokens.push(token(kind, line_no, col));
                        pos = next_pos;
                    }
                    _ if is_ident_start(ch) => {
                        let start = pos;
                        pos += 1;
                        while pos < chars.len() && is_ident_continue(chars[pos]) {
                            pos += 1;
                        }
                        let ident: String = chars[start..pos].iter().collect();
                        let kind = keyword(&ident).ok_or_else(|| LexError {
                            message: format!("unknown identifier '{}'", ident),
                            line: line_no,
                            col,
                        })?;
                        tokens.push(token(kind, line_no, col));
                    }
                    _ => {
                        return Err(LexError {
                            message: format!("unknown character '{}'", ch),
                            line: line_no,
                            col,
                        });
                    }
                }
            }

            tokens.push(token(TokenKind::Newline, line_no, raw_line.len() + 1));
        }

        tokens.push(token(TokenKind::Eof, source.lines().count() + 1, 1));
        Ok(tokens)
    }
}

fn token(kind: TokenKind, line: usize, col: usize) -> Token {
    Token { kind, line, col }
}

fn operator(chars: &[char], pos: usize) -> Option<(TokenKind, usize)> {
    let next = chars.get(pos + 1).copied();
    match (chars[pos], next) {
        ('<', Some('=')) => Some((TokenKind::Lte, pos + 2)),
        ('>', Some('=')) => Some((TokenKind::Gte, pos + 2)),
        ('=', Some('=')) => Some((TokenKind::Eq, pos + 2)),
        ('!', Some('=')) => Some((TokenKind::Neq, pos + 2)),
        ('<', _) => Some((TokenKind::Lt, pos + 1)),
        ('>', _) => Some((TokenKind::Gt, pos + 1)),
        _ => None,
    }
}

fn number_or_time(chars: &[char], pos: usize) -> Result<(TokenKind, usize), String> {
    if pos + 4 < chars.len()
        && chars[pos].is_ascii_digit()
        && chars[pos + 1].is_ascii_digit()
        && chars[pos + 2] == ':'
        && chars[pos + 3].is_ascii_digit()
        && chars[pos + 4].is_ascii_digit()
    {
        let text: String = chars[pos..=pos + 4].iter().collect();
        return Ok((TokenKind::TimeStr(text), pos + 5));
    }

    let start = pos;
    let mut end = pos;
    while end < chars.len() && chars[end].is_ascii_digit() {
        end += 1;
    }

    if end < chars.len() && chars[end] == '.' {
        end += 1;
        if end >= chars.len() || !chars[end].is_ascii_digit() {
            return Err("decimal number must include digits after '.'".to_string());
        }
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        let text: String = chars[start..end].iter().collect();
        let value = text
            .parse::<f64>()
            .map_err(|_| format!("invalid number '{}'", text))?;
        Ok((TokenKind::Number(value), end))
    } else {
        let text: String = chars[start..end].iter().collect();
        let value = text
            .parse::<usize>()
            .map_err(|_| format!("invalid integer '{}'", text))?;
        Ok((TokenKind::Integer(value), end))
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn keyword(ident: &str) -> Option<TokenKind> {
    match ident.to_ascii_lowercase().as_str() {
        "when" => Some(TokenKind::When),
        "buy" => Some(TokenKind::Buy),
        "sell" => Some(TokenKind::Sell),
        "all" => Some(TokenKind::All),
        "and" => Some(TokenKind::And),
        "or" => Some(TokenKind::Or),
        "not" => Some(TokenKind::Not),
        "between" => Some(TokenKind::Between),
        "in_position" => Some(TokenKind::InPosition),
        "ema" => Some(TokenKind::Ema),
        "ma" => Some(TokenKind::Ma),
        "rsi" => Some(TokenKind::Rsi),
        "rel_vol" => Some(TokenKind::RelVol),
        "atr" => Some(TokenKind::Atr),
        "vwap" => Some(TokenKind::Vwap),
        "bb_upper" => Some(TokenKind::BbUpper),
        "bb_lower" => Some(TokenKind::BbLower),
        "bb_mid" => Some(TokenKind::BbMid),
        "close" => Some(TokenKind::Close),
        "open" => Some(TokenKind::Open),
        "high" => Some(TokenKind::High),
        "low" => Some(TokenKind::Low),
        "volume" => Some(TokenKind::Volume),
        "prev_close" => Some(TokenKind::PrevClose),
        "cross_above" => Some(TokenKind::CrossAbove),
        "cross_below" => Some(TokenKind::CrossBelow),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLES: &str = r#"
# EMA crossover
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

    #[test]
    fn tokenizes_examples() {
        let tokens = Lexer::tokenize(EXAMPLES).unwrap();
        assert!(tokens
            .iter()
            .any(|token| token.kind == TokenKind::CrossAbove));
        assert_eq!(tokens.last().unwrap().kind, TokenKind::Eof);
    }

    #[test]
    fn tokenizes_simple_strategy() {
        let tokens = Lexer::tokenize("WHEN rsi(14) < 30\nBUY 5").unwrap();
        let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::When,
                TokenKind::Rsi,
                TokenKind::LParen,
                TokenKind::Integer(14),
                TokenKind::RParen,
                TokenKind::Lt,
                TokenKind::Integer(30),
                TokenKind::Newline,
                TokenKind::Buy,
                TokenKind::Integer(5),
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_sell_all() {
        let tokens = Lexer::tokenize("WHEN close > 100\nSELL ALL").unwrap();
        let kinds: Vec<_> = tokens.into_iter().map(|token| token.kind).collect();
        assert!(kinds.contains(&TokenKind::Sell));
        assert!(kinds.contains(&TokenKind::All));
    }

    #[test]
    fn tokenizes_rel_vol() {
        let tokens = Lexer::tokenize("WHEN rel_vol(20) > 2.0\nBUY 5").unwrap();
        assert!(tokens.iter().any(|token| token.kind == TokenKind::RelVol));
    }

    #[test]
    fn tokenizes_prev_close() {
        let tokens = Lexer::tokenize("WHEN prev_close < 100\nBUY 1").unwrap();
        assert!(tokens
            .iter()
            .any(|token| token.kind == TokenKind::PrevClose));
    }

    #[test]
    fn errors_on_unknown_character() {
        let err = Lexer::tokenize("WHEN close @ 10\nBUY 1").unwrap_err();
        assert_eq!(err.line, 1);
        assert_eq!(err.col, 12);
    }

    #[test]
    fn skips_comments_and_blank_lines() {
        let tokens = Lexer::tokenize("\n # comment\nWHEN close > 10\nBUY 1").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::When);
    }

    #[test]
    fn skips_comments() {
        let tokens = Lexer::tokenize("# this is a comment\nWHEN close > 100\nBUY 1").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::When);
    }

    #[test]
    fn skips_blank_lines() {
        let tokens = Lexer::tokenize("\n\nWHEN close > 100\nBUY 1").unwrap();
        assert_eq!(tokens[0].kind, TokenKind::When);
    }

    #[test]
    fn distinguishes_lte_from_lt() {
        let tokens = Lexer::tokenize("WHEN close <= 10\nBUY 1").unwrap();
        assert!(tokens.iter().any(|token| token.kind == TokenKind::Lte));
        assert!(!tokens.iter().any(|token| token.kind == TokenKind::Eq));
    }

    #[test]
    fn distinguishes_integer_and_number() {
        let tokens = Lexer::tokenize("WHEN close > 10.5\nBUY 1").unwrap();
        assert!(tokens
            .iter()
            .any(|token| token.kind == TokenKind::Number(10.5)));
        assert!(tokens
            .iter()
            .any(|token| token.kind == TokenKind::Integer(1)));
    }

    #[test]
    fn integer_vs_float() {
        let tokens = Lexer::tokenize("WHEN close > 105.5\nBUY 1").unwrap();
        assert!(tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Number(_))));
    }

    #[test]
    fn unknown_character_produces_lex_error() {
        assert!(Lexer::tokenize("WHEN close @ 100\nBUY 1").is_err());
    }

    #[test]
    fn cross_above_tokenizes_correctly() {
        let tokens = Lexer::tokenize("WHEN cross_above(ema(20), ema(50))\nBUY 10").unwrap();
        assert!(tokens
            .iter()
            .any(|token| token.kind == TokenKind::CrossAbove));
    }
}
