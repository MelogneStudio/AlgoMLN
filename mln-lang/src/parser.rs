use std::collections::HashSet;

use crate::ast::{Arg, BinOp, CmpOp, Def, Expr, Ident, Param, Program, Stmt, Timeframe, UnOp};
use crate::diag::{finalize_diagnostics, Diagnostic};
use crate::lexer::{lex, Kw, Token, TokenKind};
use crate::span::Span;

/// Maximum expression nesting depth before `E0120` is emitted.
pub const MAX_EXPR_DEPTH: usize = 64;

/// Output of [`parse`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseOutput {
    pub program: Program,
    pub diags: Vec<Diagnostic>,
}

/// Parses `src` into a [`Program`] and diagnostics.
///
/// Always returns a `Program` (with `Error` nodes where recovery occurred) and never panics.
pub fn parse(src: &str) -> ParseOutput {
    let lex_out = lex(src);
    let eof_offset = src.len().min(u32::MAX as usize) as u32;
    let mut parser = Parser::new(lex_out.tokens, lex_out.diags);
    let program = parser.parse_program();
    finalize_diagnostics(&mut parser.diags, eof_offset);
    ParseOutput {
        program,
        diags: parser.diags,
    }
}

fn is_block_keyword(word: &str) -> bool {
    matches!(
        word,
        "state" | "on" | "while" | "score" | "portfolio" | "use"
    )
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    lex_diag_starts: HashSet<u32>,
    depth: usize,
    stmt_has_error: bool,
    saw_err_token_in_stmt: bool,
}

impl Parser {
    fn new(tokens: Vec<Token>, lex_diags: Vec<Diagnostic>) -> Self {
        let lex_diag_starts = lex_diags.iter().map(|d| d.span.start).collect();
        Self {
            tokens,
            pos: 0,
            diags: lex_diags,
            lex_diag_starts,
            depth: 0,
            stmt_has_error: false,
            saw_err_token_in_stmt: false,
        }
    }

