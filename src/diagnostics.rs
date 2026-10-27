use crate::source::Span;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Option<Span>,
}

#[derive(Debug, Error)]
pub enum FrontendError {
    #[error("lex error {code} at {line}:{column}: {message}")]
    Lex {
        code: &'static str,
        message: String,
        line: u32,
        column: u32,
    },
    #[error("parse error {code} at {line}:{column}: {message}")]
    Parse {
        code: &'static str,
        message: String,
        line: u32,
        column: u32,
    },
    #[error("validation failed: {0:?}")]
    Validation(Vec<Diagnostic>),
}
