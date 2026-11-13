/// Represents a single market candle.
/// All timestamps are integer Unix epoch timestamps in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candle {
    /// Unix timestamp in seconds since the Unix epoch (1970-01-01T00:00:00Z).
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Candle {
    pub fn is_valid(&self, previous: Option<&Self>) -> Result<(), &'static str> {
        if ![self.open, self.high, self.low, self.close, self.volume]
            .iter()
            .all(|x| x.is_finite())
        {
            return Err("all price and volume fields must be finite");
        }
        if self.low > self.high
            || self.low > self.open
            || self.low > self.close
            || self.open > self.high
            || self.close > self.high
        {
            return Err("expected low <= {open, close} <= high");
        }
        if self.volume < 0.0 {
            return Err("volume must be non-negative");
        }
        if previous.is_some_and(|p| self.timestamp <= p.timestamp) {
            return Err("timestamp must increase");
        }
        Ok(())
    }
}
