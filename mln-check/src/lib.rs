#![forbid(unsafe_code)]

//! Static analysis for LangMelogne (`.mln`): name resolution, type checking,
//! constant folding, clock checking, and warm-up reporting.

pub mod error_codes;
pub mod types;

pub use error_codes::PHASE2_CODES;
pub use types::{
    CheckConfig, CheckedExpr, CheckedProgram, CheckedStmt, Clock, ConstVal, DefInstance,
    OutputWarmup, Tf, Ty, WarmupReport,
};
