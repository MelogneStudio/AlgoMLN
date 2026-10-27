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

impl FrontendError {
    pub fn stage(&self) -> diagnostics::ErrorStage {
        match self {
            Self::Lex { .. } => diagnostics::ErrorStage::Lexing,
            Self::Parse { .. } => diagnostics::ErrorStage::Parsing,
            Self::Validation(_) => diagnostics::ErrorStage::Validation,
        }
    }
    pub fn code(&self) -> &str {
        match self {
            Self::Lex { code, .. } | Self::Parse { code, .. } => code,
            Self::Validation(ds) => ds.first().map_or("VAL_UNKNOWN", |d| d.code),
        }
    }
    pub fn span(&self) -> Option<source::Span> {
        match self {
            Self::Lex { span, .. } | Self::Parse { span, .. } => Some(*span),
            Self::Validation(ds) => ds.first().and_then(|d| d.span),
        }
    }
}

/// Parse and validate a complete AlgoMLN source file.
pub fn compile(source: &str) -> Result<Strategy, FrontendError> {
    let tokens = lexer::lex(source)?;
    let strategy = parser::parse(&tokens)?;
    validator::validate(&strategy)?;
    Ok(strategy)
}
