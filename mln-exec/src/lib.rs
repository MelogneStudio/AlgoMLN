#![forbid(unsafe_code)]

//! Reference oracle interpreter and incremental execution engine for LangMelogne IR.

pub mod executor;
pub mod oracle;
pub(crate) mod rolling;
pub mod state;

pub use executor::Executor;
pub use oracle::{
    eval_oracle, eval_source_oracle, ExecError, OracleOutput, SeriesInputs, ValueSeries,
};
pub use state::{Bar, BarOutput, ExecutorSnapshot, NodeVal};
