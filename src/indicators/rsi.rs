use super::{candle::Candle, state::Indicator};
pub(super) struct Rsi {
    p: usize,
    p_f64: f64,
    p_minus_1_f64: f64,
    n: usize,
    prev: Option<f64>,
    gain: f64,
    loss: f64,
    avg: Option<(f64, f64)>,
}
impl Rsi {
    pub fn new(p: usize) -> Self {
        Self {
            p,
            p_f64: p as f64,
            p_minus_1_f64: (p - 1) as f64,
            n: 0,
            prev: None,
            gain: 0.,
            loss: 0.,
            avg: None,
        }
    }
}
impl Indicator for Rsi {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        let prev = self.prev.replace(c.close)?;
        let d = c.close - prev;
        let (g, l) = (d.max(0.), (-d).max(0.));
        let (a, b) = if let Some((a, b)) = self.avg {
            let v = (
                (a * self.p_minus_1_f64 + g) / self.p_f64,
                (b * self.p_minus_1_f64 + l) / self.p_f64,
            );
            self.avg = Some(v);
            v
        } else {
            self.n += 1;
            self.gain += g;
            self.loss += l;
            if self.n < self.p {
                return None;
            }
            let v = (self.gain / self.p_f64, self.loss / self.p_f64);
            self.avg = Some(v);
            v
        };
        Some(if b == 0. {
            if a == 0. {
                50.
            } else {
                100.
            }
        } else if a == 0. {
            0.
        } else {
            100. - 100. / (1. + a / b)
        })
    }
}
