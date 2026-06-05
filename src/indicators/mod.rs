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

/// Relative volume: current candle volume divided by the mean volume of the
/// previous `period` candles. The current candle is excluded from the average.
pub fn rel_vol(candles: &[crate::models::Candle], period: usize) -> Vec<f64> {
    assert!(period > 0, "rel_vol period must be greater than zero");

    let mut values = Vec::with_capacity(candles.len());
    let mut sum = 0.0;

    for (index, candle) in candles.iter().enumerate() {
        if index < period {
            values.push(0.0);
        } else {
            let mean = sum / period as f64;
            values.push(if mean == 0.0 {
                0.0
            } else {
                candle.volume / mean
            });
        }

        sum += candle.volume;
        if index + 1 > period {
            sum -= candles[index + 1 - period].volume;
        }
    }

    values
}

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

    fn candle_with_vol(volume: f64) -> Candle {
        candle(0, 1.0, 1.0, 1.0, 1.0, volume)
    }

    #[test]
    fn rel_vol_returns_zero_for_insufficient_history() {
        let candles = vec![
            candle(0, 1.0, 1.0, 1.0, 1.0, 100.0),
            candle(1, 1.0, 1.0, 1.0, 1.0, 200.0),
            candle(2, 1.0, 1.0, 1.0, 1.0, 150.0),
        ];
        let result = super::rel_vol(&candles, 5);
        assert!(result.iter().all(|&value| value == 0.0));
    }

    #[test]
    fn rel_vol_correct_value() {
        let vols = [100.0, 100.0, 100.0, 100.0, 200.0];
        let candles = vols
            .iter()
            .map(|&volume| candle_with_vol(volume))
            .collect::<Vec<_>>();
        let result = super::rel_vol(&candles, 4);
        assert!((result[4] - 2.0).abs() < 0.001);
    }

    #[test]
    fn rel_vol_equal_volume_returns_one() {
        let candles = (0..10)
            .map(|_| candle_with_vol(500.0))
            .collect::<Vec<_>>();
        let result = super::rel_vol(&candles, 3);
        for &value in &result[3..] {
            assert!((value - 1.0).abs() < 0.001);
        }
    }

    #[test]
    fn rel_vol_zero_average_returns_zero_not_nan() {
        let candles = (0..5)
            .map(|_| candle_with_vol(0.0))
            .collect::<Vec<_>>();
        let result = super::rel_vol(&candles, 3);
        assert!(result.iter().all(|value| value.is_finite()));
    }
}
