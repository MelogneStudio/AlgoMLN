//! Per-node incremental state structs, bar validation, and snapshots.
#![allow(dead_code)]

use mln_check::Tf;
use mln_std::eval::clean;
use mln_std::TriBool;

use crate::oracle::ExecError;
use crate::rolling::{
    ExtremaSummary, LagRing, LinregSummary, PairStatsSummary, PercentileWindow, StatsSummary,
    SumSummary, TwoStackWindow, WmaSummary,
};

/// Single OHLCV bar input passed to `Executor::step`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub timestamp: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl Bar {
    /// Creates a bar with the given timestamp and OHLCV fields.
    pub const fn new(
        timestamp: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        volume: f64,
    ) -> Self {
        Self {
            timestamp,
            open,
            high,
            low,
            close,
            volume,
        }
    }
}

/// Output produced for a single bar by `Executor::step`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BarOutput {
    pub timestamp: i64,
    pub entry: TriBool,
    pub exit: TriBool,
    pub entry_signal: u8,
    pub exit_signal: u8,
}

/// Runtime value of an IR node at the current bar (`Copy`, zero heap allocation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeVal {
    Num(f64),
    Bool(TriBool),
    Tf(Tf),
    Str(u32),
}

/// Validates a bar strictly: rejects non-finite OHLCV values, `high < low`, and negative volume.
pub(crate) fn validate_bar(bar: &Bar) -> Result<(), ExecError> {
    if !bar.open.is_finite()
        || !bar.high.is_finite()
        || !bar.low.is_finite()
        || !bar.close.is_finite()
        || !bar.volume.is_finite()
    {
        return Err(ExecError::InvalidBar(
            "OHLCV fields must all be finite".to_string(),
        ));
    }
    if bar.high < bar.low {
        return Err(ExecError::InvalidBar(format!(
            "high ({}) is below low ({})",
            bar.high, bar.low
        )));
    }
    if bar.volume < 0.0 {
        return Err(ExecError::InvalidBar(format!(
            "negative volume ({})",
            bar.volume
        )));
    }
    Ok(())
}

/// Validates a bar when NaN holes are permitted (rejects infinities, `high < low`, and negative volume).
pub(crate) fn validate_bar_allowing_nans(bar: &Bar) -> Result<(), ExecError> {
    for v in [bar.open, bar.high, bar.low, bar.close, bar.volume] {
        if v.is_infinite() {
            return Err(ExecError::InvalidBar(
                "infinite OHLCV values are not allowed".to_string(),
            ));
        }
    }
    if bar.high.is_finite() && bar.low.is_finite() && bar.high < bar.low {
        return Err(ExecError::InvalidBar(format!(
            "high ({}) is below low ({})",
            bar.high, bar.low
        )));
    }
    if bar.volume.is_finite() && bar.volume < 0.0 {
        return Err(ExecError::InvalidBar(format!(
            "negative volume ({})",
            bar.volume
        )));
    }
    Ok(())
}

/// Exponentially weighted moving average state (`ema`, `rma`).
#[derive(Debug, Clone, PartialEq)]
pub struct EwmState {
    alpha: f64,
    min_periods: usize,
    state: Option<f64>,
    valid: usize,
}

impl EwmState {
    pub fn new(alpha: f64, min_periods: usize) -> Self {
        Self {
            alpha,
            min_periods,
            state: None,
            valid: 0,
        }
    }

    pub fn step(&mut self, x: f64) -> f64 {
        if x.is_finite() {
            self.valid += 1;
            let next = match self.state {
                Some(prev) => self.alpha * x + (1.0 - self.alpha) * prev,
                None => x,
            };
            self.state = Some(clean(next));
        }
        if self.valid >= self.min_periods {
            self.state.unwrap_or(f64::NAN)
        } else {
            f64::NAN
        }
    }
}

/// Relative Strength Index state (`rsi`).
#[derive(Debug, Clone, PartialEq)]
pub struct RsiState {
    prev_x: f64,
    avg_gain: EwmState,
    avg_loss: EwmState,
}

impl RsiState {
    pub fn new(n: usize) -> Self {
        let alpha = 1.0 / (n as f64);
        Self {
            prev_x: f64::NAN,
            avg_gain: EwmState::new(alpha, n),
            avg_loss: EwmState::new(alpha, n),
        }
    }

    pub fn step(&mut self, x: f64) -> f64 {
        let delta = if x.is_finite() && self.prev_x.is_finite() {
            clean(x - self.prev_x)
        } else {
            f64::NAN
        };
        self.prev_x = x;
        let gain = if delta.is_finite() {
            delta.max(0.0)
        } else {
            f64::NAN
        };
        let loss = if delta.is_finite() {
            (-delta).max(0.0)
        } else {
            f64::NAN
        };
        let ag = self.avg_gain.step(gain);
        let al = self.avg_loss.step(loss);
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
    }
}

/// True Range state (`tr`).
#[derive(Debug, Clone, PartialEq)]
pub struct TrState {
    prev_close: f64,
}

impl Default for TrState {
    fn default() -> Self {
        Self::new()
    }
}

impl TrState {
    pub fn new() -> Self {
        Self {
            prev_close: f64::NAN,
        }
    }