    fn skip_err_tokens(&mut self) {
        while matches!(
            self.tokens.get(self.pos).map(|t| &t.kind),
            Some(TokenKind::Err)
        ) {
            self.saw_err_token_in_stmt = true;
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> &Token {
        self.skip_err_tokens();
        let idx = self.pos.min(self.tokens.len().saturating_sub(1));
        &self.tokens[idx]
    }

    fn peek_kind(&mut self) -> &TokenKind {
        &self.peek().kind
    }

    fn peek_second_kind(&mut self) -> Option<&TokenKind> {
        self.skip_err_tokens();
        let mut idx = self.pos + 1;
        while let Some(tok) = self.tokens.get(idx) {
            if tok.kind == TokenKind::Err {
                self.saw_err_token_in_stmt = true;
                idx += 1;
            } else {
                return Some(&tok.kind);
            }
        }
        None
    }

    fn bump(&mut self) -> Token {
        self.skip_err_tokens();
        if self.pos < self.tokens.len() - 1 {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            tok
        } else {
            self.tokens.last().expect("EOF token").clone()
        }
    }

    fn current_span(&mut self) -> Span {
        self.peek().span
    }

    fn emit_stmt_error(&mut self, diag: Diagnostic) {
        if self.stmt_has_error {
            return;
        }
        self.stmt_has_error = true;
        if self.lex_diag_starts.contains(&diag.span.start) {
            return;
        }
        self.diags.push(diag);
    }

    fn recover_to_stmt_end(&mut self) {
        let mut bracket_depth: usize = 0;
        loop {
            let kind = &self.tokens[self.pos.min(self.tokens.len() - 1)].kind;
            match kind {
                TokenKind::Eof => break,
                TokenKind::Nl | TokenKind::Ded if bracket_depth == 0 => {
                    if *kind == TokenKind::Nl {
                        self.pos += 1;
                    }
                    break;
                }
                TokenKind::LParen | TokenKind::LBracket => {
                    bracket_depth += 1;
                    self.pos += 1;
                }
                TokenKind::RParen | TokenKind::RBracket => {
                    bracket_depth = bracket_depth.saturating_sub(1);
                    self.pos += 1;
                }
                _ => {
                    self.pos += 1;
                }
            }
        }
    }

    fn skip_indented_block(&mut self) {
        if !matches!(self.peek_kind(), TokenKind::Ind) {
            return;
        }
        let mut ind_depth: usize = 0;
        loop {
            let kind = &self.tokens[self.pos.min(self.tokens.len() - 1)].kind;
            match kind {
                TokenKind::Ind => {
                    ind_depth += 1;
                    self.pos += 1;
                }
                TokenKind::Ded => {
                    self.pos += 1;
                    ind_depth = ind_depth.saturating_sub(1);
                    if ind_depth == 0 {
                        break;
                    }
                }
                TokenKind::Eof => break,
                _ => {
                    self.pos += 1;
                }
            }
        }
    }

    fn parse_program(&mut self) -> Program {
        let mut stmts = Vec::new();
        loop {
            self.skip_err_tokens();
            match self.peek_kind() {
                TokenKind::Eof => break,
                TokenKind::Nl => {
                    self.bump();
                }
                TokenKind::Ded => {
                    self.bump();
                }
                _ => {
                    stmts.push(self.parse_stmt(false));
                }
            }
        }
        Program { stmts }
    }

    fn parse_stmt(&mut self, in_def: bool) -> Stmt {
        self.stmt_has_error = false;
        self.saw_err_token_in_stmt = false;

        // If the statement starts directly with an `ERR` token (e.g., `for x in y`),
        // a lexer diagnostic was already emitted; skip the line cleanly.
        if matches!(
            self.tokens.get(self.pos).map(|t| &t.kind),
            Some(TokenKind::Err)
        ) {
            let span = self.tokens[self.pos].span;
            self.recover_to_stmt_end();
            self.skip_indented_block();
            return Stmt::Error(span);
        }

        match self.peek_kind().clone() {
            TokenKind::Ind => {
                let span = self.bump().span;
                self.emit_stmt_error(Diagnostic::error("E0155", "unexpected indent", span));
                self.recover_to_stmt_end();
                // Skip remaining lines in this unexpected indented block.
                let mut ind_depth: usize = 1;
                loop {
                    let kind = &self.tokens[self.pos.min(self.tokens.len() - 1)].kind;
                    match kind {
                        TokenKind::Ind => {
                            ind_depth += 1;
                            self.pos += 1;
                        }
                        TokenKind::Ded => {
                            self.pos += 1;
                            ind_depth -= 1;
                            if ind_depth == 0 {
                                break;
                            }
                        }
                        TokenKind::Eof => break,
                        _ => self.pos += 1,
                    }
                }
                Stmt::Error(span)
            }
            TokenKind::Kw(Kw::Def) => self.parse_def(in_def),
            TokenKind::Kw(Kw::Return) => {
                let span = self.bump().span;
                self.emit_stmt_error(Diagnostic::error(
                    "E0150",
                    "`return` is only valid as the final statement of a `def` block",
                    span,
                ));
                self.recover_to_stmt_end();
                Stmt::Error(span)
            }
            TokenKind::Semi => {
                let span = self.bump().span;
                self.emit_stmt_error(
                    Diagnostic::error("E0130", "`;` is not supported", span)
                        .with_help("remove `;` and write each statement on its own line"),
                );
                self.recover_to_stmt_end();
                Stmt::Error(span)
            }
            TokenKind::Ident(ref word)
                if is_block_keyword(word)
                    && !matches!(
                        self.peek_second_kind(),
                        Some(TokenKind::Eq | TokenKind::Aug(_))
                    ) =>
            {
                let kw_tok = self.bump();
                let word = match kw_tok.kind {
                    TokenKind::Ident(s) => s,
                    _ => unreachable!(),
                };
                self.emit_stmt_error(Diagnostic::error(
                    "E0140",
                    format!("`{word}` blocks are not available in this version"),
                    kw_tok.span,
                ));
                self.recover_to_stmt_end();
                self.skip_indented_block();
                Stmt::Error(kw_tok.span)
            }
            TokenKind::Ident(_) if matches!(self.peek_second_kind(), Some(TokenKind::Eq)) => {
                let name_tok = self.bump();
                let name = match name_tok.kind {
                    TokenKind::Ident(s) => Ident {
                        name: s,
                        span: name_tok.span,
                    },
                    _ => unreachable!(),
                };
                self.bump(); // `=`
                let value = self.parse_expr();
                self.consume_stmt_end();
                let span = name.span.join(value.span());
                Stmt::Assign { name, value, span }
            }
            TokenKind::Ident(_) if matches!(self.peek_second_kind(), Some(TokenKind::Aug(_))) => {
                let _id_tok = self.bump();
                let aug_tok = self.bump();
                let op_str = match aug_tok.kind {
                    TokenKind::Aug(op) => op.as_str(),
                    _ => unreachable!(),
                };
                self.emit_stmt_error(
                    Diagnostic::error(
                        "E0154",
                        format!("augmented assignment `{op_str}` is not supported"),
                        aug_tok.span,
                    )
                    .with_help("write `x = x + ...` as a plain assignment instead"),
                );
                self.recover_to_stmt_end();
                Stmt::Error(aug_tok.span)
            }
            _ => {
                let expr = self.parse_expr();
                let span = expr.span();
                if self.stmt_has_error {
                    self.recover_to_stmt_end();
                    return Stmt::Error(span);
                }
                match self.peek_kind().clone() {
                    TokenKind::Eq => {
                        let eq_span = self.bump().span;
                        self.emit_stmt_error(Diagnostic::error(
                            "E0153",
                            "invalid assignment target; only plain variable names may be assigned",
                            span.join(eq_span),
                        ));
                        self.recover_to_stmt_end();
                    }
                    TokenKind::Aug(op) => {
                        let aug_span = self.bump().span;
                        self.emit_stmt_error(
                            Diagnostic::error(
                                "E0154",
                                format!("augmented assignment `{}` is not supported", op.as_str()),
                                aug_span,
                            )
                            .with_help("write `x = x + ...` as a plain assignment instead"),
                        );
                        self.recover_to_stmt_end();
                    }
                    TokenKind::Semi => {
                        let semi_span = self.bump().span;
                        self.emit_stmt_error(
                            Diagnostic::error("E0130", "`;` is not supported", semi_span)
                                .with_help("remove `;` and write each statement on its own line"),
                        );
                        self.recover_to_stmt_end();
                    }
                    TokenKind::Nl | TokenKind::Eof | TokenKind::Ded => {
                        if in_def {
                            self.emit_stmt_error(Diagnostic::error(
                                "E0119",
                                "only assignments are allowed in a `def` block",
                                span,
                            ));
                        } else {
                            self.emit_stmt_error(
                                Diagnostic::error(
                                    "E0152",
                                    "expression statement has no effect",
                                    span,
                                )
                                .with_help("assign the expression to a variable, e.g. `x = ...`"),
                            );
                        }
                        if matches!(self.peek_kind(), TokenKind::Nl) {
                            self.bump();
                        }
                    }
                    _ => {
                        let bad_span = self.current_span();
                        if !self.saw_err_token_in_stmt {
                            self.emit_stmt_error(Diagnostic::error(
                                "E0100",
                                "unexpected token in statement",
                                bad_span,
                            ));
                        }
                        self.recover_to_stmt_end();
                    }
                }
                Stmt::Error(span)
            }
        }
    }

    fn parse_def(&mut self, in_def: bool) -> Stmt {
        let def_span = self.bump().span; // `def`
        if in_def {
            self.emit_stmt_error(Diagnostic::error(
                "E0117",
                "nested `def` definitions are not allowed",
                def_span,
            ));
            self.recover_to_stmt_end();
            self.skip_indented_block();
            return Stmt::Error(def_span);
        }

        let name = match self.peek_kind().clone() {
            TokenKind::Ident(s) => {
                let id_span = self.bump().span;
                Ident {
                    name: s,
                    span: id_span,
                }
            }
            _ => {
                let err_span = self.current_span();
                self.emit_stmt_error(Diagnostic::error(
                    "E0102",
                    "expected function name identifier after `def`",
                    err_span,
                ));
                self.recover_to_stmt_end();
                self.skip_indented_block();
                return Stmt::Error(def_span.join(err_span));
            }
        };

        if !matches!(self.peek_kind(), TokenKind::LParen) {
            let err_span = self.current_span();
            self.emit_stmt_error(Diagnostic::error(
                "E0100",
                "expected `(` after function name",
                err_span,
            ));
            self.recover_to_stmt_end();
            self.skip_indented_block();
            return Stmt::Error(def_span.join(err_span));
        }
        self.bump(); // `(`

        let params = self.parse_params();
        if self.stmt_has_error {
            self.recover_to_stmt_end();
            self.skip_indented_block();
            return Stmt::Error(def_span.join(name.span));
        }

        match self.peek_kind().clone() {
            TokenKind::Eq => {
                self.bump(); // `=`
                let ret = self.parse_expr();
                self.consume_stmt_end();
                let span = def_span.join(ret.span());
                Stmt::Def(Def {
                    name,
                    params,
                    body: Vec::new(),
                    ret,
                    span,
                })
            }
            TokenKind::Colon => {
                let colon_span = self.bump().span;
                self.consume_stmt_end();
                if self.stmt_has_error {
                    self.skip_indented_block();
                    return Stmt::Error(def_span.join(colon_span));
                }

                if !matches!(self.peek_kind(), TokenKind::Ind) {
                    let err_span = self.current_span();
                    self.emit_stmt_error(Diagnostic::error(
                        "E0118",
                        "`def` block must be indented and end with `return`",
                        err_span,
                    ));
                    return Stmt::Error(def_span.join(colon_span));
                }
                self.bump(); // `IND`

                let mut body = Vec::new();
                let mut ret: Option<Expr> = None;

                loop {
                    self.skip_err_tokens();
                    match self.peek_kind().clone() {
                        TokenKind::Ded => {
                            self.bump();
                            break;
                        }
                        TokenKind::Eof => break,
                        TokenKind::Nl => {
                            self.bump();
                        }
                        TokenKind::Kw(Kw::Return) => {
                            self.stmt_has_error = false;
                            self.saw_err_token_in_stmt = false;
                            let ret_kw_span = self.bump().span;
                            let ret_expr = self.parse_expr();
                            self.consume_stmt_end();
                            let ret_span = ret_kw_span.join(ret_expr.span());
                            ret = Some(ret_expr);

                            // Check if any statements remain before `DED`.
                            while matches!(self.peek_kind(), TokenKind::Nl) {
                                self.bump();
                            }
                            if !matches!(self.peek_kind(), TokenKind::Ded | TokenKind::Eof) {
                                let trailing_span = self.current_span();
                                self.diags.push(Diagnostic::error(
                                    "E0150",
                                    "`return` must be the final statement in a `def` block",
                                    trailing_span.join(ret_span),
                                ));
                                // Drain the remainder of this `def` block up to matching `DED`.
                                let mut ind_depth: usize = 1;
                                loop {
                                    let kind =
                                        &self.tokens[self.pos.min(self.tokens.len() - 1)].kind;
                                    match kind {
                                        TokenKind::Ind => {
                                            ind_depth += 1;
                                            self.pos += 1;
                                        }
                                        TokenKind::Ded => {
                                            self.pos += 1;
                                            ind_depth -= 1;
                                            if ind_depth == 0 {
                                                break;
                                            }
                                        }
                                        TokenKind::Eof => break,
                                        _ => self.pos += 1,
                                    }
                                }
                                break;
                            }
                            if matches!(self.peek_kind(), TokenKind::Ded) {
                                self.bump();
                            }
                            break;
                        }
                        _ => {
                            body.push(self.parse_stmt(true));
                        }
                    }
                }

                let ret = match ret {
                    Some(e) => e,
                    None => {
                        let end_span = body
                            .last()
                            .map(|s| s.span())
                            .unwrap_or_else(|| def_span.join(colon_span));
                        self.diags.push(Diagnostic::error(
                            "E0118",
                            format!("`def {}` block is missing its final `return`", name.name),
                            end_span,
                        ));
                        Expr::Error(Span::empty(end_span.end))
                    }
                };

                let mut span = def_span.join(name.span).join(ret.span());
                for p in &params {
                    span = span.join(p.span);
                }
                for s in &body {
                    span = span.join(s.span());
                }
                Stmt::Def(Def {
                    name,
                    params,
                    body,
                    ret,
                    span,
                })
            }
            _ => {
                let err_span = self.current_span();
                self.emit_stmt_error(Diagnostic::error(
                    "E0100",
                    "expected `=` or `:` after `def` parameter list",
                    err_span,
                ));
                self.recover_to_stmt_end();
                Stmt::Error(def_span.join(err_span))
            }
        }
    }

    fn parse_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();
        let mut seen_default = false;

        if matches!(self.peek_kind(), TokenKind::RParen) {
            self.bump();
            return params;
        }

        loop {
            if self.stmt_has_error {
                return params;
            }
            let ident = match self.peek_kind().clone() {
                TokenKind::Ident(s) => {
                    let span = self.bump().span;
                    Ident { name: s, span }
                }
                _ => {
                    let err_span = self.current_span();
                    if !self.saw_err_token_in_stmt {
                        self.emit_stmt_error(Diagnostic::error(
                            "E0102",
                            "expected parameter name identifier",
                            err_span,
                        ));
                    }
                    return params;
                }
            };

            if !seen_names.insert(ident.name.clone()) {
                self.emit_stmt_error(Diagnostic::error(
                    "E0115",
                    format!("duplicate parameter `{}`", ident.name),
                    ident.span,
                ));
            }

            let default = if matches!(self.peek_kind(), TokenKind::Eq) {
                self.bump();
                seen_default = true;
                Some(self.parse_expr())
            } else {
                if seen_default {
                    self.emit_stmt_error(Diagnostic::error(
                        "E0116",
                        format!(
                            "required parameter `{}` cannot follow a parameter with a default",
                            ident.name
                        ),
                        ident.span,
                    ));
                }
                None
            };

            let span = match &default {
                Some(d) => ident.span.join(d.span()),
                None => ident.span,
            };
            params.push(Param {
                name: ident,
                default,
                span,
            });

            if matches!(self.peek_kind(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek_kind(), TokenKind::RParen) {
                    self.bump();
                    return params;
                }
            } else {
                break;
            }
        }

        self.expect_closing_bracket(TokenKind::RParen, "`)`");
        params
    }

    fn consume_stmt_end(&mut self) {
        if self.stmt_has_error {
            self.recover_to_stmt_end();
            return;
        }
        match self.peek_kind().clone() {
            TokenKind::Nl => {
                self.bump();
            }
            TokenKind::Eof | TokenKind::Ded => {}
            TokenKind::Semi => {
                let span = self.bump().span;
                self.emit_stmt_error(
                    Diagnostic::error("E0130", "`;` is not supported", span)
                        .with_help("remove `;` and write each statement on its own line"),
                );
                self.recover_to_stmt_end();
            }
            _ => {
                let span = self.current_span();
                if !self.saw_err_token_in_stmt {
                    self.emit_stmt_error(Diagnostic::error(
                        "E0100",
                        "unexpected trailing token in statement",
                        span,
                    ));
                }
                self.recover_to_stmt_end();
            }
        }
    }

    fn enter_depth(&mut self, span: Span) -> bool {
        if self.depth >= MAX_EXPR_DEPTH {
            self.emit_stmt_error(Diagnostic::error(
                "E0120",
                format!("expression nesting exceeds maximum depth of {MAX_EXPR_DEPTH}"),
                span,
            ));
            false
        } else {
            self.depth += 1;
            true
        }
    }

    fn parse_expr(&mut self) -> Expr {
        let then_expr = self.parse_or();
        if matches!(self.peek_kind(), TokenKind::Kw(Kw::If)) {
            let if_span = self.bump().span;
            if !self.enter_depth(if_span) {
                return Expr::Error(then_expr.span().join(if_span));
            }
            let cond = self.parse_or();
            if !matches!(self.peek_kind(), TokenKind::Kw(Kw::Else)) {
                let span = self.current_span();
                if !self.saw_err_token_in_stmt {
                    self.emit_stmt_error(Diagnostic::error(
                        "E0100",
                        "expected `else` in conditional expression",
                        span,
                    ));
                }
                self.depth -= 1;
                return Expr::Error(then_expr.span().join(cond.span()));
            }
            self.bump(); // `else`
            let otherwise = self.parse_expr();
            self.depth -= 1;
            let span = then_expr.span().join(cond.span()).join(otherwise.span());
            Expr::IfElse {
                cond: Box::new(cond),
                then: Box::new(then_expr),
                otherwise: Box::new(otherwise),
                span,
            }
        } else {
            then_expr
        }
    }

    fn parse_or(&mut self) -> Expr {
        let mut lhs = self.parse_and();
        while matches!(self.peek_kind(), TokenKind::Kw(Kw::Or)) {
            self.bump();
            let rhs = self.parse_and();
            let span = lhs.span().join(rhs.span());
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs), span);
        }
        lhs
    }

    fn parse_and(&mut self) -> Expr {
        let mut lhs = self.parse_not();
        while matches!(self.peek_kind(), TokenKind::Kw(Kw::And)) {
            self.bump();
            let rhs = self.parse_not();
            let span = lhs.span().join(rhs.span());
            lhs = Expr::And(Box::new(lhs), Box::new(rhs), span);
        }
        lhs
    }

    fn parse_not(&mut self) -> Expr {
        if matches!(self.peek_kind(), TokenKind::Kw(Kw::Not)) {
            let not_span = self.bump().span;
            if !self.enter_depth(not_span) {
                return Expr::Error(not_span);
            }
            let inner = self.parse_not();
            self.depth -= 1;
            let span = not_span.join(inner.span());
            Expr::Not(Box::new(inner), span)
        } else {
            self.parse_comparison()
        }
    }

    fn parse_comparison(&mut self) -> Expr {
        let first = self.parse_arith();
        let mut rest = Vec::new();
        while let Some(op) = match self.peek_kind() {
            TokenKind::EqEq => Some(CmpOp::Eq),
            TokenKind::BangEq => Some(CmpOp::NotEq),
            TokenKind::Lt => Some(CmpOp::Lt),
            TokenKind::LtEq => Some(CmpOp::LtEq),
            TokenKind::Gt => Some(CmpOp::Gt),
            TokenKind::GtEq => Some(CmpOp::GtEq),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_arith();
            rest.push((op, rhs));
        }
        if rest.is_empty() {
            first
        } else {
            let span = first.span().join(rest.last().unwrap().1.span());
            Expr::Compare {
                first: Box::new(first),
                rest,
                span,
            }
        }
    }

    fn parse_arith(&mut self) -> Expr {
        let mut lhs = self.parse_term();
        while let Some(op) = match self.peek_kind() {
            TokenKind::Plus => Some(BinOp::Add),
            TokenKind::Minus => Some(BinOp::Sub),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_term();
            let span = lhs.span().join(rhs.span());
            lhs = Expr::Binary {
                op,
                l: Box::new(lhs),
                r: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    fn parse_term(&mut self) -> Expr {
        let mut lhs = self.parse_unary();
        while let Some(op) = match self.peek_kind() {
            TokenKind::Star => Some(BinOp::Mul),
            TokenKind::Slash => Some(BinOp::Div),
            _ => None,
        } {
            self.bump();
            let rhs = self.parse_unary();
            let span = lhs.span().join(rhs.span());
            lhs = Expr::Binary {
                op,
                l: Box::new(lhs),
                r: Box::new(rhs),
                span,
            };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        let op = match self.peek_kind() {
            TokenKind::Minus => Some(UnOp::Neg),
            TokenKind::Plus => Some(UnOp::Pos),
            _ => None,
        };
        if let Some(op) = op {
            let op_span = self.bump().span;
            if !self.enter_depth(op_span) {
                return Expr::Error(op_span);
            }
            let inner = self.parse_unary();
            self.depth -= 1;
            let span = op_span.join(inner.span());
            Expr::Unary {
                op,
                e: Box::new(inner),
                span,
            }
        } else {
            self.parse_power()
        }
    }

    fn parse_power(&mut self) -> Expr {
        let lhs = self.parse_postfix();
        if matches!(self.peek_kind(), TokenKind::StarStar) {
            let op_span = self.bump().span;
            if !self.enter_depth(op_span) {
                return Expr::Error(lhs.span().join(op_span));
            }
            let rhs = self.parse_unary();
            self.depth -= 1;
            let span = lhs.span().join(rhs.span());
            Expr::Binary {
                op: BinOp::Pow,
                l: Box::new(lhs),
                r: Box::new(rhs),
                span,
            }
        } else {
            lhs
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            if self.stmt_has_error {
                break;
            }
            match self.peek_kind().clone() {
                TokenKind::LParen => {
                    let lparen_span = self.bump().span;
                    if !self.enter_depth(lparen_span) {
                        return Expr::Error(expr.span().join(lparen_span));
                    }
                    let (args, close_span) = self.parse_args();
                    self.depth -= 1;
                    let mut span = expr.span().join(close_span);
                    for a in &args {
                        span = span.join(a.span);
                    }
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args,
                        span,
                    };
                }
                TokenKind::LBracket => {
                    let lbrack_span = self.bump().span;
                    if matches!(self.peek_kind(), TokenKind::RBracket) {
                        let rbrack_span = self.bump().span;
                        self.emit_stmt_error(Diagnostic::error(
                            "E0112",
                            "empty index `[]` is not allowed",
                            lbrack_span.join(rbrack_span),
                        ));
                        return Expr::Error(expr.span().join(rbrack_span));
                    }
                    if !self.enter_depth(lbrack_span) {
                        return Expr::Error(expr.span().join(lbrack_span));
                    }
                    let index = self.parse_expr();
                    if matches!(self.peek_kind(), TokenKind::Colon) {
                        let colon_span = self.bump().span;
                        self.emit_stmt_error(Diagnostic::error(
                            "E0113",
                            "slices are not supported",
                            colon_span,
                        ));
                        self.depth -= 1;
                        return Expr::Error(expr.span().join(colon_span));
                    }
                    let end_span = self.expect_closing_bracket(TokenKind::RBracket, "`]`");
                    self.depth -= 1;
                    let span = expr.span().join(index.span()).join(end_span);
                    expr = Expr::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                        span,
                    };
                }
                TokenKind::Dot => {
                    let dot_span = self.bump().span;
                    match self.peek_kind().clone() {
                        TokenKind::Ident(name) => {
                            let id_span = self.bump().span;
                            let span = expr.span().join(id_span);
                            expr = Expr::Attr {
                                base: Box::new(expr),
                                name: Ident {
                                    name,
                                    span: id_span,
                                },
                                span,
                            };
                        }
                        _ => {
                            let err_span = self.current_span();
                            if !self.saw_err_token_in_stmt {
                                self.emit_stmt_error(Diagnostic::error(
                                    "E0102",
                                    "expected identifier after `.`",
                                    err_span,
                                ));
                            }
                            return Expr::Error(expr.span().join(dot_span));
                        }
                    }
                }
                TokenKind::At => {
                    let at_span = self.bump().span;
                    match self.peek_kind().clone() {
                        TokenKind::Timeframe { n, unit } => {
                            let tf_span = self.bump().span;
                            let span = expr.span().join(tf_span);
                            expr = Expr::At {
                                base: Box::new(expr),
                                tf: Timeframe {
                                    n,
                                    unit,
                                    span: tf_span,
                                },
                                span,
                            };
                        }
                        _ => {
                            let err_span = self.current_span();
                            if !self.saw_err_token_in_stmt {
                                self.emit_stmt_error(Diagnostic::error(
                                    "E0114",
                                    "expected timeframe literal after `@`",
                                    err_span,
                                ));
                            }
                            return Expr::Error(expr.span().join(at_span));
                        }
                    }
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_args(&mut self) -> (Vec<Arg>, Span) {
        let mut args = Vec::new();
        let mut seen_kw = false;
        let mut kw_names: HashSet<String> = HashSet::new();

        if matches!(self.peek_kind(), TokenKind::RParen) {
            let end_span = self.bump().span;
            return (args, end_span);
        }

        loop {
            if self.stmt_has_error {
                return (args, self.current_span());
            }
            let is_kw = matches!(self.peek_kind(), TokenKind::Ident(_))
                && matches!(self.peek_second_kind(), Some(TokenKind::Eq));
            if is_kw {
                let id_tok = self.bump();
                let name = match id_tok.kind {
                    TokenKind::Ident(s) => Ident {
                        name: s,
                        span: id_tok.span,
                    },
                    _ => unreachable!(),
                };
                self.bump(); // `=`
                if !kw_names.insert(name.name.clone()) {
                    self.emit_stmt_error(Diagnostic::error(
                        "E0111",
                        format!("duplicate keyword argument `{}`", name.name),
                        name.span,
                    ));
                }
                seen_kw = true;
                let value = self.parse_expr();
                let span = name.span.join(value.span());
                args.push(Arg {
                    name: Some(name),
                    value,
                    span,
                });
            } else {
                let value = self.parse_expr();
                let span = value.span();
                if seen_kw {
                    self.emit_stmt_error(Diagnostic::error(
                        "E0110",
                        "positional argument cannot follow keyword argument",
                        span,
                    ));
                }
                args.push(Arg {
                    name: None,
                    value,
                    span,
                });
            }

            if matches!(self.peek_kind(), TokenKind::Comma) {
                self.bump();
                if matches!(self.peek_kind(), TokenKind::RParen) {
                    let end_span = self.bump().span;
                    return (args, end_span);
                }
            } else {
                break;
            }
        }

        let end_span = self.expect_closing_bracket(TokenKind::RParen, "`)`");
        (args, end_span)
    }

    fn expect_closing_bracket(&mut self, expected: TokenKind, label: &str) -> Span {
        if *self.peek_kind() == expected {
            return self.bump().span;
        }
        let span = self.current_span();
        if !self.stmt_has_error && !self.saw_err_token_in_stmt {
            self.emit_stmt_error(Diagnostic::error(
                "E0103",
                format!("expected closing bracket {label}"),
                span,
            ));
        }
        span
    }

    fn parse_primary(&mut self) -> Expr {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Num { value, pct } => {
                self.bump();
                Expr::Num {
                    value,
                    pct,
                    span: tok.span,
                }
            }
            TokenKind::Timeframe { n, unit } => {
                self.bump();
                Expr::Timeframe(Timeframe {
                    n,
                    unit,
                    span: tok.span,
                })
            }
            TokenKind::Str(s) => {
                self.bump();
                Expr::Str(s, tok.span)
            }
            TokenKind::Kw(Kw::True) => {
                self.bump();
                Expr::Bool(true, tok.span)
            }
            TokenKind::Kw(Kw::False) => {
                self.bump();
                Expr::Bool(false, tok.span)
            }
            TokenKind::Ident(name) => {
                self.bump();
                Expr::Name(Ident {
                    name,
                    span: tok.span,
                })
            }
            TokenKind::LParen => {
                let open_span = self.bump().span;
                if !self.enter_depth(open_span) {
                    return Expr::Error(open_span);
                }
                let inner = self.parse_expr();
                self.expect_closing_bracket(TokenKind::RParen, "`)`");
                self.depth -= 1;
                inner
            }
            _ => {
                if !self.saw_err_token_in_stmt {
                    self.emit_stmt_error(Diagnostic::error(
                        "E0101",
                        "expected expression",
                        tok.span,
                    ));
                }
                Expr::Error(tok.span)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::dump;

    fn parse_dump(src: &str) -> String {
        let out = parse(src);
        assert!(
            out.diags.is_empty(),
            "unexpected diagnostics for {src:?}: {:?}",
            out.diags
        );
        dump(&out.program)
    }

    fn assert_parse_err(src: &str, code: &str) -> ParseOutput {
        let out = parse(src);
        assert!(
            out.diags.iter().any(|d| d.code == code),
            "expected {code} for {src:?}, got {:?}",
            out.diags
        );
        out
    }

    #[test]
    fn p01_simple_assign() {
        assert_eq!(parse_dump("x = 1"), "(assign x 1)");
    }

    #[test]
    fn p02_mul_before_add() {
        assert_eq!(parse_dump("x = 1 + 2 * 3"), "(assign x (+ 1 (* 2 3)))");
    }

    #[test]
    fn p03_parens_override_precedence() {
        assert_eq!(parse_dump("x = (1 + 2) * 3"), "(assign x (* (+ 1 2) 3))");
    }

    #[test]
    fn p04_sub_left_assoc() {
        assert_eq!(parse_dump("x = a - b - c"), "(assign x (- (- a b) c))");
    }

    #[test]
    fn p05_div_mul_left_assoc() {
        assert_eq!(parse_dump("x = a / b * c"), "(assign x (* (/ a b) c))");
    }

    #[test]
    fn p06_unary_lower_than_pow() {
        assert_eq!(parse_dump("x = -2 ** 2"), "(assign x (neg (** 2 2)))");
    }

    #[test]
    fn p07_pow_right_assoc() {
        assert_eq!(parse_dump("x = 2 ** 3 ** 2"), "(assign x (** 2 (** 3 2)))");
    }

    #[test]
    fn p08_unary_in_pow_exponent() {
        assert_eq!(parse_dump("x = 2 ** -1"), "(assign x (** 2 (neg 1)))");
    }

    #[test]
    fn p09_double_neg_and_unary_pos() {
        assert_eq!(parse_dump("x = - - a"), "(assign x (neg (neg a)))");
        assert_eq!(parse_dump("x = +a"), "(assign x (pos a))");
    }

    #[test]
    fn p10_comparison() {
        assert_eq!(parse_dump("x = a < b"), "(assign x (cmp a < b))");
    }

    #[test]
    fn p11_chained_comparison() {
        assert_eq!(parse_dump("x = a < b <= c"), "(assign x (cmp a < b <= c))");
    }

    #[test]
    fn p12_arith_binds_tighter_than_cmp() {
        assert_eq!(
            parse_dump("x = a + 1 > b * 2"),
            "(assign x (cmp (+ a 1) > (* b 2)))"
        );
    }

    #[test]
    fn p13_and_binds_tighter_than_or() {
        assert_eq!(
            parse_dump("x = a and b or c"),
            "(assign x (or (and a b) c))"
        );
    }

    #[test]
    fn p14_or_and_precedence() {
        assert_eq!(
            parse_dump("x = a or b and c"),
            "(assign x (or a (and b c)))"
        );
    }

    #[test]
    fn p15_not_binds_tighter_than_and() {
        assert_eq!(parse_dump("x = not a and b"), "(assign x (and (not a) b))");
    }

    #[test]
    fn p16_cmp_binds_tighter_than_not() {
        assert_eq!(
            parse_dump("x = not a == b"),
            "(assign x (not (cmp a == b)))"
        );
    }

    #[test]
    fn p17_and_left_assoc() {
        assert_eq!(
            parse_dump("x = a and b and c"),
            "(assign x (and (and a b) c))"
        );
    }

    #[test]
    fn p18_ternary() {
        assert_eq!(parse_dump("x = a if c else b"), "(assign x (if c a b))");
    }

    #[test]
    fn p19_chained_ternary() {
        assert_eq!(
            parse_dump("x = a if c else b if d else e"),
            "(assign x (if c a (if d b e)))"
        );
    }

    #[test]
    fn p20_or_inside_ternary_branch() {
        assert_eq!(
            parse_dump("x = a or b if c else d"),
            "(assign x (if c (or a b) d))"
        );
    }

    #[test]
    fn p21_zero_arg_call() {
        assert_eq!(parse_dump("x = f()"), "(assign x (call f))");
    }

    #[test]
    fn p22_positional_call() {
        assert_eq!(
            parse_dump("x = ema(close, 12)"),
            "(assign x (call ema close 12))"
        );
    }

    #[test]
    fn p23_keyword_call() {
        assert_eq!(
            parse_dump("x = rsi(close, n=14)"),
            "(assign x (call rsi close (kw n 14)))"
        );
    }

    #[test]
    fn p24_trailing_comma_in_call() {
        assert_eq!(parse_dump("x = f(a, b,)"), "(assign x (call f a b))");
    }

    #[test]
    fn p25_positional_after_keyword() {
        assert_parse_err("x = f(a=1, b)", "E0110");
    }

    #[test]
    fn p26_duplicate_keyword() {
        assert_parse_err("x = f(a=1, a=2)", "E0111");
    }

    #[test]
    fn p27_chained_index() {
        assert_eq!(
            parse_dump("x = close[1][n + 1]"),
            "(assign x (idx (idx close 1) (+ n 1)))"
        );
    }

    #[test]
    fn p28_empty_index_and_slice() {
        assert_parse_err("x = close[]", "E0112");
        assert_parse_err("x = close[1:2]", "E0113");
    }

    #[test]
    fn p29_attr_and_missing_attr_ident() {
        assert_eq!(parse_dump("x = a.b.c"), "(assign x (attr (attr a b) c))");
        assert_parse_err("x = a.", "E0102");
    }

    #[test]
    fn p30_at_timeframe() {
        assert_eq!(parse_dump("x = close @ 1d"), "(assign x (at close 1d))");
    }

    #[test]
    fn p31_at_inside_call() {
        assert_eq!(
            parse_dump("x = ema(close @ 1d, 20)"),
            "(assign x (call ema (at close 1d) 20))"
        );
    }

    #[test]
    fn p32_at_in_comparison() {
        assert_eq!(
            parse_dump("x = close @ 1d > open @ 1d"),
            "(assign x (cmp (at close 1d) > (at open 1d)))"
        );
    }

    #[test]
    fn p33_at_binds_tighter_than_add() {
        assert_eq!(parse_dump("x = a + b @ 1d"), "(assign x (+ a (at b 1d)))");
    }

    #[test]
    fn p34_invalid_at_rhs() {
        assert_parse_err("x = close @ 5", "E0114");
        assert_parse_err("x = close @", "E0114");
    }

    #[test]
    fn p35_chained_at() {
        assert_eq!(
            parse_dump("x = close @ 1d @ 5m"),
            "(assign x (at (at close 1d) 5m))"
        );
    }

    #[test]
    fn p36_percent_expr() {
        assert_eq!(parse_dump("x = close * 1.5%"), "(assign x (* close 1.5%))");
    }

    #[test]
    fn p37_bool_and_string_exprs() {
        assert_eq!(parse_dump("x = true"), "(assign x true)");
        assert_eq!(parse_dump("x = 'abc'"), "(assign x \"abc\")");
    }

    #[test]
    fn p38_single_line_def() {
        assert_eq!(
            parse_dump("def f(x) = x * 2"),
            "(def f (x) (body) (return (* x 2)))"
        );
    }

    #[test]
    fn p39_def_default_param() {
        assert_eq!(
            parse_dump("def f(x, n=14) = x"),
            "(def f (x (n 14)) (body) (return x))"
        );
    }

    #[test]
    fn p40_def_block() {
        assert_eq!(
            parse_dump("def g(x):\n    a = x + 1\n    return a * 2"),
            "(def g (x) (body (assign a (+ x 1))) (return (* a 2)))"
        );
    }

    #[test]
    fn p41_duplicate_and_misordered_params() {
        assert_parse_err("def f(x, x) = x", "E0115");
        assert_parse_err("def f(n=1, x) = x", "E0116");
    }

    #[test]
    fn p42_def_block_errors() {
        assert_parse_err("def f(x):\n    def g(y) = y\n    return x", "E0117");
        assert_parse_err("def f(x):\n    a = x + 1", "E0118");
        assert_parse_err("def f(x):\n    x + 1\n    return x", "E0119");
    }

    #[test]
    fn p43_return_outside_def_or_not_last() {
        assert_parse_err("return 1", "E0150");
        assert_parse_err("def f(x):\n    return x\n    a = 1", "E0150");
    }

    #[test]
    fn p44_semicolon_not_supported() {
        let out = assert_parse_err("x = 1; y = 2", "E0130");
        assert!(out.diags[0].help.is_some());
    }

    #[test]
    fn p45_expression_stmt() {
        let out = assert_parse_err("ema(close, 5)", "E0152");
        assert!(out.diags[0].help.is_some());
    }

    #[test]
    fn p46_invalid_assignment_targets() {
        for src in ["1 = x", "a[1] = 2", "f(x) = 1", "a.b = 1", "true = 1"] {
            assert_parse_err(src, "E0153");
        }
    }

    #[test]
    fn p47_augmented_assignment() {
        let out = assert_parse_err("x += 1", "E0154");
        assert!(out.diags[0].help.is_some());
    }

    #[test]
    fn p48_reserved_block_keywords() {
        let out = assert_parse_err("on enter:\n    a = 1\nb = 2", "E0140");
        assert_eq!(dump(&out.program), "(error)\n(assign b 2)");

        for src in [
            "state peak:\n    a = 1",
            "score vol:\n    a = 1",
            "portfolio:\n    a = 1",
            "while long:\n    a = 1",
            "use plugin 'x'",
        ] {
            assert_parse_err(src, "E0140");
        }
    }

    #[test]
    fn p49_block_keywords_as_variable_names() {
        assert_eq!(parse_dump("state = 1"), "(assign state 1)");
        assert_eq!(parse_dump("on = 2"), "(assign on 2)");
        assert_eq!(parse_dump("use = 3"), "(assign use 3)");
    }

    #[test]
    fn p50_empty_and_comment_only_programs() {
        assert_eq!(parse_dump(""), "");
        assert_eq!(parse_dump("# just a comment\n\n# another"), "");
    }
}
