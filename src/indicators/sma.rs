use super::{candle::Candle, state::Indicator};
pub(super) struct Sma {
    p: usize,
    buf: Vec<f64>,
    pos: usize,
    sum: f64,
    n: usize,
    ticks: usize,
}
impl Sma {
    pub fn new(p: usize) -> Self {
        Self {
            p,
            buf: vec![0.; p],
            pos: 0,
            sum: 0.,
            n: 0,
            ticks: 0,
        }
    }
}
impl Indicator for Sma {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        self.ticks += 1;
        if self.n == self.p {
            self.sum -= self.buf[self.pos]
        } else {
            self.n += 1
        }
        self.buf[self.pos] = c.close;
        self.sum += c.close;
        self.pos = (self.pos + 1) % self.p;
        if self.ticks % 4096 == 0 {
            self.sum = self.buf.iter().sum()
        }
        (self.n == self.p).then(|| self.sum / self.p as f64)
    }
}
