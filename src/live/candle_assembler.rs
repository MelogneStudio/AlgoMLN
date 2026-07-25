use crate::models::{Candle, Tick};

pub struct CandleAssembler {
    symbol: String,
    current_minute: Option<i64>,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl CandleAssembler {
    pub fn new(symbol: String) -> Self {
        Self {
            symbol,
            current_minute: None,
            open: 0.0,
            high: 0.0,
            low: 0.0,
            close: 0.0,
            volume: 0.0,
        }
    }

    /// Feed one tick. Returns `Some(completed_candle)` when the first tick of
    /// a new minute closes the previous minute's candle.
    pub fn feed(&mut self, tick: &Tick) -> Option<Candle> {
        if tick.symbol != self.symbol {
            return None;
        }

        let tick_minute = tick.timestamp / 60_000 * 60_000;

        match self.current_minute {
            None => {
                self.start_new_candle(tick_minute, tick);
                None
            }
            Some(current_minute) if tick_minute == current_minute => {
                self.high = self.high.max(tick.ltp);
                self.low = self.low.min(tick.ltp);
                self.close = tick.ltp;
                self.volume += tick.volume as f64;
                None
            }
            Some(current_minute) => {
                let completed = Candle {
                    timestamp: current_minute,
                    open: self.open,
                    high: self.high,
                    low: self.low,
                    close: self.close,
                    volume: self.volume,
                };
                self.start_new_candle(tick_minute, tick);
                Some(completed)
            }
        }
    }

    fn start_new_candle(&mut self, minute: i64, tick: &Tick) {
        self.current_minute = Some(minute);
        self.open = tick.ltp;
        self.high = tick.ltp;
        self.low = tick.ltp;
        self.close = tick.ltp;
        self.volume = tick.volume as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(price: f64, volume: u64, timestamp: i64) -> Tick {
        Tick {
            symbol: "NIFTY".to_string(),
            ltp: price,
            volume,
            timestamp,
        }
    }

    #[test]
    fn test_first_tick_no_candle() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert_eq!(assembler.feed(&tick(100.0, 10, 60_000)), None);
    }

    #[test]
    fn test_same_minute_no_candle() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert_eq!(assembler.feed(&tick(100.0, 10, 60_000)), None);
        assert_eq!(assembler.feed(&tick(101.0, 11, 61_000)), None);
        assert_eq!(assembler.feed(&tick(99.0, 12, 119_999)), None);
    }

    #[test]
    fn test_new_minute_returns_candle() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert_eq!(assembler.feed(&tick(100.0, 10, 60_000)), None);
        assert_eq!(assembler.feed(&tick(105.0, 15, 61_000)), None);
        assert_eq!(assembler.feed(&tick(98.0, 20, 119_999)), None);

        let candle = assembler
            .feed(&tick(110.0, 25, 120_000))
            .expect("first tick of new minute should close previous candle");

        assert_eq!(candle.timestamp, 60_000);
        assert_eq!(candle.open, 100.0);
        assert_eq!(candle.high, 105.0);
        assert_eq!(candle.low, 98.0);
        assert_eq!(candle.close, 98.0);
        assert_eq!(candle.volume, 45.0);
    }

    #[test]
    fn test_high_low_tracked() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert_eq!(assembler.feed(&tick(100.0, 10, 60_000)), None);
        assert_eq!(assembler.feed(&tick(105.0, 10, 61_000)), None);
        assert_eq!(assembler.feed(&tick(98.0, 10, 62_000)), None);

        let candle = assembler
            .feed(&tick(101.0, 10, 120_000))
            .expect("new minute should close previous candle");

        assert_eq!(candle.high, 105.0);
        assert_eq!(candle.low, 98.0);
    }
}
