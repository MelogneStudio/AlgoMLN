use crate::models::Candle;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VwapPoint {
    pub value: f64,
}

pub fn vwap(candles: &[Candle]) -> Vec<Option<VwapPoint>> {
    let mut output = Vec::with_capacity(candles.len());
    let mut current_day = None;
    let mut cumulative_price_volume = 0.0;
    let mut cumulative_volume = 0.0;

    for candle in candles {
        let candle_day = candle.timestamp.div_euclid(86_400_000);
        if current_day != Some(candle_day) {
            current_day = Some(candle_day);
            cumulative_price_volume = 0.0;
            cumulative_volume = 0.0;
        }

        let typical_price = (candle.high + candle.low + candle.close) / 3.0;
        cumulative_price_volume += typical_price * candle.volume;
        cumulative_volume += candle.volume;

        if cumulative_volume == 0.0 {
            output.push(None);
        } else {
            output.push(Some(VwapPoint {
                value: cumulative_price_volume / cumulative_volume,
            }));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::tests::candle;

    #[test]
    fn vwap_resets_on_new_utc_day() {
        let day = 86_400_000;
        let candles = vec![
            candle(0, 10.0, 12.0, 9.0, 9.0, 10.0),
            candle(1_000, 20.0, 24.0, 18.0, 18.0, 10.0),
            candle(day, 100.0, 120.0, 90.0, 90.0, 5.0),
        ];

        let values = vwap(&candles);

        assert_eq!(values[0], Some(VwapPoint { value: 10.0 }));
        assert_eq!(values[1], Some(VwapPoint { value: 15.0 }));
        assert_eq!(values[2], Some(VwapPoint { value: 100.0 }));
    }
}
