//! Pre-allocated, zero-step-allocation sliding-window and lag buffers.
#![allow(dead_code)]

use mln_std::eval::clean;

/// Monoid summary for a two-stack sliding window.
pub(crate) trait WindowMonoid: Copy {
    type Item: Copy;
    fn single(item: Self::Item) -> Self;
    fn merge(left: Self, right: Self) -> Self;
}

/// Fixed-capacity two-stack sliding window queue (`O(1)` amortized per bar, zero allocations after `new`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TwoStackWindow<M: WindowMonoid> {
    cap: usize,
    front: Vec<(M::Item, M)>,
    back: Vec<(M::Item, M)>,
}

impl<M: WindowMonoid> TwoStackWindow<M> {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            front: Vec::with_capacity(cap),
            back: Vec::with_capacity(cap),
        }
    }

    pub fn len(&self) -> usize {
        self.front.len() + self.back.len()
    }

    pub fn is_full(&self) -> bool {
        self.cap > 0 && self.len() == self.cap
    }

    pub fn push(&mut self, item: M::Item) -> Option<M> {
        if self.cap == 0 {
            return None;
        }
        if self.len() == self.cap {
            self.pop_front();
        }
        let s = M::single(item);
        let merged = match self.back.last() {
            Some(&(_, prev)) => M::merge(prev, s),
            None => s,
        };
        self.back.push((item, merged));
        if self.is_full() {
            self.summary()
        } else {
            None
        }
    }

    fn pop_front(&mut self) {
        if self.front.is_empty() {
            while let Some((item, _)) = self.back.pop() {
                let s = M::single(item);
                let merged = match self.front.last() {
                    Some(&(_, prev)) => M::merge(s, prev),
                    None => s,
                };
                self.front.push((item, merged));
            }
        }
        self.front.pop();
    }

    pub fn summary(&self) -> Option<M> {
        match (self.front.last(), self.back.last()) {
            (Some(&(_, lf)), Some(&(_, rb))) => Some(M::merge(lf, rb)),
            (Some(&(_, lf)), None) => Some(lf),
            (None, Some(&(_, rb))) => Some(rb),
            (None, None) => None,
        }
    }
}

/// Sum summary (`sum`, `mean`, `sma`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SumSummary {
    Invalid,
    Valid { count: usize, sum: f64 },
}

impl WindowMonoid for SumSummary {
    type Item = f64;

    #[inline]
    fn single(item: f64) -> Self {
        if item.is_finite() {
            SumSummary::Valid {
                count: 1,
                sum: item,
            }
        } else {
            SumSummary::Invalid
        }
    }

    #[inline]
    fn merge(left: Self, right: Self) -> Self {
        match (left, right) {
            (
                SumSummary::Valid { count: ca, sum: sa },
                SumSummary::Valid { count: cb, sum: sb },
            ) => SumSummary::Valid {
                count: ca + cb,
                sum: sa + sb,
            },
            _ => SumSummary::Invalid,
        }
    }
}

/// Chan's parallel Welford summary (`var`, `std`, `bollinger`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum StatsSummary {
    Invalid,
    Valid { count: usize, mean: f64, m2: f64 },
}

impl WindowMonoid for StatsSummary {
    type Item = f64;

    #[inline]
    fn single(item: f64) -> Self {
        if item.is_finite() {
            StatsSummary::Valid {
                count: 1,
                mean: item,
                m2: 0.0,
            }
        } else {
            StatsSummary::Invalid
        }
    }

    #[inline]
    fn merge(left: Self, right: Self) -> Self {
        match (left, right) {
            (
                StatsSummary::Valid {
                    count: na,
                    mean: ma,
                    m2: m2a,
                },
                StatsSummary::Valid {
                    count: nb,
                    mean: mb,
                    m2: m2b,
                },
            ) => {
                let n = na + nb;
                let nf = n as f64;
                let delta = mb - ma;
                let mean = ma + delta * (nb as f64) / nf;
                let m2 = m2a + m2b + delta * delta * ((na * nb) as f64) / nf;
                StatsSummary::Valid { count: n, mean, m2 }
            }
            _ => StatsSummary::Invalid,
        }
    }
}

/// Chan's parallel covariance/correlation summary (`cov`, `corr`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PairStatsSummary {
    Invalid,
    Valid {
        count: usize,
        mx: f64,
        my: f64,
        m2x: f64,
        m2y: f64,
        cxy: f64,
    },
}

impl WindowMonoid for PairStatsSummary {
    type Item = (f64, f64);

    #[inline]
    fn single((x, y): (f64, f64)) -> Self {
        if x.is_finite() && y.is_finite() {
            PairStatsSummary::Valid {
                count: 1,
                mx: x,
                my: y,
                m2x: 0.0,
                m2y: 0.0,
                cxy: 0.0,
            }
        } else {
            PairStatsSummary::Invalid
        }
    }

