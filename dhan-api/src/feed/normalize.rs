use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const IST_OFFSET_SECONDS: i64 = 19_800;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LttConvention {
    Utc,
    IstShifted,
}

impl std::fmt::Display for LttConvention {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Utc => write!(f, "UTC"),
            Self::IstShifted => write!(f, "IST-shifted"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LttDetectionResult {
    /// Fresh tick with detected convention and normalized UTC epoch timestamp.
    Valid {
        time_utc: i64,
        convention: LttConvention,
    },
    /// ltt <= 0 means no trade time.
    NoTradeTime,
    /// Neither UTC nor IST-shifted matched within tolerance.
    Stale {
        raw_ltt: i64,
        recv_utc: i64,
    },
}

#[derive(Clone, Debug)]
pub struct LttDetectorConfig {
    pub tolerance_seconds: i64,
    pub consecutive_ticks_to_lock: usize,
}

impl Default for LttDetectorConfig {
    fn default() -> Self {
        Self {
            tolerance_seconds: 60,
            consecutive_ticks_to_lock: 3,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LttDetector {
    pub config: LttDetectorConfig,
    pub locked_convention: Option<LttConvention>,
    pub candidate_convention: Option<LttConvention>,
    pub consecutive_matches: usize,
    pub stale_count: usize,
    pub no_trade_time_count: usize,
    pub valid_count: usize,
    pub convention_flips: usize,
}

impl Default for LttDetector {
    fn default() -> Self {
        Self::new(LttDetectorConfig::default())
    }
}

impl LttDetector {
    pub fn new(config: LttDetectorConfig) -> Self {
        Self {
            config,
            locked_convention: None,
            candidate_convention: None,
            consecutive_matches: 0,
            stale_count: 0,
            no_trade_time_count: 0,
            valid_count: 0,
            convention_flips: 0,
        }
    }

    pub fn with_tolerance(tolerance_seconds: i64) -> Self {
        Self::new(LttDetectorConfig {
            tolerance_seconds,
            consecutive_ticks_to_lock: 3,
        })
    }

    pub fn detect(&mut self, ltt: i32, recv_utc: i64) -> LttDetectionResult {
        if ltt <= 0 {
            self.no_trade_time_count += 1;
            return LttDetectionResult::NoTradeTime;
        }

        let raw_ltt = ltt as i64;
        let diff_utc = (recv_utc - raw_ltt).abs();
        let diff_ist = (recv_utc - (raw_ltt - IST_OFFSET_SECONDS)).abs();

        let matched = if diff_utc < self.config.tolerance_seconds {
            Some(LttConvention::Utc)
        } else if diff_ist < self.config.tolerance_seconds {
            Some(LttConvention::IstShifted)
        } else {
            None
        };

        let convention = match matched {
            Some(c) => c,
            None => {
                self.stale_count += 1;
                return LttDetectionResult::Stale { raw_ltt, recv_utc };
            }
        };

        // Handle locking and debounced flipping
        if let Some(locked) = self.locked_convention {
            if convention == locked {
                self.candidate_convention = Some(locked);
                self.consecutive_matches = self.config.consecutive_ticks_to_lock;
            } else {
                if self.candidate_convention == Some(convention) {
                    self.consecutive_matches += 1;
                    if self.consecutive_matches >= self.config.consecutive_ticks_to_lock {
                        eprintln!(
                            "WARNING: LTT convention flipped from {} to {}! (recv_utc={}, raw_ltt={})",
                            locked, convention, recv_utc, raw_ltt
                        );
                        self.convention_flips += 1;
                        self.locked_convention = Some(convention);
                    }
                } else {
                    self.candidate_convention = Some(convention);
                    self.consecutive_matches = 1;
                }
            }
        } else {
            if self.candidate_convention == Some(convention) {
                self.consecutive_matches += 1;
                if self.consecutive_matches >= self.config.consecutive_ticks_to_lock {
                    self.locked_convention = Some(convention);
                }
            } else {
                self.candidate_convention = Some(convention);
                self.consecutive_matches = 1;
                if self.config.consecutive_ticks_to_lock <= 1 {
                    self.locked_convention = Some(convention);
                }
            }
        }

        let time_utc = match convention {
            LttConvention::Utc => raw_ltt,
            LttConvention::IstShifted => raw_ltt - IST_OFFSET_SECONDS,
        };

        self.valid_count += 1;
        LttDetectionResult::Valid {
            time_utc,
            convention,
        }
    }
}

/// Corrects Dhan's LTT (IST wall-clock encoded as an epoch) to true UTC epoch seconds.
/// True UTC epoch is `ltt - 19800`.
/// Raw LTT must never leave the feed module.
#[inline]
pub fn ltt_to_utc_seconds(ltt: i32) -> i64 {
    (ltt as i64) - IST_OFFSET_SECONDS
}

/// Normalizes f32 price to integer paise, removing floating-point noise.
/// Example: 1300.8 arrives as 1300.800048828125 -> 130080 paise.
#[inline]
pub fn ltp_to_paise(ltp: f32) -> i64 {
    (ltp as f64 * 100.0).round() as i64
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub security_id: i32,
    pub price_paise: i64,
    pub time_utc: i64,
    pub volume: Option<i64>,
    pub recv_ns: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Deduper {
    last_ticks: HashMap<i32, (i64, i64)>, // security_id -> (time_utc, price_paise)
}

impl Deduper {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns true if this tick is accepted (new price or new LTT timestamp),
    /// or false if it is a duplicate (same time_utc and price_paise for the same instrument).
    pub fn accept(&mut self, security_id: i32, time_utc: i64, price_paise: i64) -> bool {
        if let Some(&(last_time, last_price)) = self.last_ticks.get(&security_id) {
            if last_time == time_utc && last_price == price_paise {
                return false;
            }
        }
        self.last_ticks.insert(security_id, (time_utc, price_paise));
        true
    }

    pub fn clear(&mut self) {
        self.last_ticks.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ltt_correction() {
        assert_eq!(ltt_to_utc_seconds(1700000000), 1700000000 - 19800);
        assert_eq!(ltt_to_utc_seconds(19800), 0);
        assert_eq!(ltt_to_utc_seconds(0), -19800);
    }

    #[test]
    fn test_paise_rounding() {
        // Spec test: f32 noise 1300.8 arrives as 1300.800048828125
        let noisy: f32 = 1300.800048828125;
        assert_eq!(ltp_to_paise(noisy), 130080);
        assert_eq!(ltp_to_paise(1367.0), 136700);
        assert_eq!(ltp_to_paise(1344.8), 134480);
        assert_eq!(ltp_to_paise(0.05), 5);
        assert_eq!(ltp_to_paise(10.005), 1001);
    }

    #[test]
    fn test_deduper() {
        let mut deduper = Deduper::new();

        // Initial tick is accepted
        assert!(deduper.accept(2885, 1700000000, 130080));

        // Exact duplicate (same time and price) is dropped
        assert!(!deduper.accept(2885, 1700000000, 130080));

        // Unchanged price with a new LTT is a real tick and is kept
        assert!(deduper.accept(2885, 1700000001, 130080));

        // Changed price with same LTT is kept
        assert!(deduper.accept(2885, 1700000001, 130100));

        // Another duplicate of the latest tick is dropped
        assert!(!deduper.accept(2885, 1700000001, 130100));

        // Different security ID does not collide
        assert!(deduper.accept(2886, 1700000001, 130100));
        assert!(!deduper.accept(2886, 1700000001, 130100));
    }

    #[test]
    fn test_ltt_detector_utc_convention() {
        let mut detector = LttDetector::default();
        let recv_utc = 1795145959; // 2026-11-20 09:09:19 IST in true UTC
        let raw_ltt = 1795145959; // matches UTC exactly

        let res = detector.detect(raw_ltt as i32, recv_utc);
        assert_eq!(
            res,
            LttDetectionResult::Valid {
                time_utc: 1795145959,
                convention: LttConvention::Utc,
            }
        );
        assert_eq!(detector.valid_count, 1);
        assert_eq!(detector.stale_count, 0);
        assert_eq!(detector.no_trade_time_count, 0);
    }

    #[test]
    fn test_ltt_detector_ist_shifted_convention() {
        let mut detector = LttDetector::default();
        let recv_utc = 1794978910; // 2026-11-18 14:55:10 IST in true UTC
        let raw_ltt = 1794978910 + IST_OFFSET_SECONDS; // 1794998710 (IST wall-clock encoded as epoch)

        let res = detector.detect(raw_ltt as i32, recv_utc);
        assert_eq!(
            res,
            LttDetectionResult::Valid {
                time_utc: 1794978910,
                convention: LttConvention::IstShifted,
            }
        );
        assert_eq!(detector.valid_count, 1);
        assert_eq!(detector.stale_count, 0);
    }

    #[test]
    fn test_ltt_detector_stale_ltt() {
        let mut detector = LttDetector::default();
        let recv_utc = 1795145400; // 2026-11-20 morning
        let raw_ltt = 1795084192; // stale LTT from previous session (Nov 19)

        let res = detector.detect(raw_ltt as i32, recv_utc);
        assert_eq!(
            res,
            LttDetectionResult::Stale {
                raw_ltt: 1795084192,
                recv_utc: 1795145400,
            }
        );
        assert_eq!(detector.stale_count, 1);
        assert_eq!(detector.valid_count, 0);
    }

    #[test]
    fn test_ltt_detector_zero_and_negative_ltt() {
        let mut detector = LttDetector::default();
        assert_eq!(detector.detect(0, 1795145000), LttDetectionResult::NoTradeTime);
        assert_eq!(detector.detect(-5, 1795145000), LttDetectionResult::NoTradeTime);
        assert_eq!(detector.no_trade_time_count, 2);
        assert_eq!(detector.valid_count, 0);
    }

    #[test]
    fn test_ltt_detector_locking_and_flip_warning() {
        let mut detector = LttDetector::new(LttDetectorConfig {
            tolerance_seconds: 60,
            consecutive_ticks_to_lock: 3,
        });

        let base_utc = 1794978910;
        let ist_shifted_ltt = (base_utc + IST_OFFSET_SECONDS) as i32;

        // Tick 1 (IST-shifted)
        assert_eq!(
            detector.detect(ist_shifted_ltt, base_utc),
            LttDetectionResult::Valid {
                time_utc: base_utc,
                convention: LttConvention::IstShifted,
            }
        );
        assert_eq!(detector.locked_convention, None);

        // Tick 2 (IST-shifted)
        detector.detect(ist_shifted_ltt + 1, base_utc + 1);
        assert_eq!(detector.locked_convention, None);

        // Tick 3 (IST-shifted) -> locks!
        detector.detect(ist_shifted_ltt + 2, base_utc + 2);
        assert_eq!(detector.locked_convention, Some(LttConvention::IstShifted));
        assert_eq!(detector.convention_flips, 0);

        // Now Dhan flips convention to true UTC.
        // Isolated tick 1: does NOT flip locked convention yet
        let utc_ltt = (base_utc + 10) as i32;
        detector.detect(utc_ltt, base_utc + 10);
        assert_eq!(detector.locked_convention, Some(LttConvention::IstShifted));
        assert_eq!(detector.convention_flips, 0);

        // Tick 2: still pending confirmation
        detector.detect(utc_ltt + 1, base_utc + 11);
        assert_eq!(detector.locked_convention, Some(LttConvention::IstShifted));
        assert_eq!(detector.convention_flips, 0);

        // Tick 3: reaches consecutive_ticks_to_lock (3) -> confirms flip!
        let res = detector.detect(utc_ltt + 2, base_utc + 12);
        assert_eq!(
            res,
            LttDetectionResult::Valid {
                time_utc: base_utc + 12,
                convention: LttConvention::Utc,
            }
        );
        assert_eq!(detector.convention_flips, 1);
        assert_eq!(detector.locked_convention, Some(LttConvention::Utc));
    }

    #[test]
    fn test_ltt_detector_configurable_tolerance() {
        let mut tight = LttDetector::with_tolerance(10);
        let now = 1795145000;
        // 15 seconds drift -> stale with 10s tolerance
        assert_eq!(
            tight.detect((now - 15) as i32, now),
            LttDetectionResult::Stale {
                raw_ltt: now - 15,
                recv_utc: now
            }
        );

        let mut loose = LttDetector::with_tolerance(60);
        // 15 seconds drift -> valid with 60s tolerance
        assert_eq!(
            loose.detect((now - 15) as i32, now),
            LttDetectionResult::Valid {
                time_utc: now - 15,
                convention: LttConvention::Utc
            }
        );
    }
}
