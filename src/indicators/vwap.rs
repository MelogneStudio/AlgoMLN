use super::{candle::Candle, state::Indicator};
pub(super) struct Vwap {
    day: Option<i64>,
    pv: f64,
    v: f64,
}
impl Vwap {
    pub fn new() -> Self {
        Self {
            day: None,
            pv: 0.,
            v: 0.,
        }
    }
}
impl Indicator for Vwap {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        // Timestamps are epoch seconds; +19800 shifts to IST so the accumulator
        // resets at IST midnight, matching the session convention used elsewhere.
        let day = (c.timestamp + 19_800).div_euclid(86_400);
        if self.day != Some(day) {
            self.day = Some(day);
            self.pv = 0.;
            self.v = 0.
        }
        let typ = (c.high + c.low + c.close) / 3.;
        self.pv += typ * c.volume;
        self.v += c.volume;
        (self.v > 0.).then(|| self.pv / self.v)
    }
}
