//! Whole-array reference implementations for Tier 1 operators and Kleene three-valued logic.

/// Three-valued Kleene boolean logic (`True`, `False`, `Unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriBool {
    True,
    False,
    Unknown,
}

impl TriBool {
    /// Logical negation under Kleene logic (`not Unknown == Unknown`).
    #[inline]
    pub const fn not(self) -> Self {
        match self {
            TriBool::True => TriBool::False,
            TriBool::False => TriBool::True,
            TriBool::Unknown => TriBool::Unknown,
        }
    }

    /// Logical conjunction under Kleene logic (`False and Unknown == False`).
    #[inline]
    pub const fn and(self, other: Self) -> Self {
        match (self, other) {
            (TriBool::False, _) | (_, TriBool::False) => TriBool::False,
            (TriBool::True, TriBool::True) => TriBool::True,
            _ => TriBool::Unknown,
        }
    }

    /// Logical disjunction under Kleene logic (`True or Unknown == True`).
    #[inline]
    pub const fn or(self, other: Self) -> Self {
        match (self, other) {
            (TriBool::True, _) | (_, TriBool::True) => TriBool::True,
            (TriBool::False, TriBool::False) => TriBool::False,
            _ => TriBool::Unknown,
        }
    }

    /// Converts a program output value to a signal integer (`Unknown` counts as `0`).
    #[inline]
    pub const fn to_signal(self) -> u8 {
        match self {
            TriBool::True => 1,
            TriBool::False | TriBool::Unknown => 0,
        }
    }
}

/// Normalizes non-finite values (`inf`, `-inf`, `NaN`) to `f64::NAN`.
#[inline]
pub fn clean(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        f64::NAN
    }
}

/// Division that returns `NaN` on zero divisor or non-finite inputs, never infinity.
#[inline]
pub fn safe_div(a: f64, b: f64) -> f64 {
    if a.is_finite() && b.is_finite() && b != 0.0 {
        clean(a / b)
    } else {
        f64::NAN
    }
}

pub fn op_abs(x: &[f64]) -> Vec<f64> {
    x.iter()
        .map(|&v| if v.is_finite() { v.abs() } else { f64::NAN })
        .collect()
}

