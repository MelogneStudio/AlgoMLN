use crate::models::Candle;

pub fn ma(candles: &[Candle], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; candles.len()];
    }

    let mut values = Vec::with_capacity(candles.len());
    let mut sum = 0.0;

    for (index, candle) in candles.iter().enumerate() {
        sum += candle.close;

        if index >= period {
            sum -= candles[index - period].close;
        }

        if index + 1 >= period {
            values.push(Some(sum / period as f64));
        } else {
            values.push(None);
        }
    }

    values
}

pub fn ema(candles: &[Candle], period: usize) -> Vec<Option<f64>> {
    ema_values(candles.iter().map(|candle| candle.close), period)
}

pub(crate) fn ema_values(values: impl IntoIterator<Item = f64>, period: usize) -> Vec<Option<f64>> {
    let values = values.into_iter().collect::<Vec<_>>();
    if period == 0 {
        return vec![None; values.len()];
    }

    let mut output = Vec::with_capacity(values.len());
    let mut warmup_sum = 0.0;
    let mut previous_ema = None;
    let multiplier = 2.0 / (period as f64 + 1.0);

    for (index, value) in values.iter().copied().enumerate() {
        if index + 1 < period {
            warmup_sum += value;
            output.push(None);
            continue;
        }

        if index + 1 == period {
            warmup_sum += value;
            let initial = warmup_sum / period as f64;
            previous_ema = Some(initial);
            output.push(Some(initial));
            continue;
        }

        let next = (value - previous_ema.unwrap()) * multiplier + previous_ema.unwrap();
        previous_ema = Some(next);
        output.push(Some(next));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::tests::candles_from_closes;

    #[test]
    fn ma_pads_until_period_is_available() {
        let candles = candles_from_closes(&[1.0, 2.0, 3.0, 4.0]);

        assert_eq!(ma(&candles, 3), vec![None, None, Some(2.0), Some(3.0)]);
    }

    #[test]
    fn ema_starts_with_sma_seed() {
        let candles = candles_from_closes(&[1.0, 2.0, 3.0, 4.0]);

        assert_eq!(ema(&candles, 3), vec![None, None, Some(2.0), Some(3.0)]);
    }
}
