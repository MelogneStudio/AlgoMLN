use crate::models::Candle;

pub fn atr(candles: &[Candle], period: usize) -> Vec<Option<f64>> {
    if period == 0 || candles.is_empty() {
        return vec![None; candles.len()];
    }

    let true_ranges = true_ranges(candles);
    let mut output = Vec::with_capacity(candles.len());
    let mut sum = 0.0;
    let mut previous_atr = None;

    for (index, true_range) in true_ranges.into_iter().enumerate() {
        if index + 1 < period {
            sum += true_range;
            output.push(None);
            continue;
        }

        if index + 1 == period {
            sum += true_range;
            let initial = sum / period as f64;
            previous_atr = Some(initial);
            output.push(Some(initial));
            continue;
        }

        let next = ((previous_atr.unwrap() * (period as f64 - 1.0)) + true_range) / period as f64;
        previous_atr = Some(next);
        output.push(Some(next));
    }

    output
}

fn true_ranges(candles: &[Candle]) -> Vec<f64> {
    candles
        .iter()
        .enumerate()
        .map(|(index, candle)| {
            if index == 0 {
                candle.high - candle.low
            } else {
                let previous_close = candles[index - 1].close;
                (candle.high - candle.low)
                    .max((candle.high - previous_close).abs())
                    .max((candle.low - previous_close).abs())
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::tests::candle;

    #[test]
    fn atr_uses_true_range_and_wilder_smoothing() {
        let candles = vec![
            candle(0, 9.0, 12.0, 8.0, 10.0, 1.0),
            candle(1, 10.0, 15.0, 9.0, 14.0, 1.0),
            candle(2, 14.0, 16.0, 13.0, 15.0, 1.0),
            candle(3, 15.0, 18.0, 14.0, 17.0, 1.0),
        ];

        assert_eq!(
            atr(&candles, 3),
            vec![None, None, Some(13.0 / 3.0), Some(4.222222222222222)]
        );
    }
}