pub fn op_sqrt(x: &[f64]) -> Vec<f64> {
    x.iter()
        .map(|&v| {
            if v.is_finite() && v >= 0.0 {
                v.sqrt()
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_log(x: &[f64]) -> Vec<f64> {
    x.iter()
        .map(|&v| {
            if v.is_finite() && v > 0.0 {
                v.ln()
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_exp(x: &[f64]) -> Vec<f64> {
    x.iter()
        .map(|&v| {
            if v.is_finite() {
                clean(v.exp())
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_min(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            if x.is_finite() && y.is_finite() {
                x.min(y)
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_max(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            if x.is_finite() && y.is_finite() {
                x.max(y)
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_where(cond: &[TriBool], a: &[f64], b: &[f64]) -> Vec<f64> {
    cond.iter()
        .zip(a.iter().zip(b.iter()))
        .map(|(&c, (&x, &y))| match c {
            TriBool::True => clean(x),
            TriBool::False => clean(y),
            TriBool::Unknown => f64::NAN,
        })
        .collect()
}

pub fn op_clamp(x: &[f64], lo: &[f64], hi: &[f64]) -> Vec<f64> {
    x.iter()
        .zip(lo.iter().zip(hi.iter()))
        .map(|(&v, (&l, &h))| {
            if v.is_finite() && l.is_finite() && h.is_finite() && l <= h {
                v.clamp(l, h)
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_mod(a: &[f64], b: &[f64]) -> Vec<f64> {
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| {
            if x.is_finite() && y.is_finite() && y != 0.0 {
                // Match Python/pandas floored modulo (`x % y`).
                clean(x - y * (x / y).floor())
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_diff(x: &[f64], n: usize) -> Vec<f64> {
    (0..x.len())
        .map(|t| {
            if t < n {
                f64::NAN
            } else {
                let cur = x[t];
                let prev = x[t - n];
                if cur.is_finite() && prev.is_finite() {
                    clean(cur - prev)
                } else {
                    f64::NAN
                }
            }
        })
        .collect()
}

pub fn op_pct_change(x: &[f64], n: usize) -> Vec<f64> {
    (0..x.len())
        .map(|t| {
            if t < n {
                f64::NAN
            } else {
                safe_div(x[t] - x[t - n], x[t - n])
            }
        })
        .collect()
}

pub fn op_cumsum(x: &[f64]) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.len());
    let mut acc = 0.0_f64;
    for &v in x {
        if v.is_finite() {
            acc += v;
            out.push(clean(acc));
        } else {
            out.push(f64::NAN);
        }
    }
    out
}

fn rolling_window<F>(x: &[f64], n: usize, mut f: F) -> Vec<f64>
where
    F: FnMut(&[f64]) -> f64,
{
    if n == 0 {
        return vec![f64::NAN; x.len()];
    }
    (0..x.len())
        .map(|t| {
            if t + 1 < n {
                f64::NAN
            } else {
                let w = &x[t + 1 - n..=t];
                if w.iter().all(|v| v.is_finite()) {
                    clean(f(w))
                } else {
                    f64::NAN
                }
            }
        })
        .collect()
}

pub fn op_sum(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| w.iter().sum())
}

pub fn op_mean(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| w.iter().sum::<f64>() / (w.len() as f64))
}

pub fn op_sma(x: &[f64], n: usize) -> Vec<f64> {
    op_mean(x, n)
}

pub fn op_var(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| {
        if w.len() < 2 {
            return f64::NAN;
        }
        let len = w.len() as f64;
        let mean = w.iter().sum::<f64>() / len;
        let ss = w.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
        ss / ((w.len() - 1) as f64)
    })
}

pub fn op_std(x: &[f64], n: usize) -> Vec<f64> {
    op_var(x, n).into_iter().map(|v| v.sqrt()).collect()
}

pub fn ewm_mean(x: &[f64], alpha: f64, min_periods: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(x.len());
    let mut state: Option<f64> = None;
    let mut valid = 0usize;
    for &v in x {
        if v.is_finite() {
            valid += 1;
            let next = match state {
                Some(prev) => alpha * v + (1.0 - alpha) * prev,
                None => v,
            };
            state = Some(clean(next));
        }
        if valid >= min_periods {
            out.push(state.unwrap_or(f64::NAN));
        } else {
            out.push(f64::NAN);
        }
    }
    out
}

pub fn op_ema(x: &[f64], n: usize) -> Vec<f64> {
    if n == 0 {
        return vec![f64::NAN; x.len()];
    }
    let alpha = 2.0 / ((n as f64) + 1.0);
    ewm_mean(x, alpha, n)
}

pub fn op_rma(x: &[f64], n: usize) -> Vec<f64> {
    if n == 0 {
        return vec![f64::NAN; x.len()];
    }
    let alpha = 1.0 / (n as f64);
    ewm_mean(x, alpha, n)
}

pub fn op_wma(x: &[f64], n: usize) -> Vec<f64> {
    if n == 0 {
        return vec![f64::NAN; x.len()];
    }
    let nf = n as f64;
    let denom = nf * (nf + 1.0) / 2.0;
    rolling_window(x, n, |w| {
        w.iter()
            .enumerate()
            .map(|(i, &v)| ((i + 1) as f64) * v)
            .sum::<f64>()
            / denom
    })
}

pub fn op_highest(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| {
        w.iter().copied().fold(f64::NEG_INFINITY, f64::max)
    })
}

pub fn op_lowest(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| w.iter().copied().fold(f64::INFINITY, f64::min))
}

pub fn op_rsi(x: &[f64], n: usize) -> Vec<f64> {
    if n == 0 {
        return vec![f64::NAN; x.len()];
    }
    let delta = op_diff(x, 1);
    let gains: Vec<f64> = delta
        .iter()
        .map(|&d| if d.is_finite() { d.max(0.0) } else { f64::NAN })
        .collect();
    let losses: Vec<f64> = delta
        .iter()
        .map(|&d| {
            if d.is_finite() {
                (-d).max(0.0)
            } else {
                f64::NAN
            }
        })
        .collect();
    let avg_gain = op_rma(&gains, n);
    let avg_loss = op_rma(&losses, n);

    avg_gain
        .into_iter()
        .zip(avg_loss)
        .map(|(ag, al)| {
            if !ag.is_finite() || !al.is_finite() {
                f64::NAN
            } else if al == 0.0 {
                if ag == 0.0 {
                    50.0
                } else {
                    100.0
                }
            } else {
                100.0 - 100.0 / (1.0 + ag / al)
            }
        })
        .collect()
}

pub fn op_tr(high: &[f64], low: &[f64], close: &[f64]) -> Vec<f64> {
    let len = high.len().min(low.len()).min(close.len());
    (0..len)
        .map(|t| {
            if t == 0 {
                return f64::NAN;
            }
            let h = high[t];
            let l = low[t];
            let pc = close[t - 1];
            if h.is_finite() && l.is_finite() && pc.is_finite() {
                clean((h - l).max((h - pc).abs()).max((l - pc).abs()))
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_atr(high: &[f64], low: &[f64], close: &[f64], n: usize) -> Vec<f64> {
    let tr = op_tr(high, low, close);
    op_rma(&tr, n)
}

pub fn op_macd(x: &[f64], fast: usize, slow: usize, signal: usize) -> Vec<f64> {
    let ef = op_ema(x, fast);
    let es = op_ema(x, slow);
    let macd_line: Vec<f64> = ef
        .into_iter()
        .zip(es)
        .map(|(a, b)| {
            if a.is_finite() && b.is_finite() {
                clean(a - b)
            } else {
                f64::NAN
            }
        })
        .collect();
    let sig_line = op_ema(&macd_line, signal);
    macd_line
        .into_iter()
        .zip(sig_line)
        .map(|(m, s)| {
            if m.is_finite() && s.is_finite() {
                clean(m - s)
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_bollinger(x: &[f64], n: usize, mult: f64) -> Vec<f64> {
    let mid = op_sma(x, n);
    let sd = op_std(x, n);
    mid.into_iter()
        .zip(sd)
        .map(|(m, s)| {
            if m.is_finite() && s.is_finite() && mult.is_finite() {
                clean(m + mult * s)
            } else {
                f64::NAN
            }
        })
        .collect()
}

pub fn op_cov(x: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let len = x.len().min(y.len());
    if n < 2 {
        return vec![f64::NAN; len];
    }
    (0..len)
        .map(|t| {
            if t + 1 < n {
                return f64::NAN;
            }
            let wx = &x[t + 1 - n..=t];
            let wy = &y[t + 1 - n..=t];
            if !wx.iter().all(|v| v.is_finite()) || !wy.iter().all(|v| v.is_finite()) {
                return f64::NAN;
            }
            let nf = n as f64;
            let mx = wx.iter().sum::<f64>() / nf;
            let my = wy.iter().sum::<f64>() / nf;
            let sum_prod = wx
                .iter()
                .zip(wy.iter())
                .map(|(&a, &b)| (a - mx) * (b - my))
                .sum::<f64>();
            clean(sum_prod / ((n - 1) as f64))
        })
        .collect()
}

pub fn op_corr(x: &[f64], y: &[f64], n: usize) -> Vec<f64> {
    let len = x.len().min(y.len());
    if n < 2 {
        return vec![f64::NAN; len];
    }
    (0..len)
        .map(|t| {
            if t + 1 < n {
                return f64::NAN;
            }
            let wx = &x[t + 1 - n..=t];
            let wy = &y[t + 1 - n..=t];
            if !wx.iter().all(|v| v.is_finite()) || !wy.iter().all(|v| v.is_finite()) {
                return f64::NAN;
            }
            let nf = n as f64;
            let mx = wx.iter().sum::<f64>() / nf;
            let my = wy.iter().sum::<f64>() / nf;
            let mut sxx = 0.0_f64;
            let mut syy = 0.0_f64;
            let mut sxy = 0.0_f64;
            for (&a, &b) in wx.iter().zip(wy.iter()) {
                let dx = a - mx;
                let dy = b - my;
                sxx += dx * dx;
                syy += dy * dy;
                sxy += dx * dy;
            }
            let denom = (sxx * syy).sqrt();
            if denom == 0.0 || !denom.is_finite() {
                f64::NAN
            } else {
                clean(sxy / denom)
            }
        })
        .collect()
}

fn linreg_pair(w: &[f64]) -> (f64, f64) {
    let n = w.len();
    if n < 2 {
        return (f64::NAN, f64::NAN);
    }
    let nf = n as f64;
    let mean_i = (nf - 1.0) / 2.0;
    let mean_y = w.iter().sum::<f64>() / nf;
    let mut num = 0.0_f64;
    let mut den = 0.0_f64;
    for (i, &y) in w.iter().enumerate() {
        let di = (i as f64) - mean_i;
        num += di * (y - mean_y);
        den += di * di;
    }
    if den == 0.0 {
        return (f64::NAN, f64::NAN);
    }
    let slope = num / den;
    let intercept = mean_y - slope * mean_i;
    (clean(slope), clean(intercept))
}

pub fn op_linreg_slope(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| linreg_pair(w).0)
}

pub fn op_linreg_intercept(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| linreg_pair(w).1)
}

pub fn op_percentile_rank(x: &[f64], n: usize) -> Vec<f64> {
    rolling_window(x, n, |w| {
        let cur = w[w.len() - 1];
        let count = w.iter().filter(|&&v| v <= cur).count();
        (count as f64) / (w.len() as f64) * 100.0
    })
}

pub fn op_crossover(a: &[f64], b: &[f64]) -> Vec<TriBool> {
    let len = a.len().min(b.len());
    (0..len)
        .map(|t| {
            if t == 0 {
                return TriBool::Unknown;
            }
            let (a0, b0, a1, b1) = (a[t - 1], b[t - 1], a[t], b[t]);
            if !a0.is_finite() || !b0.is_finite() || !a1.is_finite() || !b1.is_finite() {
                TriBool::Unknown
            } else if a0 <= b0 && a1 > b1 {
                TriBool::True
            } else {
                TriBool::False
            }
        })
        .collect()
}

pub fn op_crossunder(a: &[f64], b: &[f64]) -> Vec<TriBool> {
    let len = a.len().min(b.len());
    (0..len)
        .map(|t| {
            if t == 0 {
                return TriBool::Unknown;
            }
            let (a0, b0, a1, b1) = (a[t - 1], b[t - 1], a[t], b[t]);
            if !a0.is_finite() || !b0.is_finite() || !a1.is_finite() || !b1.is_finite() {
                TriBool::Unknown
            } else if a0 >= b0 && a1 < b1 {
                TriBool::True
            } else {
                TriBool::False
            }
        })
        .collect()
}

pub fn op_since(cond: &[TriBool]) -> Vec<f64> {
    let mut out = Vec::with_capacity(cond.len());
    let mut last_true: Option<u64> = None;
    for &c in cond {
        match c {
            TriBool::True => {
                last_true = Some(0);
                out.push(0.0);
            }
            TriBool::False => {
                if let Some(cnt) = last_true {
                    let next = cnt + 1;
                    last_true = Some(next);
                    out.push(next as f64);
                } else {
                    out.push(f64::NAN);
                }
            }
            TriBool::Unknown => {
                out.push(f64::NAN);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kleene_truth_tables() {
        let vals = [TriBool::True, TriBool::False, TriBool::Unknown];
        assert_eq!(TriBool::True.not(), TriBool::False);
        assert_eq!(TriBool::False.not(), TriBool::True);
        assert_eq!(TriBool::Unknown.not(), TriBool::Unknown);

        assert_eq!(TriBool::False.and(TriBool::Unknown), TriBool::False);
        assert_eq!(TriBool::Unknown.and(TriBool::False), TriBool::False);
        assert_eq!(TriBool::True.and(TriBool::Unknown), TriBool::Unknown);

        assert_eq!(TriBool::True.or(TriBool::Unknown), TriBool::True);
        assert_eq!(TriBool::Unknown.or(TriBool::True), TriBool::True);
        assert_eq!(TriBool::False.or(TriBool::Unknown), TriBool::Unknown);

        for a in vals {
            assert_eq!(a.and(TriBool::True), a);
            assert_eq!(a.or(TriBool::False), a);
        }
    }

    #[test]
    fn std_two_pass_adversarial_accuracy() {
        let data: Vec<f64> = (0..50).map(|i| 1e9 + ((i % 5) as f64) * 0.25).collect();
        let out = op_std(&data, 20);
        // Expected sample std of [0, 0.25, 0.5, 0.75, 1.0] repeated 4 times:
        let w: Vec<f64> = (0..20).map(|i| ((i % 5) as f64) * 0.25).collect();
        let mean = w.iter().sum::<f64>() / 20.0;
        let expected = (w.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 19.0).sqrt();
        assert!((out[19] - expected).abs() / expected < 1e-12);
    }

    #[test]
    fn clamp_reversed_bounds_never_panics() {
        let out = op_clamp(&[5.0], &[10.0], &[1.0]);
        assert!(out[0].is_nan());
    }

    #[test]
    fn wma_large_or_zero_window_never_panics() {
        let out_max = op_wma(&[1.0, 2.0, 3.0], usize::MAX);
        assert_eq!(out_max.len(), 3);
        assert!(out_max.iter().all(|v| v.is_nan()));

        let out_zero = op_wma(&[1.0, 2.0], 0);
        assert_eq!(out_zero.len(), 2);
        assert!(out_zero.iter().all(|v| v.is_nan()));
    }
}
