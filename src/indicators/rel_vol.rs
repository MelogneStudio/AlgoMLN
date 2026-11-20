use super::{candle::Candle, state::Indicator};
pub(super) struct RelVol {
    p: usize,
    values: Vec<f64>,
    count: usize,
    pos: usize,
    sum: f64,
    ticks: usize,
}
impl RelVol {
    pub fn new(p: usize) -> Self {
        Self {
            p,
            values: vec![0.0; p],
            count: 0,
            pos: 0,
            sum: 0.,
            ticks: 0,
        }
    }
}
impl Indicator for RelVol {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        if self.p == 0 {
            return None;
        }
        self.ticks += 1;
        let out = if self.count == self.p {
            let avg = self.sum / self.p as f64;
            (avg > 0.).then(|| c.volume / avg)
        } else {
            None
        };
        if self.count == self.p {
            self.sum -= self.values[self.pos];
        } else {
            self.count += 1;
        }
        self.values[self.pos] = c.volume;
        self.sum += c.volume;
        self.pos = (self.pos + 1) % self.p;
        if (self.ticks & 4095) == 0 {
            self.sum = self.values.iter().take(self.count).sum();
        }
        out
    }
}
