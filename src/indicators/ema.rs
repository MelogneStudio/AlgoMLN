use super::{candle::Candle, state::Indicator};
pub(super) struct Ema {
    p: usize,
    n: usize,
    sum: f64,
    value: Option<f64>,
    alpha: f64,
}
impl Ema {
    pub fn new(p: usize) -> Self {
        Self {
            p,
            n: 0,
            sum: 0.,
            value: None,
            alpha: 2. / (p as f64 + 1.),
        }
    }
}
impl Indicator for Ema {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        if let Some(v) = self.value {
            let next = self.alpha * c.close + (1. - self.alpha) * v;
            self.value = next.is_finite().then_some(next)
        } else {
            self.n += 1;
            self.sum += c.close;
            if self.n == self.p {
                let v = self.sum / self.p as f64;
                if v.is_finite() {
                    self.value = Some(v)
                }
            }
        }
        self.value
    }
}
