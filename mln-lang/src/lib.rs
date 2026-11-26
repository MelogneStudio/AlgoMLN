#![forbid(unsafe_code)]

//! Front-end for the LangMelogne (`.mln`) strategy language:
//! lexer, parser, AST, canonical dump, and diagnostics.

pub mod ast;
pub mod diag;
pub mod dump;
pub mod error_codes;
pub mod lexer;
pub mod parser;
pub mod span;

pub use ast::{Arg, BinOp, CmpOp, Def, Expr, Ident, Param, Program, Stmt, Timeframe, UnOp};
pub use diag::{line_col, render, Diagnostic, Severity};
pub use dump::dump;
pub use error_codes::{ErrorCodeInfo, ERROR_CODES};
pub use lexer::{lex, tokens_str, AugOp, Kw, LexOutput, TfUnit, Token, TokenKind};
pub use parser::{parse, ParseOutput};
pub use span::Span;
