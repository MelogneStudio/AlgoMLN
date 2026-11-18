use std::collections::HashMap;

pub const IST_OFFSET_SECONDS: i64 = 19_800;

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
}