    #[inline]
    fn merge(left: Self, right: Self) -> Self {
        match (left, right) {
            (
                PairStatsSummary::Valid {
                    count: na,
                    mx: mxa,
                    my: mya,
                    m2x: m2xa,
                    m2y: m2ya,
                    cxy: cxya,
                },
                PairStatsSummary::Valid {
                    count: nb,
                    mx: mxb,
                    my: myb,
                    m2x: m2xb,
                    m2y: m2yb,
                    cxy: cxyb,
                },
            ) => {
                let n = na + nb;
                let nf = n as f64;
                let dx = mxb - mxa;
                let dy = myb - mya;
                let weight = ((na * nb) as f64) / nf;
                PairStatsSummary::Valid {
                    count: n,
                    mx: mxa + dx * (nb as f64) / nf,
                    my: mya + dy * (nb as f64) / nf,
                    m2x: m2xa + m2xb + dx * dx * weight,
                    m2y: m2ya + m2yb + dy * dy * weight,
                    cxy: cxya + cxyb + dx * dy * weight,
                }
            }
            _ => PairStatsSummary::Invalid,
        }
    }
}

/// Chan's parallel OLS linear-regression summary over window indices `0..n-1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum LinregSummary {
    Invalid,
    Valid {
        count: usize,
        mean_y: f64,
        cov_iy: f64,
    },
}

impl WindowMonoid for LinregSummary {
    type Item = f64;

    #[inline]
    fn single(item: f64) -> Self {
        if item.is_finite() {
            LinregSummary::Valid {
                count: 1,
                mean_y: item,
                cov_iy: 0.0,
            }
        } else {
            LinregSummary::Invalid
        }
    }

    #[inline]
    fn merge(left: Self, right: Self) -> Self {
        match (left, right) {
            (
                LinregSummary::Valid {
                    count: na,
                    mean_y: ma,
                    cov_iy: ca,
                },
                LinregSummary::Valid {
                    count: nb,
                    mean_y: mb,
                    cov_iy: cb,
                },
            ) => {
                let n = na + nb;
                let nf = n as f64;
                let dx = nf * 0.5;
                let dy = mb - ma;
                LinregSummary::Valid {
                    count: n,
                    mean_y: ma + dy * (nb as f64) / nf,
                    cov_iy: ca + cb + dx * dy * ((na * nb) as f64) / nf,
                }
            }
            _ => LinregSummary::Invalid,
        }
    }
}

impl LinregSummary {
    #[inline]
    pub fn slope_and_intercept(self) -> (f64, f64) {
        match self {
            LinregSummary::Valid {
                count,
                mean_y,
                cov_iy,
            } if count >= 2 => {
                let nf = count as f64;
                let den = nf * (nf * nf - 1.0) / 12.0;
                let slope = clean(cov_iy / den);
                let intercept = clean(mean_y - slope * ((nf - 1.0) * 0.5));
                (slope, intercept)
            }
            _ => (f64::NAN, f64::NAN),
        }
    }
}

/// Linearly-weighted moving average summary (`wma`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum WmaSummary {
    Invalid,
    Valid { count: usize, sum: f64, wsum: f64 },
}

impl WindowMonoid for WmaSummary {
    type Item = f64;

    #[inline]
    fn single(item: f64) -> Self {
        if item.is_finite() {
            WmaSummary::Valid {
                count: 1,
                sum: item,
                wsum: item,
            }
        } else {
            WmaSummary::Invalid
        }
    }

    #[inline]
    fn merge(left: Self, right: Self) -> Self {
        match (left, right) {
            (
                WmaSummary::Valid {
                    count: ca,
                    sum: sa,
                    wsum: wa,
                },
                WmaSummary::Valid {
                    count: cb,
                    sum: sb,
                    wsum: wb,
                },
            ) => WmaSummary::Valid {
                count: ca + cb,
                sum: sa + sb,
                wsum: wa + wb + (ca as f64) * sb,
            },
            _ => WmaSummary::Invalid,
        }
    }
}

/// Sliding-window extrema summary (`highest`, `lowest`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ExtremaSummary {
    Invalid,
    Valid { min: f64, max: f64 },
}

impl WindowMonoid for ExtremaSummary {
    type Item = f64;

    #[inline]
    fn single(item: f64) -> Self {
        if item.is_finite() {
            ExtremaSummary::Valid {
                min: item,
                max: item,
            }
        } else {
            ExtremaSummary::Invalid
        }
    }

    #[inline]
    fn merge(left: Self, right: Self) -> Self {
        match (left, right) {
            (
                ExtremaSummary::Valid {
                    min: mina,
                    max: maxa,
                },
                ExtremaSummary::Valid {
                    min: minb,
                    max: maxb,
                },
            ) => ExtremaSummary::Valid {
                min: mina.min(minb),
                max: maxa.max(maxb),
            },
            _ => ExtremaSummary::Invalid,
        }
    }
}

/// Fixed-capacity ring buffer for `percentile_rank` (linear per bar, zero allocation after `new`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PercentileWindow {
    ring: Vec<f64>,
    idx: usize,
    filled: usize,
    nan_count: usize,
}

