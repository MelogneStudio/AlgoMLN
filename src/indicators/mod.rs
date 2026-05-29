pub mod atr;
pub mod bb;
pub mod ma;
pub mod rsi;
pub mod vwap;

pub use atr::atr;
pub use bb::{bbands, BBands};
pub use ma::{ema, ma};
pub use rsi::rsi;
pub use vwap::{vwap, VwapPoint};

#[cfg(test)]
pub(crate) mod tests {
    use crate::models::Candle;

    pub fn candles_from_closes(closes: &[f64]) -> Vec<Candle> {
        closes
            .iter()
            .enumerate()
            .map(|(index, close)| candle(index as i64, *close, *close, *close, *close, 1.0))
            .collect()
    }

    pub fn candle(
        timestamp: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    ) -> Candle {
        Candle {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
        }
    }
}
