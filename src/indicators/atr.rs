use super::{candle::Candle, state::Indicator};
pub(super) struct Atr {
    p: usize,
    n: usize,
    sum: f64,
    prev: Option<f64>,
    value: Option<f64>,
}
impl Atr {
    pub fn new(p: usize) -> Self {
        Self {
            p,
            n: 0,
            sum: 0.,
            prev: None,
            value: None,
        }
    }
}
impl Indicator for Atr {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        if let Some(prev) = self.prev {
            let tr = (c.high - c.low)
                .max((c.high - prev).abs())
                .max((c.low - prev).abs());
            if let Some(v) = self.value {
                self.value = Some((v * (self.p - 1) as f64 + tr) / self.p as f64)
            } else {
                self.n += 1;
                self.sum += tr;
                if self.n == self.p {
                    self.value = Some(self.sum / self.p as f64)
                }
            }
        }
        self.prev = Some(c.close);
        self.value
    }
}
