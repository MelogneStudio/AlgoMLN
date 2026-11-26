use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;

use crate::diag::{finalize_diagnostics, Diagnostic, MAX_DIAGNOSTICS};
use crate::span::Span;

/// Maximum allowed source size in bytes (1 MiB).
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;

/// Timeframe unit suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TfUnit {
    Minute,
    Hour,
    Day,
    Week,
}

impl fmt::Display for TfUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TfUnit::Minute => f.write_str("m"),
            TfUnit::Hour => f.write_str("h"),
            TfUnit::Day => f.write_str("d"),
            TfUnit::Week => f.write_str("w"),
        }
    }
}

/// Hard keywords in LangMelogne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kw {
    Def,
    Return,
    And,
    Or,
    Not,
    If,
    Else,
    True,
    False,
}

impl Kw {
    pub fn as_str(self) -> &'static str {
        match self {
            Kw::Def => "def",
            Kw::Return => "return",
            Kw::And => "and",
            Kw::Or => "or",
            Kw::Not => "not",
            Kw::If => "if",
            Kw::Else => "else",
            Kw::True => "true",
            Kw::False => "false",
        }
    }
}

/// Augmented assignment operators (lexed as a single token, rejected by the parser).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AugOp {
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
}

impl AugOp {
    pub fn as_str(self) -> &'static str {
        match self {
            AugOp::AddAssign => "+=",
            AugOp::SubAssign => "-=",
            AugOp::MulAssign => "*=",
            AugOp::DivAssign => "/=",
        }
    }
}

/// Token variants produced by [`lex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Num { value: Decimal, pct: bool },
    Timeframe { n: u32, unit: TfUnit },
    Str(String),
    Kw(Kw),
    Aug(AugOp),
    Plus,
    Minus,
    Star,
    Slash,
    StarStar,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Eq,
    At,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Dot,
    Semi,
    Err,
    Nl,
    Ind,
    Ded,
    Eof,
}

/// A lexed token with its byte [`Span`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// Output of [`lex`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexOutput {
    pub tokens: Vec<Token>,
    pub diags: Vec<Diagnostic>,
}

/// Formats a token slice in the canonical test representation.
pub fn tokens_str(tokens: &[Token]) -> String {
    let mut out = String::new();
    for (i, tok) in tokens.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        match &tok.kind {
            TokenKind::Ident(s) => out.push_str(&format!("ID({s})")),
            TokenKind::Num { value, pct } => {
                let norm = value.normalize();
                if *pct {
                    out.push_str(&format!("NUM({norm}%)"));
                } else {
                    out.push_str(&format!("NUM({norm})"));
                }
            }
            TokenKind::Timeframe { n, unit } => out.push_str(&format!("TF({n}{unit})")),
            TokenKind::Str(s) => out.push_str(&format!("STR({s})")),
            TokenKind::Kw(k) => out.push_str(&format!("KW({})", k.as_str())),
            TokenKind::Aug(a) => out.push_str(&format!("AUG({})", a.as_str())),
            TokenKind::Plus => out.push('+'),
            TokenKind::Minus => out.push('-'),
            TokenKind::Star => out.push('*'),
            TokenKind::Slash => out.push('/'),
            TokenKind::StarStar => out.push_str("**"),
            TokenKind::EqEq => out.push_str("=="),
            TokenKind::BangEq => out.push_str("!="),
            TokenKind::Lt => out.push('<'),
            TokenKind::LtEq => out.push_str("<="),
            TokenKind::Gt => out.push('>'),
            TokenKind::GtEq => out.push_str(">="),
            TokenKind::Eq => out.push('='),
            TokenKind::At => out.push('@'),
            TokenKind::LParen => out.push('('),
            TokenKind::RParen => out.push(')'),
            TokenKind::LBracket => out.push('['),
            TokenKind::RBracket => out.push(']'),
            TokenKind::Comma => out.push(','),
            TokenKind::Colon => out.push(':'),
            TokenKind::Dot => out.push('.'),
            TokenKind::Semi => out.push(';'),
            TokenKind::Err => out.push_str("ERR"),
            TokenKind::Nl => out.push_str("NL"),
            TokenKind::Ind => out.push_str("IND"),
            TokenKind::Ded => out.push_str("DED"),
            TokenKind::Eof => out.push_str("EOF"),
        }
    }
    out
}

/// Tokenizes `src` into tokens and diagnostics. Total: never panics or hangs.
pub fn lex(src: &str) -> LexOutput {
    let eof_offset = src.len().min(u32::MAX as usize) as u32;
    if src.len() > MAX_SOURCE_BYTES {
        let diag = Diagnostic::error(
            "E0017",
            format!(
                "source is {} bytes, exceeding the 1 MiB ({} byte) limit",
                src.len(),
                MAX_SOURCE_BYTES
            ),
            Span::new(0, 0),
        );
        return LexOutput {
            tokens: vec![Token {
                kind: TokenKind::Eof,
                span: Span::empty(eof_offset),
            }],
            diags: vec![diag],
        };
    }

    let mut lexer = Lexer::new(src);
    lexer.run();
    finalize_diagnostics(&mut lexer.diags, eof_offset);
    LexOutput {
        tokens: lexer.tokens,
        diags: lexer.diags,
    }
}

