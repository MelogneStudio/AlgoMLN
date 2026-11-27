#![forbid(unsafe_code)]

//! Reference oracle interpreter and execution engine for LangMelogne IR.

pub mod oracle;
pub(crate) mod rolling;

pub use oracle::{
    eval_oracle, eval_source_oracle, ExecError, OracleOutput, SeriesInputs, ValueSeries,
};
