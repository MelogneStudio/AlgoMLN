use crate::{indicators::ma::ema, models::Candle};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBands {
    pub upper: f64,
    pub mid: f64,
    pub lower: f64,
}

pub fn bbands(candles: &[Candle], period: usize, std_dev_multiplier: f64) -> Vec<Option<BBands>> {
    if period == 0 {
        return vec![None; candles.len()];
    }

    let mids = ema(candles, period);
    let mut output = Vec::with_capacity(candles.len());

    for (index, mid) in mids.into_iter().enumerate() {
        let Some(mid) = mid else {
            output.push(None);
            continue;
        };

        let start = index + 1 - period;
        let variance = candles[start..=index]
            .iter()
            .map(|candle| (candle.close - mid).powi(2))
            .sum::<f64>()
            / period as f64;
        let std_dev = variance.sqrt();

        output.push(Some(BBands {
            upper: mid + std_dev_multiplier * std_dev,
            mid,
            lower: mid - std_dev_multiplier * std_dev,
        }));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::tests::candles_from_closes;

    #[test]
    fn bbands_returns_upper_mid_lower_after_warmup() {
        let candles = candles_from_closes(&[1.0, 2.0, 3.0]);

        let values = bbands(&candles, 3, 2.0);

        assert_eq!(values[0], None);
        assert_eq!(values[1], None);
        let bands = values[2].unwrap();
        assert_eq!(bands.mid, 2.0);
        assert!(bands.upper > bands.mid);
        assert!(bands.lower < bands.mid);
    }
}
