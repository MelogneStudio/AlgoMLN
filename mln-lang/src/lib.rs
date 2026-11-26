#![forbid(unsafe_code)]

//! Front-end for the LangMelogne (`.mln`) strategy language:
//! lexer, parser, AST, canonical dump, and diagnostics.

pub mod diag;
pub mod error_codes;
pub mod lexer;
pub mod span;

pub use diag::{line_col, render, Diagnostic, Severity};
pub use error_codes::{ErrorCodeInfo, ERROR_CODES};
pub use lexer::{lex, tokens_str, AugOp, Kw, LexOutput, TfUnit, Token, TokenKind};
pub use span::Span;
