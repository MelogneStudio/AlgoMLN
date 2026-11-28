#![forbid(unsafe_code)]

//! Reference oracle interpreter and incremental execution engine for LangMelogne IR.
//!
//! ```rust
//! use mln_exec::{Bar, Executor};
//! use mln_ir::lower_default;
//! use mln_lang::parse;
//!
//! let src = "s = sma(close, 3)\nentry = close > s\nexit = close < open";
//! let parsed = parse(src);
//! assert!(parsed.diags.is_empty());
//! let ir = lower_default(&parsed.program).unwrap();
//!
//! let mut exec = Executor::new(&ir).unwrap();
//! let bars = [
//!     Bar::new(1_700_000_000, 10.0, 11.0, 9.5, 10.5, 100.0),
//!     Bar::new(1_700_000_060, 11.0, 12.0, 10.5, 11.5, 120.0),
//!     Bar::new(1_700_000_120, 12.0, 13.0, 11.5, 12.5, 140.0),
//! ];
//! let mut out = Vec::with_capacity(bars.len());
//! exec.run_chunk_into(&bars, &mut out).unwrap();
//! assert_eq!(out.len(), 3);
//! assert_eq!(out[2].entry_signal, 1);
//!
//! let snap = exec.snapshot();
//! let mut resumed = Executor::new(&ir).unwrap();
//! resumed.restore(&snap).unwrap();
//! ```

pub mod executor;
pub mod oracle;
pub(crate) mod rolling;
pub mod state;

pub use executor::Executor;
pub use oracle::{
    eval_oracle, eval_source_oracle, ExecError, OracleOutput, SeriesInputs, ValueSeries,
};
pub use state::{Bar, BarOutput, ExecutorSnapshot, NodeVal};