impl PercentileWindow {
    pub fn new(n: usize) -> Self {
        Self {
            ring: vec![f64::NAN; n],
            idx: 0,
            filled: 0,
            nan_count: 0,
        }
    }

    pub fn push(&mut self, x: f64) -> f64 {
        let n = self.ring.len();
        if n == 0 {
            return f64::NAN;
        }
        if self.filled == n {
            if !self.ring[self.idx].is_finite() {
                self.nan_count -= 1;
            }
        } else {
            self.filled += 1;
        }
        self.ring[self.idx] = x;
        if !x.is_finite() {
            self.nan_count += 1;
        }
        self.idx = (self.idx + 1) % n;

        if self.filled < n || self.nan_count > 0 {
            return f64::NAN;
        }
        let count = self.ring.iter().filter(|&&v| v <= x).count();
        (count as f64) / (n as f64) * 100.0
    }
}

/// Fixed-capacity lag ring buffer (`x[k]`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LagRing<T: Copy> {
    buf: Vec<T>,
    idx: usize,
}

impl<T: Copy> LagRing<T> {
    pub fn new(lag: usize, init: T) -> Self {
        Self {
            buf: vec![init; lag.saturating_add(1)],
            idx: 0,
        }
    }

    pub fn push(&mut self, val: T) -> T {
        let len = self.buf.len();
        self.buf[self.idx] = val;
        let read_idx = (self.idx + 1) % len;
        let out = self.buf[read_idx];
        self.idx = read_idx;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_stack_stats_matches_two_pass_on_1e9_with_nans() {
        let mut win: TwoStackWindow<StatsSummary> = TwoStackWindow::new(20);
        let data: Vec<f64> = (0..80)
            .map(|i| {
                if i == 25 || i == 26 {
                    f64::NAN
                } else {
                    1e9 + (((i * 17) % 9) as f64 - 4.0) * 0.125
                }
            })
            .collect();

        for (t, &x) in data.iter().enumerate() {
            let s = win.push(x);
            if t < 19 || data[t - 19..=t].iter().any(|v| !v.is_finite()) {
                assert!(matches!(s, None | Some(StatsSummary::Invalid)), "bar {t}");
            } else {
                let Some(StatsSummary::Valid { count, m2, .. }) = s else {
                    panic!("expected valid stats at {t}");
                };
                let got_std = (m2 / ((count - 1) as f64)).sqrt();
                let w = &data[t - 19..=t];
                let mean = w.iter().sum::<f64>() / 20.0;
                let exp_std = (w.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / 19.0).sqrt();
                assert!(
                    (got_std - exp_std).abs() <= 1e-7 * exp_std.max(1.0),
                    "bar {t}: got={got_std} exp={exp_std}"
                );
            }
        }
    }

    #[test]
    fn two_stack_linreg_matches_ols_on_1e9() {
        let mut win: TwoStackWindow<LinregSummary> = TwoStackWindow::new(15);
        let data: Vec<f64> = (0..50)
            .map(|i| 1e9 + (i as f64) * 0.5 + (((i * 7) % 5) as f64) * 0.1)
            .collect();

        for (t, &x) in data.iter().enumerate() {
            if let Some(summary) = win.push(x) {
                let (slope, intercept) = summary.slope_and_intercept();
                let w = &data[t - 14..=t];
                let mean_i = 7.0;
                let mean_y = w.iter().sum::<f64>() / 15.0;
                let num: f64 = w
                    .iter()
                    .enumerate()
                    .map(|(i, &y)| ((i as f64) - mean_i) * (y - mean_y))
                    .sum();
                let den: f64 = (0..15).map(|i| ((i as f64) - mean_i).powi(2)).sum();
                let exp_slope = num / den;
                let exp_int = mean_y - exp_slope * mean_i;
                assert!(
                    (slope - exp_slope).abs() <= 1e-7 * exp_slope.abs().max(1.0),
                    "slope={slope} exp={exp_slope}"
                );
                assert!(
                    (intercept - exp_int).abs() <= 1e-6 * exp_int.abs().max(1.0),
                    "intercept={intercept} exp={exp_int}"
                );
            }
        }
    }

    #[test]
    fn two_stack_wma_exact_order() {
        let mut win: TwoStackWindow<WmaSummary> = TwoStackWindow::new(4);
        for v in [10.0, 20.0, 30.0] {
            assert!(win.push(v).is_none());
        }
        let s = win.push(40.0).unwrap();
        let WmaSummary::Valid { count, wsum, .. } = s else {
            panic!();
        };
        assert_eq!(count, 4);
        assert_eq!(wsum, 300.0);
        // Push 50.0 -> window [20, 30, 40, 50]: 1*20 + 2*30 + 3*40 + 4*50 = 400
        let s2 = win.push(50.0).unwrap();
        let WmaSummary::Valid { wsum: wsum2, .. } = s2 else {
            panic!();
        };
        assert_eq!(wsum2, 400.0);
    }
}
