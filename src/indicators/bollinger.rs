use super::{candle::Candle, spec::IndicatorType, state::Indicator};
pub(super) struct Bollinger {
    p: usize,
    kind: IndicatorType,
    m: f64,
    anchor: Option<f64>,
    buf: Vec<f64>,
    pos: usize,
    n: usize,
    sum: f64,
    mean_offset: f64,
    m2: f64,
    ticks: usize,
}
impl Bollinger {
    pub fn new(p: usize, kind: IndicatorType, m: f64) -> Self {
        Self {
            p,
            kind,
            m,
            anchor: None,
            buf: vec![0.; p],
            pos: 0,
            n: 0,
            sum: 0.,
            mean_offset: 0.,
            m2: 0.,
            ticks: 0,
        }
    }
}
impl Indicator for Bollinger {
    fn update(&mut self, c: &Candle) -> Option<f64> {
        self.ticks += 1;
        let full = self.n == self.p;
        if full {
            let old = self.buf[self.pos] - self.anchor.unwrap_or(0.0);
            self.sum -= old;
            if self.p > 1 {
                let next_mean = (self.p as f64 * self.mean_offset - old) / (self.p - 1) as f64;
                self.m2 -= (old - self.mean_offset) * (old - next_mean);
                self.mean_offset = next_mean;
            } else {
                self.mean_offset = 0.0;
                self.m2 = 0.0;
            }
        } else {
            self.n += 1
        }
        let x = c.close;
        let anchor = *self.anchor.get_or_insert(x);
        let centered = x - anchor;
        self.buf[self.pos] = x;
        self.sum += centered;
        let n = if full { self.p } else { self.n };
        let delta = centered - self.mean_offset;
        self.mean_offset += delta / n as f64;
        self.m2 += delta * (centered - self.mean_offset);
        self.pos = (self.pos + 1) % self.p;
        if self.ticks % 4096 == 0 {
            self.sum = self.buf.iter().take(self.n).map(|x| x - anchor).sum();
            self.mean_offset = 0.0;
            self.m2 = 0.0;
            for (i, value) in self.buf.iter().take(self.n).enumerate() {
                let value = *value - anchor;
                let delta = value - self.mean_offset;
                let n = (i + 1) as f64;
                self.mean_offset += delta / n;
                self.m2 += delta * (value - self.mean_offset);
            }
        }
        if self.n < self.p {
            return None;
        }
        let mean_offset = self.sum / self.p as f64;
        let mid = anchor + mean_offset;
        let sd = (self.m2 / self.p as f64).max(0.).sqrt();
        Some(match self.kind {
            IndicatorType::BbUpper => mid + self.m * sd,
            IndicatorType::BbLower => mid - self.m * sd,
            _ => mid,
        })
    }
}
