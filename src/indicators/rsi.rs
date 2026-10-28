use super::{candle::Candle, state::Indicator};
pub(super) struct Rsi {
    p: usize,
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
                (a * (self.p - 1) as f64 + g) / self.p as f64,
                (b * (self.p - 1) as f64 + l) / self.p as f64,
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
            let v = (self.gain / self.p as f64, self.loss / self.p as f64);
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
