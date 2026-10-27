use crate::{diagnostics::FrontendError, source::Span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    Word(String),
    Number(String),
    Time(String),
    LParen,
    RParen,
    Comma,
    Percent,
    Plus,
    Minus,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    Newline,
    Eof,
}

pub const MAX_SOURCE_BYTES: usize = 1_048_576;
pub const MAX_TOKENS: usize = 100_000;
pub const MAX_TOKEN_BYTES: usize = 256;

pub fn lex(source: &str) -> Result<Vec<Token>, FrontendError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(err("LEX_LIMIT", "source exceeds 1 MiB limit", 1, 1));
    }
    let mut out = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;
    while i < source.len() {
        let c = source[i..].chars().next().expect("valid UTF-8 boundary");
        if c == '\r' && source[i..].starts_with("\r\n") {
            out.push(Token {
                kind: TokenKind::Newline,
                span: Span::new(i, i + 2, line, col),
            });
            i += 2;
            line += 1;
            col = 1;
            continue;
        }
        if c == '\r' {
            return Err(err(
                "LEX_NEWLINE",
                "lone carriage return is not a newline; use LF or CRLF",
                line,
                col,
            ));
        }
        if c == '\n' {
            out.push(Token {
                kind: TokenKind::Newline,
                span: Span::new(i, i + 1, line, col),
            });
            i += 1;
            line += 1;
            col = 1;
            continue;
        }
        if c.is_whitespace() {
            i += c.len_utf8();
            col += 1;
            continue;
        }
        let start = i;
        let start_col = col;
        let kind = match c {
            '(' => {
                i += 1;
                TokenKind::LParen
            }
            ')' => {
                i += 1;
                TokenKind::RParen
            }
            ',' => {
                i += 1;
                TokenKind::Comma
            }
            '%' => {
                i += 1;
                TokenKind::Percent
            }
            '+' => {
                i += 1;
                TokenKind::Plus
            }
            '-' => {
                i += 1;
                TokenKind::Minus
            }
            '<' | '>' | '=' | '!' => {
                i += 1;
                let next = source.get(i..).and_then(|s| s.chars().next());
                let pair = next == Some('=');
                if pair {
                    i += 1;
                }
                match (c, pair) {
                    ('<', true) => TokenKind::Le,
                    ('<', false) => TokenKind::Lt,
                    ('>', true) => TokenKind::Ge,
                    ('>', false) => TokenKind::Gt,
                    ('=', true) => TokenKind::Eq,
                    ('!', true) => TokenKind::Ne,
                    _ => return Err(err("LEX_OPERATOR", "invalid operator", line, start_col)),
                }
            }
            '.' if source[i..]
                .chars()
                .nth(1)
                .is_some_and(|x| x.is_ascii_digit()) =>
            {
                return Err(err(
                    "LEX_NUMBER",
                    "leading decimal point is not permitted",
                    line,
                    start_col,
                ));
            }
            '.' => {
                return Err(err(
                    "LEX_NUMBER",
                    "unexpected decimal point",
                    line,
                    start_col,
                ))
            }
            x if x.is_ascii_digit() => {
                while source
                    .get(i..)
                    .and_then(|s| s.chars().next())
                    .is_some_and(|x| x.is_ascii_digit())
                {
                    i += 1;
                }
                if source[i..].starts_with(':') {
                    i += 1;
                    let minute_start = i;
                    while source
                        .get(i..)
                        .and_then(|s| s.chars().next())
                        .is_some_and(|x| x.is_ascii_digit())
                    {
                        i += 1;
                    }
                    if i - minute_start != 2
                        || source
                            .get(i..)
                            .and_then(|s| s.chars().next())
                            .is_some_and(|x| x == ':' || x.is_ascii_alphanumeric() || x == '_')
                    {
                        return Err(err(
                            "LEX_TIME",
                            "time must have exactly two minute digits and a token boundary",
                            line,
                            start_col,
                        ));
                    }
                    TokenKind::Time(source[start..i].to_owned())
                } else {
                    if source[i..].starts_with('.') {
                        i += 1;
                        if !source
                            .get(i..)
                            .and_then(|s| s.chars().next())
                            .is_some_and(|x| x.is_ascii_digit())
                        {
                            return Err(err(
                                "LEX_NUMBER",
                                "decimal point must be followed by digits",
                                line,
                                start_col,
                            ));
                        }
                        while source
                            .get(i..)
                            .and_then(|s| s.chars().next())
                            .is_some_and(|x| x.is_ascii_digit())
                        {
                            i += 1;
                        }
                    }
                    if source
                        .get(i..)
                        .and_then(|s| s.chars().next())
                        .is_some_and(|x| x == '.' || x.is_alphabetic() || x == '_')
                    {
                        return Err(err(
                            "LEX_NUMBER",
                            "malformed or adjacent numeric literal",
                            line,
                            start_col,
                        ));
                    }
                    TokenKind::Number(source[start..i].to_owned())
                }
            }
            x if x.is_alphabetic() || x == '_' => {
                i += x.len_utf8();
                while source
                    .get(i..)
                    .and_then(|s| s.chars().next())
                    .is_some_and(|x| x.is_alphanumeric() || x == '_')
                {
                    i += source[i..].chars().next().unwrap().len_utf8();
                }
                if i - start > MAX_TOKEN_BYTES {
                    return Err(err(
                        "LEX_LIMIT",
                        "identifier exceeds 256-byte limit",
                        line,
                        start_col,
                    ));
                }
                TokenKind::Word(source[start..i].to_owned())
            }
            _ => {
                return Err(err(
                    "LEX_CHAR",
                    format!("unexpected character {c:?}"),
                    line,
                    start_col,
                ))
            }
        };
        if i - start > MAX_TOKEN_BYTES && matches!(kind, TokenKind::Number(_) | TokenKind::Time(_))
        {
            return Err(err(
                "LEX_LIMIT",
                "numeric or time literal exceeds 256-byte limit",
                line,
                start_col,
            ));
        }
        col += source[start..i].chars().count() as u32;
        if matches!(kind, TokenKind::Time(_)) {
            let tail = source.get(i..).and_then(|s| s.chars().next());
            if tail.is_some_and(|x| x.is_ascii_digit() || x == ':' || x.is_alphabetic() || x == '_')
            {
                return Err(err("LEX_TIME", "malformed time literal", line, start_col));
            }
        }
        if matches!(kind, TokenKind::Number(_))
            && source
                .get(i..)
                .and_then(|s| s.chars().next())
                .is_some_and(|x| x.is_ascii_alphanumeric() || x == '_')
        {
            return Err(err(
                "LEX_NUMBER",
                "malformed numeric suffix",
                line,
                start_col,
            ));
        }
        out.push(Token {
            kind,
            span: Span::new(start, i, line, start_col),
        });
        if out.len() > MAX_TOKENS {
            return Err(err("LEX_LIMIT", "token count exceeds 100,000", line, col));
        }
    }
    out.push(Token {
        kind: TokenKind::Eof,
        span: Span::new(i, i, line, col),
    });
    Ok(out)
}
fn err(code: &'static str, message: impl Into<String>, line: u32, column: u32) -> FrontendError {
    let span = Span::new(0, 0, line, column);
    FrontendError::Lex {
        code,
        message: message.into(),
        line,
        column,
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximal_operators_and_unicode_columns() {
        let t = lex("λ <= 2\r\n!=").unwrap();
        assert_eq!(t[0].span.column, 1);
        assert!(matches!(t[1].kind, TokenKind::Le));
        assert!(matches!(t[4].kind, TokenKind::Ne));
        assert_eq!(t[4].span.line, 2);
    }
    #[test]
    fn malformed_numbers_fail() {
        assert!(lex("12.3.4").is_err());
        assert!(lex("1abc").is_err());
        assert!(lex("1e5").is_err());
        assert!(lex(".5").is_err());
        assert!(lex("1..5").is_err());
        assert!(lex("10:3").is_err());
        assert!(lex("# comment").is_err());
        assert!(lex("\rTRADE_IN A").is_err());
    }
    #[test]
    fn source_positions_and_limits_are_explicit() {
        let tokens = lex("λ\tA\r\nB").unwrap();
        assert_eq!(tokens[1].span.column, 3);
        assert_eq!(tokens[3].span.line, 2);
        assert!(lex(&"a".repeat(MAX_TOKEN_BYTES + 1)).is_err());
    }
}
