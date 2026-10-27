//! AlgoMLN strategy language frontend. This crate parses and validates strategy
//! descriptions; it deliberately does not evaluate market data or execute trades.

pub mod ast;
pub mod diagnostics;
pub mod lexer;
pub mod parser;
pub mod source;
pub mod validator;

pub use ast::Strategy;
pub use diagnostics::{Diagnostic, FrontendError};

/// Parse and validate a complete AlgoMLN source file.
pub fn compile(source: &str) -> Result<Strategy, FrontendError> {
    let tokens = lexer::lex(source)?;
    let strategy = parser::parse(&tokens)?;
    validator::validate(&strategy)?;
    Ok(strategy)
}