struct Lexer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
    indent_stack: Vec<usize>,
    bracket_stack: Vec<(u8, Span)>,
    at_line_start: bool,
    seen_first_stmt: bool,
    emitted_py_kw_on_line: bool,
    line_has_tokens: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        let pos = if src.starts_with('\u{FEFF}') { 3 } else { 0 };
        Self {
            src,
            bytes: src.as_bytes(),
            pos,
            tokens: Vec::new(),
            diags: Vec::new(),
            indent_stack: vec![0],
            bracket_stack: Vec::new(),
            at_line_start: true,
            seen_first_stmt: false,
            emitted_py_kw_on_line: false,
            line_has_tokens: false,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.pos + offset).copied()
    }

    fn push_token(&mut self, kind: TokenKind, start: usize, end: usize) {
        self.tokens.push(Token {
            kind,
            span: Span::new(start as u32, end as u32),
        });
        self.line_has_tokens = true;
    }

    fn handle_line_start(&mut self) {
        let indent_start = self.pos;
        let mut spaces = 0usize;
        let mut tab_width = 0usize;
        let mut first_tab: Option<Span> = None;

        while let Some(b) = self.peek() {
            if b == b' ' {
                spaces += 1;
                tab_width += 1;
                self.pos += 1;
            } else if b == b'\t' {
                if first_tab.is_none() {
                    first_tab = Some(Span::new(self.pos as u32, (self.pos + 1) as u32));
                }
                tab_width += 4;
                self.pos += 1;
            } else {
                break;
            }
        }

        // Blank lines and comment-only lines produce no tokens and do not affect indentation.
        match self.peek() {
            None | Some(b'\n' | b'#') => {
                self.at_line_start = false;
                return;
            }
            Some(b'\r') if self.peek_at(1) == Some(b'\n') => {
                self.at_line_start = false;
                return;
            }
            _ => {}
        }

        self.at_line_start = false;

        let indent_span = Span::new(indent_start as u32, self.pos as u32);
        let had_tab = first_tab.is_some();
        if let Some(tab_span) = first_tab {
            self.diags.push(Diagnostic::error(
                "E0010",
                "tabs are not allowed in indentation; use 4 spaces per level",
                tab_span,
            ));
        }

        let effective = if had_tab { tab_width } else { spaces };

        if !self.seen_first_stmt {
            self.seen_first_stmt = true;
            if effective > 0 && !had_tab {
                self.diags.push(Diagnostic::error(
                    "E0013",
                    "the first statement line cannot be indented",
                    indent_span,
                ));
            }
            return;
        }

        let current = *self.indent_stack.last().unwrap_or(&0);
        if effective == current {
            return;
        }

        if effective > current {
            if !had_tab {
                let diff = effective - current;
                if diff != 4 && diff % 4 == 0 {
                    self.diags.push(Diagnostic::error(
                        "E0011",
                        format!(
                            "indentation jumped from {current} to {effective} spaces (expected +4)"
                        ),
                        indent_span,
                    ));
                } else if diff != 4 {
                    self.diags.push(Diagnostic::error(
                        "E0012",
                        format!("indentation of {effective} spaces is not a multiple of 4"),
                        indent_span,
                    ));
                }
            }
            self.indent_stack.push(effective);
            self.tokens.push(Token {
                kind: TokenKind::Ind,
                span: indent_span,
            });
        } else {
            while let Some(&top) = self.indent_stack.last() {
                if top <= effective {
                    break;
                }
                self.indent_stack.pop();
                self.tokens.push(Token {
                    kind: TokenKind::Ded,
                    span: Span::empty(self.pos as u32),
                });
            }
            let top = *self.indent_stack.last().unwrap_or(&0);
            if top != effective && !had_tab {
                self.diags.push(Diagnostic::error(
                    "E0012",
                    format!("indentation of {effective} spaces does not match any outer block"),
                    indent_span,
                ));
            }
        }
    }

    fn run(&mut self) {
        while self.pos < self.bytes.len() {
            if self.at_line_start && self.bracket_stack.is_empty() {
                self.handle_line_start();
                if self.pos >= self.bytes.len() {
                    break;
                }
            }

            let b = self.bytes[self.pos];
            match b {
                b' ' => {
                    self.pos += 1;
                }
                b'\t' => {
                    let start = self.pos;
                    self.pos += 1;
                    if self.bracket_stack.is_empty() {
                        self.diags.push(Diagnostic::error(
                            "E0010",
                            "tabs are not allowed; use spaces",
                            Span::new(start as u32, self.pos as u32),
                        ));
                    }
                }
                b'\r' => {
                    if self.peek_at(1) == Some(b'\n') {
                        let start = self.pos;
                        self.pos += 2;
                        self.handle_newline(start, self.pos);
                    } else {
                        let start = self.pos;
                        self.pos += 1;
                        self.diags.push(Diagnostic::error(
                            "E0001",
                            "unexpected lone carriage return",
                            Span::new(start as u32, self.pos as u32),
                        ));
                        self.push_token(TokenKind::Err, start, self.pos);
                    }
                }
                b'\n' => {
                    let start = self.pos;
                    self.pos += 1;
                    self.handle_newline(start, self.pos);
                }
                b'#' => {
                    while self.pos < self.bytes.len()
                        && self.bytes[self.pos] != b'\n'
                        && self.bytes[self.pos] != b'\r'
                    {
                        self.pos += 1;
                    }
                }
                b'\'' | b'"' => self.lex_string(),
                b'0'..=b'9' => self.lex_number(),
                b'.' => {
                    if matches!(self.peek_at(1), Some(b'0'..=b'9')) {
                        self.lex_number();
                    } else {
                        let start = self.pos;
                        self.pos += 1;
                        self.push_token(TokenKind::Dot, start, self.pos);
                    }
                }
                b'A'..=b'Z' | b'a'..=b'z' | b'_' => self.lex_ident_or_kw(),
                b'+' => {
                    self.lex_two_or_one(b'=', TokenKind::Aug(AugOp::AddAssign), TokenKind::Plus)
                }
                b'-' => {
                    self.lex_two_or_one(b'=', TokenKind::Aug(AugOp::SubAssign), TokenKind::Minus)
                }
                b'*' => {
                    if self.peek_at(1) == Some(b'*') {
                        let start = self.pos;
                        self.pos += 2;
                        self.push_token(TokenKind::StarStar, start, self.pos);
                    } else {
                        self.lex_two_or_one(
                            b'=',
                            TokenKind::Aug(AugOp::MulAssign),
                            TokenKind::Star,
                        );
                    }
                }
                b'/' => {
                    self.lex_two_or_one(b'=', TokenKind::Aug(AugOp::DivAssign), TokenKind::Slash)
                }
                b'=' => self.lex_two_or_one(b'=', TokenKind::EqEq, TokenKind::Eq),
                b'<' => self.lex_two_or_one(b'=', TokenKind::LtEq, TokenKind::Lt),
                b'>' => self.lex_two_or_one(b'=', TokenKind::GtEq, TokenKind::Gt),
                b'!' => {
                    let start = self.pos;
                    if self.peek_at(1) == Some(b'=') {
                        self.pos += 2;
                        self.push_token(TokenKind::BangEq, start, self.pos);
                    } else {
                        self.pos += 1;
                        self.diags.push(
                            Diagnostic::error(
                                "E0006",
                                "unsupported operator `!`",
                                Span::new(start as u32, self.pos as u32),
                            )
                            .with_help("use `not` instead of `!`"),
                        );
                        self.push_token(TokenKind::Err, start, self.pos);
                    }
                }
                b'&' => {
                    let start = self.pos;
                    if self.peek_at(1) == Some(b'&') {
                        self.pos += 2;
                        self.diags.push(
                            Diagnostic::error(
                                "E0006",
                                "unsupported operator `&&`",
                                Span::new(start as u32, self.pos as u32),
                            )
                            .with_help("use `and` instead of `&&`"),
                        );
                    } else {
                        self.pos += 1;
                        self.diags.push(Diagnostic::error(
                            "E0001",
                            "invalid character `&`",
                            Span::new(start as u32, self.pos as u32),
                        ));
                    }
                    self.push_token(TokenKind::Err, start, self.pos);
                }
                b'|' => {
                    let start = self.pos;
                    if self.peek_at(1) == Some(b'|') {
                        self.pos += 2;
                        self.diags.push(
                            Diagnostic::error(
                                "E0006",
                                "unsupported operator `||`",
                                Span::new(start as u32, self.pos as u32),
                            )
                            .with_help("use `or` instead of `||`"),
                        );
                    } else {
                        self.pos += 1;
                        self.diags.push(Diagnostic::error(
                            "E0001",
                            "invalid character `|`",
                            Span::new(start as u32, self.pos as u32),
                        ));
                    }
                    self.push_token(TokenKind::Err, start, self.pos);
                }
                b'%' => {
                    let start = self.pos;
                    self.pos += 1;
                    self.diags.push(
                        Diagnostic::error(
                            "E0001",
                            "unexpected `%` operator",
                            Span::new(start as u32, self.pos as u32),
                        )
                        .with_help(
                            "LangMelogne has no `%` operator; use `mod(a, b)` for modulo or attach `%` directly to a number",
                        ),
                    );
                    self.push_token(TokenKind::Err, start, self.pos);
                }
                b'@' => self.push_single(TokenKind::At),
                b'(' => {
                    let start = self.pos;
                    self.pos += 1;
                    let span = Span::new(start as u32, self.pos as u32);
                    self.bracket_stack.push((b'(', span));
                    self.push_token(TokenKind::LParen, start, self.pos);
                }
                b'[' => {
                    let start = self.pos;
                    self.pos += 1;
                    let span = Span::new(start as u32, self.pos as u32);
                    self.bracket_stack.push((b'[', span));
                    self.push_token(TokenKind::LBracket, start, self.pos);
                }
                b')' => self.handle_close_bracket(b')', b'(', TokenKind::RParen),
                b']' => self.handle_close_bracket(b']', b'[', TokenKind::RBracket),
                b',' => self.push_single(TokenKind::Comma),
                b':' => self.push_single(TokenKind::Colon),
                b';' => self.push_single(TokenKind::Semi),
                _ => {
                    let start = self.pos;
                    let ch = self.src[self.pos..]
                        .chars()
                        .next()
                        .expect("valid UTF-8 char");
                    self.pos += ch.len_utf8();
                    let msg = if ch == '\0' {
                        "invalid NUL character".to_string()
                    } else {
                        format!("invalid character `{ch}`")
                    };
                    self.diags.push(Diagnostic::error(
                        "E0001",
                        msg,
                        Span::new(start as u32, self.pos as u32),
                    ));
                    self.push_token(TokenKind::Err, start, self.pos);
                }
            }
        }

        let eof = self.bytes.len();
        if self.line_has_tokens {
            self.push_token(TokenKind::Nl, eof, eof);
        }

        while self.indent_stack.len() > 1 {
            self.indent_stack.pop();
            self.tokens.push(Token {
                kind: TokenKind::Ded,
                span: Span::empty(eof as u32),
            });
        }

        let max_needed = (MAX_DIAGNOSTICS + 1).saturating_sub(self.diags.len());
        if max_needed > 0 && !self.bracket_stack.is_empty() {
            let count = self.bracket_stack.len().min(max_needed);
            let mut line: u32 = 1;
            let mut col: u32 = 1;
            let mut i = 0usize;
            for &(open_byte, open_span) in &self.bracket_stack[..count] {
                let target = (open_span.start as usize).min(self.src.len());
                while i < target {
                    let ch = self.src[i..].chars().next().expect("valid UTF-8 slice");
                    let ch_len = ch.len_utf8();
                    if ch == '\r' {
                        if i + 1 < target && self.bytes.get(i + 1) == Some(&b'\n') {
                            i += 2;
                        } else {
                            i += 1;
                        }
                        line += 1;
                        col = 1;
                        continue;
                    } else if ch == '\n' {
                        line += 1;
                        col = 1;
                    } else {
                        col += 1;
                    }
                    i += ch_len;
                }
                let ch = open_byte as char;
                self.diags.push(Diagnostic::error(
                    "E0015",
                    format!("unclosed `{ch}` opened at {line}:{col}"),
                    Span::empty(eof as u32),
                ));
            }
        }
        self.bracket_stack.clear();

        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::empty(eof as u32),
        });
    }

    fn handle_newline(&mut self, nl_start: usize, nl_end: usize) {
        if !self.bracket_stack.is_empty() {
            // Inside `()` and `[]`, newlines and indentation are ignored.
            return;
        }
        if self.line_has_tokens {
            self.push_token(TokenKind::Nl, nl_start, nl_end);
        }
        self.line_has_tokens = false;
        self.emitted_py_kw_on_line = false;
        self.at_line_start = true;
    }

    fn handle_close_bracket(&mut self, close_byte: u8, expected_open: u8, kind: TokenKind) {
        let start = self.pos;
        self.pos += 1;
        let span = Span::new(start as u32, self.pos as u32);
        if self.bracket_stack.last().map(|(b, _)| *b) == Some(expected_open) {
            self.bracket_stack.pop();
            self.push_token(kind, start, self.pos);
        } else {
            let ch = close_byte as char;
            self.diags.push(Diagnostic::error(
                "E0014",
                format!("unmatched closing `{ch}`"),
                span,
            ));
            self.push_token(TokenKind::Err, start, self.pos);
        }
    }

    fn push_single(&mut self, kind: TokenKind) {
        let start = self.pos;
        self.pos += 1;
        self.push_token(kind, start, self.pos);
    }

    fn lex_two_or_one(&mut self, second: u8, two: TokenKind, one: TokenKind) {
        let start = self.pos;
        if self.peek_at(1) == Some(second) {
            self.pos += 2;
            self.push_token(two, start, self.pos);
        } else {
            self.pos += 1;
            self.push_token(one, start, self.pos);
        }
    }

    fn lex_string(&mut self) {
        let start = self.pos;
        let quote = self.bytes[self.pos];
        self.pos += 1;
        let mut out = String::new();
        let mut invalid_escape = false;

        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == quote {
                self.pos += 1;
                if invalid_escape {
                    self.push_token(TokenKind::Err, start, self.pos);
                } else {
                    self.push_token(TokenKind::Str(out), start, self.pos);
                }
                return;
            }
            if b == b'\n' || b == b'\r' {
                self.diags.push(Diagnostic::error(
                    "E0002",
                    "unterminated string literal",
                    Span::new(start as u32, self.pos as u32),
                ));
                self.push_token(TokenKind::Err, start, self.pos);
                return;
            }
            if b == b'\\' {
                let esc_start = self.pos;
                self.pos += 1;
                if self.pos >= self.bytes.len()
                    || self.bytes[self.pos] == b'\n'
                    || self.bytes[self.pos] == b'\r'
                {
                    self.diags.push(Diagnostic::error(
                        "E0002",
                        "unterminated string literal",
                        Span::new(start as u32, self.pos as u32),
                    ));
                    self.push_token(TokenKind::Err, start, self.pos);
                    return;
                }
                let esc = self.bytes[self.pos];
                match esc {
                    b'n' => {
                        out.push('\n');
                        self.pos += 1;
                    }
                    b't' => {
                        out.push('\t');
                        self.pos += 1;
                    }
                    b'\\' => {
                        out.push('\\');
                        self.pos += 1;
                    }
                    b'\'' => {
                        out.push('\'');
                        self.pos += 1;
                    }
                    b'"' => {
                        out.push('"');
                        self.pos += 1;
                    }
                    _ => {
                        let ch = self.src[self.pos..].chars().next().expect("valid UTF-8");
                        self.pos += ch.len_utf8();
                        invalid_escape = true;
                        self.diags.push(Diagnostic::error(
                            "E0007",
                            format!("invalid string escape `\\{ch}`"),
                            Span::new(esc_start as u32, self.pos as u32),
                        ));
                    }
                }
            } else {
                let ch = self.src[self.pos..].chars().next().expect("valid UTF-8");
                out.push(ch);
                self.pos += ch.len_utf8();
            }
        }

        self.diags.push(Diagnostic::error(
            "E0002",
            "unterminated string literal",
            Span::new(start as u32, self.pos as u32),
        ));
        self.push_token(TokenKind::Err, start, self.pos);
    }

    fn lex_number(&mut self) {
        let start = self.pos;

        // Reject non-decimal radix prefixes such as `0x10`, `0b10`, `0o10` as E0003.
        if self.peek() == Some(b'0')
            && matches!(
                self.peek_at(1),
                Some(b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
            )
        {
            self.pos += 2;
            while matches!(
                self.peek(),
                Some(b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_')
            ) {
                self.pos += 1;
            }
            self.diags.push(Diagnostic::error(
                "E0003",
                "malformed number literal (only base-10 decimals are supported)",
                Span::new(start as u32, self.pos as u32),
            ));
            self.push_token(TokenKind::Err, start, self.pos);
            return;
        }

        let mut malformed = false;
        let mut has_dot = false;
        let mut has_exp = false;

        if self.peek() == Some(b'.') {
            has_dot = true;
            self.pos += 1;
            let frac_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9' | b'_')) {
                self.pos += 1;
            }
            if !valid_digit_group(&self.src[frac_start..self.pos]) {
                malformed = true;
            }
        } else {
            let int_start = self.pos;
            while matches!(self.peek(), Some(b'0'..=b'9' | b'_')) {
                self.pos += 1;
            }
            if !valid_digit_group(&self.src[int_start..self.pos]) {
                malformed = true;
            }

            if self.peek() == Some(b'.') {
                has_dot = true;
                self.pos += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    let frac_start = self.pos;
                    while matches!(self.peek(), Some(b'0'..=b'9' | b'_')) {
                        self.pos += 1;
                    }
                    if !valid_digit_group(&self.src[frac_start..self.pos]) {
                        malformed = true;
                    }
                } else {
                    malformed = true;
                }
            }
        }

        // Exponent only triggers when `e`/`E` is followed by digits or `[+-]` + digits.
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let has_signed_digit = matches!(self.peek_at(1), Some(b'+' | b'-'))
                && matches!(self.peek_at(2), Some(b'0'..=b'9' | b'_'));
            let has_digit = matches!(self.peek_at(1), Some(b'0'..=b'9'));
            if has_digit || has_signed_digit {
                has_exp = true;
                self.pos += 1;
                if matches!(self.peek(), Some(b'+' | b'-')) {
                    self.pos += 1;
                }
                let exp_start = self.pos;
                while matches!(self.peek(), Some(b'0'..=b'9' | b'_')) {
                    self.pos += 1;
                }
                if !valid_digit_group(&self.src[exp_start..self.pos]) {
                    malformed = true;
                }
            }
        }

        let num_end = self.pos;
        if malformed {
            // Consume any attached suffix so we do not split a broken number into spurious identifiers.
            while matches!(
                self.peek(),
                Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
            ) {
                self.pos += 1;
            }
            self.diags.push(Diagnostic::error(
                "E0003",
                "malformed number literal",
                Span::new(start as u32, self.pos as u32),
            ));
            self.push_token(TokenKind::Err, start, self.pos);
            return;
        }

        let raw_num = &self.src[start..num_end];
        let parsed = parse_decimal(raw_num);
        let Some(value) = parsed else {
            self.diags.push(Diagnostic::error(
                "E0003",
                "number literal is out of decimal range",
                Span::new(start as u32, num_end as u32),
            ));
            self.push_token(TokenKind::Err, start, num_end);
            return;
        };

        // Check touching suffix (`%` or identifier-like letters).
        if self.peek() == Some(b'%') {
            self.pos += 1;
            self.push_token(TokenKind::Num { value, pct: true }, start, self.pos);
            return;
        }

        if matches!(self.peek(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_')) {
            let suf_start = self.pos;
            while matches!(
                self.peek(),
                Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
            ) {
                self.pos += 1;
            }
            let suffix = &self.src[suf_start..self.pos];
            let unit = match suffix {
                "m" => Some(TfUnit::Minute),
                "h" => Some(TfUnit::Hour),
                "d" => Some(TfUnit::Day),
                "w" => Some(TfUnit::Week),
                _ => None,
            };
            if let Some(unit) = unit {
                if !has_dot && !has_exp {
                    if let Ok(n) = u32::try_from(value.mantissa()) {
                        if value.scale() == 0 && !value.is_sign_negative() && n > 0 {
                            self.push_token(TokenKind::Timeframe { n, unit }, start, self.pos);
                            return;
                        }
                    }
                }
                self.diags.push(Diagnostic::error(
                    "E0005",
                    "timeframe literal must be a positive integer followed by `m`, `h`, `d`, or `w`",
                    Span::new(start as u32, self.pos as u32),
                ));
                self.push_token(TokenKind::Err, start, self.pos);
            } else {
                self.diags.push(Diagnostic::error(
                    "E0004",
                    format!("unknown number suffix `{suffix}`"),
                    Span::new(start as u32, self.pos as u32),
                ));
                self.push_token(TokenKind::Err, start, self.pos);
            }
            return;
        }

        self.push_token(TokenKind::Num { value, pct: false }, start, num_end);
    }

    fn lex_ident_or_kw(&mut self) {
        let start = self.pos;
        self.pos += 1;
        while matches!(
            self.peek(),
            Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')
        ) {
            self.pos += 1;
        }
        let word = &self.src[start..self.pos];

        let kind = match word {
            "def" => TokenKind::Kw(Kw::Def),
            "return" => TokenKind::Kw(Kw::Return),
            "and" => TokenKind::Kw(Kw::And),
            "or" => TokenKind::Kw(Kw::Or),
            "not" => TokenKind::Kw(Kw::Not),
            "if" => TokenKind::Kw(Kw::If),
            "else" => TokenKind::Kw(Kw::Else),
            "true" => TokenKind::Kw(Kw::True),
            "false" => TokenKind::Kw(Kw::False),
            _ if is_unsupported_python_kw(word) => {
                if !self.emitted_py_kw_on_line {
                    self.emitted_py_kw_on_line = true;
                    let help = if matches!(word, "for" | "break" | "continue") {
                        "loops are not supported in LangMelogne; use windowed functions like `sum(x, n)` or `highest(x, n)`"
                    } else {
                        "this Python keyword is not part of LangMelogne"
                    };
                    self.diags.push(
                        Diagnostic::error(
                            "E0021",
                            format!("unsupported Python keyword `{word}`"),
                            Span::new(start as u32, self.pos as u32),
                        )
                        .with_help(help),
                    );
                }
                TokenKind::Err
            }
            _ => TokenKind::Ident(word.to_string()),
        };

        self.push_token(kind, start, self.pos);
    }
}

