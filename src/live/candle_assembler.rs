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

    /// Feed one tick. Returns zero or more candles that *closed* on this tick:
    /// - One candle for the minute the previous ticks were accumulating in
    ///   (its OHLCV is whatever prices/volume arrived in that minute).
    /// - One zero-volume *gap candle* per minute that was skipped between the
    ///   previous minute and the new tick's minute. Gap candles use the last
    ///   seen close as their open/high/low/close so indicators that read
    ///   `PrevClose` see a contiguous series across feed gaps.
    ///
    /// Returns an empty `Vec` when the tick falls inside the in-progress
    /// minute or when the tick's symbol doesn't match the assembler's symbol.
    pub fn feed(&mut self, tick: &Tick) -> Vec<Candle> {
        if tick.symbol != self.symbol {
            return Vec::new();
        }

        let tick_minute = tick.timestamp / 60_000 * 60_000;

        let mut emitted = Vec::new();

        match self.current_minute {
            None => {
                self.start_new_candle(tick_minute, tick);
            }
            Some(current_minute) if tick_minute == current_minute => {
                self.high = self.high.max(tick.ltp);
                self.low = self.low.min(tick.ltp);
                self.close = tick.ltp;
                self.volume += tick.volume as f64;
            }
            Some(current_minute) => {
                // Close the in-progress minute.
                emitted.push(Candle {
                    timestamp: current_minute,
                    open: self.open,
                    high: self.high,
                    low: self.low,
                    close: self.close,
                    volume: self.volume,
                });

                // Emit one zero-volume gap candle per missing minute between
                // the closed minute and the new tick's minute. Each gap candle
                // is flat at the last seen close so downstream indicators that
                // rely on a uniform one-candle-per-period cadence see a
                // continuous series rather than a discontinuity.
                let last_close = self.close;
                let mut gap_minute = current_minute + 60_000;
                while gap_minute < tick_minute {
                    emitted.push(Candle {
                        timestamp: gap_minute,
                        open: last_close,
                        high: last_close,
                        low: last_close,
                        close: last_close,
                        volume: 0.0,
                    });
                    gap_minute += 60_000;
                }

                self.start_new_candle(tick_minute, tick);
            }
        }

        emitted
    }

    /// Emit the in-progress minute's candle, if any, and reset state.
    ///
    /// The live tick loop calls this from its `cancel.cancelled()` branch and
    /// from its `RecvError::Closed` branch so the last partial minute's
    /// prices and volume aren't silently dropped when the session ends or
    /// the feed shuts down.
    ///
    /// Returns `None` if no tick has ever been fed (no minute in progress).
    /// Calling `flush` a second time after the first non-`None` return also
    /// returns `None`.
    pub fn flush(&mut self) -> Option<Candle> {
        let current_minute = self.current_minute?;
        let candle = Candle {
            timestamp: current_minute,
            open: self.open,
            high: self.high,
            low: self.low,
            close: self.close,
            volume: self.volume,
        };
        self.current_minute = None;
        self.open = 0.0;
        self.high = 0.0;
        self.low = 0.0;
        self.close = 0.0;
        self.volume = 0.0;
        Some(candle)
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

        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());
    }

    #[test]
    fn test_same_minute_no_candle() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());
        assert!(assembler.feed(&tick(101.0, 11, 61_000)).is_empty());
        assert!(assembler.feed(&tick(99.0, 12, 119_999)).is_empty());
    }

    #[test]
    fn test_new_minute_returns_candle() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());
        assert!(assembler.feed(&tick(105.0, 15, 61_000)).is_empty());
        assert!(assembler.feed(&tick(98.0, 20, 119_999)).is_empty());

        let candles = assembler.feed(&tick(110.0, 25, 120_000));
        assert_eq!(candles.len(), 1, "exactly one minute closed, no gaps");

        let candle = &candles[0];
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

        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());
        assert!(assembler.feed(&tick(105.0, 10, 61_000)).is_empty());
        assert!(assembler.feed(&tick(98.0, 10, 62_000)).is_empty());

        let candles = assembler.feed(&tick(101.0, 10, 120_000));
        assert_eq!(candles.len(), 1);

        assert_eq!(candles[0].high, 105.0);
        assert_eq!(candles[0].low, 98.0);
    }

    // B2 (audit): a multi-minute feed gap previously caused the assembler to
    // emit only the last-traded minute and silently drop the in-between
    // minutes, giving the engine a non-contiguous series. After the fix the
    // assembler emits a zero-volume gap candle for every minute strictly
    // between the closed minute and the new tick's minute, flat at the last
    // seen close.
    #[test]
    fn test_gap_emits_zero_volume_minutes() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        // Tick in minute 1 [60_000, 120_000): last close will be 98.0.
        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());
        assert!(assembler.feed(&tick(105.0, 15, 61_000)).is_empty());
        assert!(assembler.feed(&tick(98.0, 20, 119_999)).is_empty());

        // Next tick at minute 3 (180_000). The new tick *starts* minute 3,
        // so only minute 2 is a gap fill; minute 3 is the new in-progress
        // candle. Minute 1 (real) + minute 2 (gap) = 2 emitted candles.
        let candles = assembler.feed(&tick(110.0, 25, 180_000));
        assert_eq!(candles.len(), 2, "minute 1 (real) + minute 2 (gap)");

        // Minute 1: real OHLCV.
        let m1 = &candles[0];
        assert_eq!(m1.timestamp, 60_000);
        assert_eq!(m1.open, 100.0);
        assert_eq!(m1.high, 105.0);
        assert_eq!(m1.low, 98.0);
        assert_eq!(m1.close, 98.0);
        assert_eq!(m1.volume, 45.0);

        // Minute 2: zero-volume gap candle flat at last close.
        let m2 = &candles[1];
        assert_eq!(m2.timestamp, 120_000);
        assert_eq!(m2.open, 98.0);
        assert_eq!(m2.high, 98.0);
        assert_eq!(m2.low, 98.0);
        assert_eq!(m2.close, 98.0);
        assert_eq!(m2.volume, 0.0);
    }

    #[test]
    fn test_multi_minute_gap_emits_one_per_missing_minute() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());

        // 5-minute jump: from minute 1 to minute 6 (360_000). Should emit
        // minute 1 (real) + minutes 2, 3, 4, 5 (4 gap fills) = 5 candles.
        let candles = assembler.feed(&tick(110.0, 25, 360_000));
        assert_eq!(
            candles.len(),
            5,
            "one real minute + four gap fills for a 5-minute jump"
        );

        // The closed minute is at timestamp 60_000; gap minutes start at 120_000.
        assert_eq!(candles[0].timestamp, 60_000);
        assert_eq!(candles[0].volume, 10.0, "real minute has the original volume");
        for (i, gap) in candles.iter().enumerate().skip(1) {
            assert_eq!(gap.timestamp, 60_000 + (i as i64) * 60_000);
            assert_eq!(gap.volume, 0.0, "gap candle {} has zero volume", i);
            assert_eq!(gap.open, 100.0);
            assert_eq!(gap.high, 100.0);
            assert_eq!(gap.low, 100.0);
            assert_eq!(gap.close, 100.0, "gap candle flat at last close");
        }
    }

    #[test]
    fn test_flush_emits_partial_candle() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        // Two ticks in the same minute — minute stays in progress.
        assert!(assembler.feed(&tick(100.0, 10, 60_000)).is_empty());
        assert!(assembler.feed(&tick(105.0, 20, 61_000)).is_empty());

        // Flush emits the partial minute's accumulated prices/volume.
        let candle = assembler
            .flush()
            .expect("in-progress minute should be flushed");

        assert_eq!(candle.timestamp, 60_000);
        assert_eq!(candle.open, 100.0);
        assert_eq!(candle.high, 105.0);
        assert_eq!(candle.low, 100.0);
        assert_eq!(candle.close, 105.0);
        assert_eq!(candle.volume, 30.0);

        // A second flush is a no-op (no minute in progress).
        assert!(assembler.flush().is_none());
    }

    #[test]
    fn test_flush_before_any_tick_is_none() {
        let mut assembler = CandleAssembler::new("NIFTY".to_string());

        // No tick has been fed — nothing to flush.
        assert!(assembler.flush().is_none());
    }
}
