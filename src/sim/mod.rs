//! Deterministic, local historical simulation primitives.
//!
//! This module depends on the strategy engine and execution port only. Data
//! sources are replaceable; the file source never performs network access.

pub mod broker;
pub mod export;
pub mod runner;
pub mod source;
pub mod types;

#[cfg(test)]
mod tests;

pub use broker::SimBroker;
pub use export::write_results;
pub use runner::{run, run_precomputed};
pub use source::{
    seeded_synthetic_candles, CandleSource, CandleStream, FileSource, SourceError, SourceStats,
};
pub use types::{
    DateRange, EquityPoint, FeeModel, Fill, Fingerprint, GapPolicy, OpenPosition, RunConfig,
    RunResult, SimError, StageTimings, SymbolEquityPoint, Timeframe, Trade,
};