fn valid_digit_group(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    if bytes[0] == b'_' || bytes[bytes.len() - 1] == b'_' {
        return false;
    }
    !bytes.windows(2).any(|w| w == b"__")
}

fn parse_decimal(raw: &str) -> Option<Decimal> {
    let mut cleaned = String::with_capacity(raw.len() + 1);
    if raw.starts_with('.') {
        cleaned.push('0');
    }
    for ch in raw.chars() {
        if ch == '_' {
            continue;
        }
        if ch == 'E' {
            cleaned.push('e');
        } else {
            cleaned.push(ch);
        }
    }
    if cleaned.contains('e') {
        Decimal::from_scientific(&cleaned).ok()
    } else {
        Decimal::from_str(&cleaned).ok()
    }
}

fn is_unsupported_python_kw(word: &str) -> bool {
    matches!(
        word,
        "for"
            | "break"
            | "continue"
            | "class"
            | "import"
            | "lambda"
            | "try"
            | "except"
            | "with"
            | "global"
            | "nonlocal"
            | "pass"
            | "del"
            | "yield"
            | "async"
            | "await"
            | "raise"
            | "assert"
            | "is"
            | "in"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_str(src: &str) -> String {
        let out = lex(src);
        assert!(
            out.diags.is_empty(),
            "unexpected diagnostics for {src:?}: {:?}",
            out.diags
        );
        tokens_str(&out.tokens)
    }

    fn assert_lex_err(src: &str, expected_code: &str) -> LexOutput {
        let out = lex(src);
        assert!(
            out.diags.iter().any(|d| d.code == expected_code),
            "expected {expected_code} for {src:?}, got {:?}",
            out.diags
        );
        assert_eq!(
            out.tokens.last().map(|t| &t.kind),
            Some(&TokenKind::Eof),
            "token stream must end with EOF for {src:?}"
        );
        out
    }

    #[test]
    fn l01_empty_file() {
        assert_eq!(lex_str(""), "EOF");
    }

    #[test]
    fn l02_simple_assign() {
        assert_eq!(lex_str("x = 1"), "ID(x) = NUM(1) NL EOF");
    }

    #[test]
    fn l03_trailing_lf() {
        assert_eq!(lex_str("x = 1\n"), lex_str("x = 1"));
    }

    #[test]
    fn l04_two_lines() {
        assert_eq!(
            lex_str("x = 1\ny = 2"),
            "ID(x) = NUM(1) NL ID(y) = NUM(2) NL EOF"
        );
    }

    #[test]
    fn l05_comments_and_blank_lines() {
        assert_eq!(lex_str("# note\n\nx = 1 # tail"), "ID(x) = NUM(1) NL EOF");
    }

    #[test]
    fn l06_multiple_blank_lines() {
        assert_eq!(lex_str("x = 1\n\n\ny = 2"), lex_str("x = 1\ny = 2"));
    }

    #[test]
    fn l07_crlf() {
        assert_eq!(lex_str("x = 1\r\ny = 2"), lex_str("x = 1\ny = 2"));
    }

    #[test]
    fn l08_leading_bom() {
        assert_eq!(lex_str("\u{FEFF}x = 1"), lex_str("x = 1"));
    }

    #[test]
    fn l09_no_spaces() {
        assert_eq!(lex_str("x=1"), lex_str("x = 1"));
    }

    #[test]
    fn l10_number_formats() {
        assert_eq!(
            lex_str("1 1.5 .5 1_000 1e3 1.5E-3"),
            "NUM(1) NUM(1.5) NUM(0.5) NUM(1000) NUM(1000) NUM(0.0015) NL EOF"
        );
    }

    #[test]
    fn l11_trailing_dot_number() {
        assert_lex_err("1.", "E0003");
    }

    #[test]
    fn l12_malformed_numbers() {
        assert_lex_err("1__0", "E0003");
        assert_lex_err("1_", "E0003");
        assert_lex_err("0x10", "E0003");
    }

    #[test]
    fn l13_percent_literals() {
        assert_eq!(lex_str("40% 1.5%"), "NUM(40%) NUM(1.5%) NL EOF");
    }

    #[test]
    fn l14_timeframe_literals() {
        assert_eq!(
            lex_str("5m 15m 1h 1d 2w"),
            "TF(5m) TF(15m) TF(1h) TF(1d) TF(2w) NL EOF"
        );
    }

    #[test]
    fn l15_invalid_timeframes() {
        assert_lex_err("0m", "E0005");
        assert_lex_err("1.5h", "E0005");
    }

    #[test]
    fn l16_unknown_number_suffix() {
        assert_lex_err("5min", "E0004");
        assert_lex_err("3x", "E0004");
    }

    #[test]
    fn l17_space_separates_number_and_ident() {
        assert_eq!(lex_str("5 m"), "NUM(5) ID(m) NL EOF");
    }

    #[test]
    fn l18_identifiers_with_digits_and_underscores() {
        assert_eq!(
            lex_str("m5 x1m _a a_1"),
            "ID(m5) ID(x1m) ID(_a) ID(a_1) NL EOF"
        );
    }

    #[test]
    fn l19_keywords_are_case_sensitive() {
        assert_eq!(lex_str("Def AND True"), "ID(Def) ID(AND) ID(True) NL EOF");
    }

    #[test]
    fn l20_hard_keywords() {
        assert_eq!(
            lex_str("def return and or not if else true false"),
            "KW(def) KW(return) KW(and) KW(or) KW(not) KW(if) KW(else) KW(true) KW(false) NL EOF"
        );
    }

    #[test]
    fn l21_unsupported_python_keywords() {
        for kw in ["for", "import", "lambda", "in"] {
            let out = assert_lex_err(kw, "E0021");
            assert!(out.diags[0].help.is_some(), "expected help hint for `{kw}`");
        }
    }

    #[test]
    fn l22_stray_percent() {
        let out = assert_lex_err("a % b", "E0001");
        assert_eq!(tokens_str(&out.tokens), "ID(a) ERR ID(b) NL EOF");
        let help = out.diags[0].help.as_deref().unwrap_or("");
        assert!(help.contains("mod(a, b)"), "unexpected help: {help}");
    }

    #[test]
    fn l23_punctuation() {
        assert_eq!(
            lex_str("+ - * / ** == != < <= > >= = @ ( ) [ ] , : . ;"),
            "+ - * / ** == != < <= > >= = @ ( ) [ ] , : . ; NL EOF"
        );
    }

    #[test]
    fn l24_maximal_munch_eq() {
        assert_eq!(lex_str("a===b"), "ID(a) == = ID(b) NL EOF");
    }

    #[test]
    fn l25_mul_neg() {
        assert_eq!(lex_str("a*-b"), "ID(a) * - ID(b) NL EOF");
    }

    #[test]
    fn l26_c_style_bool_ops() {
        for (src, kw) in [("&&", "and"), ("||", "or"), ("!", "not")] {
            let out = assert_lex_err(src, "E0006");
            let help = out.diags[0].help.as_deref().unwrap_or("");
            assert!(help.contains(kw), "help for {src} should name {kw}");
        }
    }

    #[test]
    fn l27_augmented_assign() {
        assert_eq!(lex_str("x += 1"), "ID(x) AUG(+=) NUM(1) NL EOF");
    }

    #[test]
    fn l28_strings() {
        assert_eq!(lex_str("'ab' 'cd' ''"), "STR(ab) STR(cd) STR() NL EOF");
        assert_eq!(
            lex_str("\"ab\" \"cd\" \"\""),
            "STR(ab) STR(cd) STR() NL EOF"
        );
    }

    #[test]
    fn l29_string_escape_newline() {
        let out = lex("'a\\nb'");
        assert!(out.diags.is_empty());
        assert!(matches!(&out.tokens[0].kind, TokenKind::Str(s) if s == "a\nb"));
    }

    #[test]
    fn l30_invalid_string_escape() {
        assert_lex_err("'\\q'", "E0007");
    }

    #[test]
    fn l31_unterminated_string() {
        assert_lex_err("'abc", "E0002");
        assert_lex_err("'a\nb'", "E0002");
    }

    #[test]
    fn l32_non_ascii_outside_and_inside_strings_comments() {
        assert_lex_err("x = é", "E0001");
        assert_eq!(lex_str("x = 'é' # é"), "ID(x) = STR(é) NL EOF");
    }

    #[test]
    fn l33_nul_and_lone_cr() {
        assert_lex_err("x\0", "E0001");
        assert_lex_err("x = 1\ry = 2", "E0001");
    }

    #[test]
    fn l34_def_block_indent_dedent() {
        assert_eq!(
            lex_str("def f(x):\n    return x\ny = 1"),
            "KW(def) ID(f) ( ID(x) ) : NL IND KW(return) ID(x) NL DED ID(y) = NUM(1) NL EOF"
        );
    }

    #[test]
    fn l35_nested_indents_and_double_dedent() {
        assert_eq!(
            lex_str("a:\n    b\n        c\nd"),
            "ID(a) : NL IND ID(b) NL IND ID(c) NL DED DED ID(d) NL EOF"
        );
    }

    #[test]
    fn l36_tab_in_indentation() {
        assert_lex_err("a:\n\tb", "E0010");
    }

    #[test]
    fn l37_indent_not_multiple_of_4() {
        assert_lex_err("a:\n  b", "E0012");
    }

    #[test]
    fn l38_indent_jumps_more_than_one_level() {
        assert_lex_err("a:\n        b", "E0011");
    }

    #[test]
    fn l39_indented_first_line() {
        assert_lex_err("   x = 1 ", "E0013");
    }

    #[test]
    fn l40_newlines_and_indent_ignored_in_parens() {
        assert_eq!(lex_str("f(a,\n      b)"), "ID(f) ( ID(a) , ID(b) ) NL EOF");
    }

    #[test]
    fn l41_unmatched_closing_bracket() {
        assert_lex_err("x = 1)", "E0014");
    }

    #[test]
    fn l42_unclosed_bracket_at_eof() {
        let src = "x = (1";
        let out = assert_lex_err(src, "E0015");
        let e0015 = out.diags.iter().find(|d| d.code == "E0015").unwrap();
        assert_eq!(e0015.span, Span::empty(src.len() as u32));
        assert!(
            e0015.message.contains("1:5"),
            "expected opening 1:5 in message, got: {}",
            e0015.message
        );
    }

    #[test]
    fn l43_exact_spans() {
        let out = lex("ab = 12");
        assert!(out.diags.is_empty());
        assert_eq!(out.tokens[0].span, Span::new(0, 2));
        assert_eq!(out.tokens[1].span, Span::new(3, 4));
        assert_eq!(out.tokens[2].span, Span::new(5, 7));
    }

    #[test]
    fn l44_trailing_spaces_and_whitespace_only_line_in_block() {
        assert_eq!(
            lex_str("a:   \n    x = 1   \n    \n    y = 2   "),
            "ID(a) : NL IND ID(x) = NUM(1) NL ID(y) = NUM(2) NL DED EOF"
        );
    }

    #[test]
    fn l45_attr_and_at_tf() {
        assert_eq!(lex_str("a.b.c"), "ID(a) . ID(b) . ID(c) NL EOF");
        assert_eq!(lex_str("close@1d"), "ID(close) @ TF(1d) NL EOF");
    }

    #[test]
    fn l46_source_too_large() {
        let src = "x".repeat(MAX_SOURCE_BYTES + 1);
        let out = assert_lex_err(&src, "E0017");
        assert_eq!(tokens_str(&out.tokens), "EOF");
    }

    #[test]
    fn l47_many_unclosed_brackets_is_linear_and_capped() {
        let src = "(".repeat(MAX_SOURCE_BYTES);
        let t0 = std::time::Instant::now();
        let out = lex(&src);
        assert!(
            t0.elapsed().as_secs_f64() < 1.0,
            "lexing 1 MiB of unclosed `(` took too long: {:?}",
            t0.elapsed()
        );
        assert_eq!(out.diags.len(), MAX_DIAGNOSTICS + 1);
        assert!(out.diags[..MAX_DIAGNOSTICS]
            .iter()
            .all(|d| d.code == "E0015"));
        assert_eq!(out.diags[MAX_DIAGNOSTICS].code, "E0199");
    }
}
