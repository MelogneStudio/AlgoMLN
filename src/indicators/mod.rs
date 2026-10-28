//! Incremental, single-threaded market indicators.
mod atr;
mod bollinger;
mod ema;
mod provider;
mod rel_vol;
mod rsi;
mod sma;
mod state;
mod vwap;

pub mod candle;
pub mod error;
pub mod spec;
pub use candle::Candle;
pub use error::IndicatorError;
pub use provider::{IncrementalProvider, IndicatorHandle, IndicatorProvider};
pub use spec::{IndicatorSpec, IndicatorType, PriceField};

#[cfg(test)]
mod tests;
