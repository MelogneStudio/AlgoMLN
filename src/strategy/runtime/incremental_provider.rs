use std::collections::HashMap;
use std::time::Instant;

use crate::indicators::{atr, bbands, ema, ma, rel_vol, rsi, vwap};
use crate::models::Candle;
use crate::strategy::dsl::IndicatorKind;

use super::indicator_provider::{IndicatorProvider, IndicatorProviderProfile};

/// Maximum candles retained in the rolling window.
/// Must be >= the largest indicator period the user might configure.
pub const MAX_WINDOW: usize = 500;

#[derive(Debug, Default)]
pub struct BoundedWindowProvider {
    window: Vec<Candle>,
    cache: HashMap<(IndicatorKind, usize), f64>,
    profile: IndicatorProviderProfile,
}

impl BoundedWindowProvider {
    pub fn new() -> Self {
        Self {
            window: Vec::with_capacity(MAX_WINDOW + 1),
            cache: HashMap::new(),
            profile: IndicatorProviderProfile::default(),
        }
    }

    fn sync_window(&mut self, candles: &[Candle]) {
        let Some(current) = candles.last() else {
            return;
        };

        if self.window.last() == Some(current) {
            return;
        }

        if self.window.is_empty()
            || candles
                .get(candles.len().saturating_sub(2))
                .is_some_and(|previous| self.window.last() == Some(previous))
        {
            self.advance(current);
            self.cache.clear();
            return;
        }

        self.window.clear();
        let start = candles.len().saturating_sub(MAX_WINDOW);
        self.window.extend_from_slice(&candles[start..]);
        self.cache.clear();
    }
}

impl IndicatorProvider for BoundedWindowProvider {
    fn get(&mut self, kind: &IndicatorKind, period: usize, candles: &[Candle]) -> Option<f64> {
        let started = Instant::now();
        self.profile.get_calls += 1;
        self.sync_window(candles);

        if period == 0 || self.window.is_empty() {
            self.profile.cache_misses += 1;
            self.profile.get_time += started.elapsed();
            return None;
        }

        let key = (kind.clone(), period);
        if let Some(value) = self.cache.get(&key) {
            self.profile.cache_hits += 1;
            self.profile.get_time += started.elapsed();
            return Some(*value);
        }

        self.profile.cache_misses += 1;
        let value = latest_indicator_value(kind, period, &self.window);
        self.profile.get_time += started.elapsed();
        let value = value?;
        self.cache.insert(key, value);
        Some(value)
    }

    fn clear_cache(&mut self) {
        self.cache.clear();
    }

    fn advance(&mut self, candle: &Candle) {
        if self.window.last() == Some(candle) {
            return;
        }

        self.window.push(candle.clone());
        if self.window.len() > MAX_WINDOW {
            self.window.remove(0);
        }
    }

    fn profile(&self) -> IndicatorProviderProfile {
        self.profile
    }
}

fn latest_indicator_value(kind: &IndicatorKind, period: usize, candles: &[Candle]) -> Option<f64> {
    let values = match kind {
        IndicatorKind::Ma => ma(candles, period),
        IndicatorKind::Ema => ema(candles, period),
        IndicatorKind::Rsi => rsi(candles, period),
        IndicatorKind::RelVol => {
            return rel_vol(candles, period)
                .last()
                .copied()
                .filter(|value| value.is_finite());
        }
        IndicatorKind::Atr => atr(candles, period),
        IndicatorKind::Vwap => {
            return vwap(candles)
                .last()
                .copied()
                .flatten()
                .map(|point| point.value)
                .filter(|value| value.is_finite());
        }
        IndicatorKind::BbUpper => {
            return bbands(candles, period, 2.0)
                .last()
                .copied()
                .flatten()
                .map(|bands| bands.upper)
                .filter(|value| value.is_finite());
        }
        IndicatorKind::BbLower => {
            return bbands(candles, period, 2.0)
                .last()
                .copied()
                .flatten()
                .map(|bands| bands.lower)
                .filter(|value| value.is_finite());
        }
        IndicatorKind::BbMid => {
            return bbands(candles, period, 2.0)
                .last()
                .copied()
                .flatten()
                .map(|bands| bands.mid)
                .filter(|value| value.is_finite());
        }
    };

    values
        .last()
        .copied()
        .flatten()
        .filter(|value| value.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candle(close: f64) -> Candle {
        Candle {
            timestamp: close as i64,
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
        }
    }

    #[test]
    fn window_does_not_grow_past_max() {
        let mut provider = BoundedWindowProvider::new();
        for i in 0..(MAX_WINDOW + 50) {
            provider.advance(&make_candle(i as f64));
        }

        assert_eq!(provider.window.len(), MAX_WINDOW);
    }

    #[test]
    fn returns_none_before_period_satisfied() {
        let mut provider = BoundedWindowProvider::new();
        for i in 0..13 {
            provider.advance(&make_candle(i as f64 + 10.0));
        }

        let result = provider.get(&IndicatorKind::Rsi, 14, &[]);

        assert!(result.is_none());
    }

    #[test]
    fn returns_value_once_period_satisfied() {
        let mut provider = BoundedWindowProvider::new();
        for i in 0..20 {
            provider.advance(&make_candle(i as f64 + 10.0));
        }

        let result = provider.get(&IndicatorKind::Ma, 5, &[]);

        assert!(result.is_some());
    }

    #[test]
    fn cache_hit_returns_same_value() {
        let mut provider = BoundedWindowProvider::new();
        for i in 0..20 {
            provider.advance(&make_candle(i as f64 + 10.0));
        }

        let v1 = provider.get(&IndicatorKind::Ma, 5, &[]);
        let v2 = provider.get(&IndicatorKind::Ma, 5, &[]);

        assert_eq!(v1, v2);
    }

    #[test]
    fn clear_cache_allows_recompute() {
        let mut provider = BoundedWindowProvider::new();
        for i in 0..20 {
            provider.advance(&make_candle(i as f64 + 10.0));
        }

        let v1 = provider.get(&IndicatorKind::Ma, 5, &[]);
        provider.clear_cache();
        provider.advance(&make_candle(200.0));
        let v2 = provider.get(&IndicatorKind::Ma, 5, &[]);

        assert_ne!(v1, v2);
    }

    #[test]
    fn benchmark_1000_candles_with_bounded_provider() {
        use std::time::Instant;

        let mut provider = BoundedWindowProvider::new();
        let candles: Vec<Candle> = (0..1000).map(|i| make_candle(i as f64 + 10.0)).collect();

        let start = Instant::now();
        for i in 1..=candles.len() {
            provider.clear_cache();
            provider.get(&IndicatorKind::Ema, 20, &candles[..i]);
            provider.get(&IndicatorKind::Rsi, 14, &candles[..i]);
            provider.advance(&candles[i - 1]);
        }
        let elapsed = start.elapsed();

        println!(
            "BoundedWindowProvider: 1000 candles, EMA(20)+RSI(14) = {:?}",
            elapsed
        );
    }
}
