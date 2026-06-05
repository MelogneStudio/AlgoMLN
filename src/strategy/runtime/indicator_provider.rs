use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::indicators::{atr, bbands, ema, ma, rel_vol, rsi, vwap};
use crate::models::Candle;
use crate::strategy::dsl::IndicatorKind;

pub trait IndicatorProvider: Send + Sync {
    fn get(&mut self, kind: &IndicatorKind, period: usize, candles: &[Candle]) -> Option<f64>;

    fn clear_cache(&mut self) {}

    fn advance(&mut self, _candle: &Candle) {}

    fn profile(&self) -> IndicatorProviderProfile {
        IndicatorProviderProfile::default()
    }
}

#[derive(Debug, Default)]
pub struct FullRecomputeProvider {
    cache: HashMap<(IndicatorKind, usize), f64>,
    profile: IndicatorProviderProfile,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct IndicatorProviderProfile {
    pub get_calls: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub get_time: Duration,
}

impl FullRecomputeProvider {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            profile: IndicatorProviderProfile::default(),
        }
    }
}

impl IndicatorProvider for FullRecomputeProvider {
    fn get(&mut self, kind: &IndicatorKind, period: usize, candles: &[Candle]) -> Option<f64> {
        let started = Instant::now();
        self.profile.get_calls += 1;

        if period == 0 || candles.is_empty() {
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
        let value = latest_indicator_value(kind, period, candles);
        self.profile.get_time += started.elapsed();
        let value = value?;
        self.cache.insert(key, value);
        Some(value)
    }

    fn clear_cache(&mut self) {
        self.cache.clear();
    }

    fn profile(&self) -> IndicatorProviderProfile {
        self.profile
    }
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
    fn benchmark_1000_candles_with_full_recompute_provider() {
        let mut provider = FullRecomputeProvider::new();
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
            "FullRecomputeProvider: 1000 candles, EMA(20)+RSI(14) = {:?}",
            elapsed
        );
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