    pub fn step(&mut self, high: f64, low: f64, close: f64) -> f64 {
        let out = if high.is_finite() && low.is_finite() && self.prev_close.is_finite() {
            clean(
                (high - low)
                    .max((high - self.prev_close).abs())
                    .max((low - self.prev_close).abs()),
            )
        } else {
            f64::NAN
        };
        self.prev_close = close;
        out
    }
}

/// Average True Range state (`atr`).
#[derive(Debug, Clone, PartialEq)]
pub struct AtrState {
    tr: TrState,
    rma: EwmState,
}

impl AtrState {
    pub fn new(n: usize) -> Self {
        Self {
            tr: TrState::new(),
            rma: EwmState::new(1.0 / (n as f64), n),
        }
    }

    pub fn step(&mut self, high: f64, low: f64, close: f64) -> f64 {
        let tr = self.tr.step(high, low, close);
        self.rma.step(tr)
    }
}

/// MACD histogram state (`macd`).
#[derive(Debug, Clone, PartialEq)]
pub struct MacdState {
    ema_fast: EwmState,
    ema_slow: EwmState,
    ema_sig: EwmState,
}

impl MacdState {
    pub fn new(fast: usize, slow: usize, signal: usize) -> Self {
        Self {
            ema_fast: EwmState::new(2.0 / ((fast as f64) + 1.0), fast),
            ema_slow: EwmState::new(2.0 / ((slow as f64) + 1.0), slow),
            ema_sig: EwmState::new(2.0 / ((signal as f64) + 1.0), signal),
        }
    }

    pub fn step(&mut self, x: f64) -> f64 {
        let ef = self.ema_fast.step(x);
        let es = self.ema_slow.step(x);
        let macd_line = if ef.is_finite() && es.is_finite() {
            clean(ef - es)
        } else {
            f64::NAN
        };
        let sig_line = self.ema_sig.step(macd_line);
        if macd_line.is_finite() && sig_line.is_finite() {
            clean(macd_line - sig_line)
        } else {
            f64::NAN
        }
    }
}

/// Cumulative sum state (`cumsum`).
#[derive(Debug, Clone, PartialEq)]
pub struct CumsumState {
    acc: f64,
}

impl Default for CumsumState {
    fn default() -> Self {
        Self::new()
    }
}

impl CumsumState {
    pub fn new() -> Self {
        Self { acc: 0.0 }
    }

    pub fn step(&mut self, x: f64) -> f64 {
        if x.is_finite() {
            self.acc += x;
            clean(self.acc)
        } else {
            f64::NAN
        }
    }
}

/// Crossover / crossunder state.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossState {
    prev_a: f64,
    prev_b: f64,
    up: bool,
}

impl CrossState {
    pub fn new(up: bool) -> Self {
        Self {
            prev_a: f64::NAN,
            prev_b: f64::NAN,
            up,
        }
    }

    pub fn step(&mut self, a: f64, b: f64) -> TriBool {
        let out = if !self.prev_a.is_finite()
            || !self.prev_b.is_finite()
            || !a.is_finite()
            || !b.is_finite()
        {
            TriBool::Unknown
        } else if self.up {
            if self.prev_a <= self.prev_b && a > b {
                TriBool::True
            } else {
                TriBool::False
            }
        } else if self.prev_a >= self.prev_b && a < b {
            TriBool::True
        } else {
            TriBool::False
        };
        self.prev_a = a;
        self.prev_b = b;
        out
    }
}

/// Bars-since-true state (`since`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinceState {
    last_true: Option<u64>,
}

impl Default for SinceState {
    fn default() -> Self {
        Self::new()
    }
}

impl SinceState {
    pub fn new() -> Self {
        Self { last_true: None }
    }

    pub fn step(&mut self, cond: TriBool) -> f64 {
        match cond {
            TriBool::True => {
                self.last_true = Some(0);
                0.0
            }
            TriBool::False => {
                if let Some(cnt) = self.last_true {
                    let next = cnt.saturating_add(1);
                    self.last_true = Some(next);
                    next as f64
                } else {
                    f64::NAN
                }
            }
            TriBool::Unknown => {
                if let Some(cnt) = self.last_true {
                    self.last_true = Some(cnt.saturating_add(1));
                }
                f64::NAN
            }
        }
    }
}

/// Per-node runtime state stored inside an `Executor`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum NodeState {
    Stateless,
    Lag(LagRing<NodeVal>),
    Diff(LagRing<f64>),
    PctChange(LagRing<f64>),
    Cumsum(CumsumState),
    Sum(TwoStackWindow<SumSummary>),
    Stats(TwoStackWindow<StatsSummary>),
    Ewm(EwmState),
    Wma(TwoStackWindow<WmaSummary>),
    Extrema(TwoStackWindow<ExtremaSummary>),
    Rsi(RsiState),
    Tr(TrState),
    Atr(AtrState),
    Macd(MacdState),
    PairStats(TwoStackWindow<PairStatsSummary>),
    Linreg(TwoStackWindow<LinregSummary>),
    Percentile(PercentileWindow),
    Cross(CrossState),
    Since(SinceState),
}

/// Serializable-in-memory state snapshot produced by `Executor::snapshot`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutorSnapshot {
    pub(crate) ir_fingerprint: u64,
    pub(crate) bar_count: u64,
    pub(crate) node_states: Vec<NodeState>,
    pub(crate) node_vals: Vec<NodeVal>,
}
