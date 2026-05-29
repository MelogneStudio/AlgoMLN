use crate::{indicators::ma::ema_values, models::Candle};

pub fn rsi(candles: &[Candle], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; candles.len()];
    }
    if candles.len() < 2 {
        return vec![None; candles.len()];
    }

    let mut gains = Vec::with_capacity(candles.len() - 1);
    let mut losses = Vec::with_capacity(candles.len() - 1);

    for window in candles.windows(2) {
        let change = window[1].close - window[0].close;
        gains.push(change.max(0.0));
        losses.push((-change).max(0.0));
    }

    let avg_gains = ema_values(gains, period);
    let avg_losses = ema_values(losses, period);
    let mut output = Vec::with_capacity(candles.len());
    output.push(None);

    for (avg_gain, avg_loss) in avg_gains.into_iter().zip(avg_losses) {
        output.push(match (avg_gain, avg_loss) {
            (Some(gain), Some(loss)) if loss == 0.0 && gain == 0.0 => Some(50.0),
            (Some(_), Some(loss)) if loss == 0.0 => Some(100.0),
            (Some(gain), Some(loss)) => {
                let relative_strength = gain / loss;
                Some(100.0 - (100.0 / (1.0 + relative_strength)))
            }
            _ => None,
        });
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::tests::candles_from_closes;

    #[test]
    fn rsi_pads_until_enough_changes_exist() {
        let candles = candles_from_closes(&[1.0, 2.0, 3.0, 2.0, 4.0]);

        let values = rsi(&candles, 3);

        assert_eq!(values[0], None);
        assert_eq!(values[1], None);
        assert_eq!(values[2], None);
        assert!(values[3].is_some());
        assert!(values[4].is_some());
    }

    #[test]
    fn rsi_is_one_hundred_when_there_are_no_losses() {
        let candles = candles_from_closes(&[1.0, 2.0, 3.0, 4.0]);

        assert_eq!(rsi(&candles, 2), vec![None, None, Some(100.0), Some(100.0)]);
    }
}
