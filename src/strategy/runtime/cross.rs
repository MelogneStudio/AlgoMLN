use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct CrossDetector {
    prev_values: BTreeMap<CrossStateKey, f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CrossStateKey {
    pub rule_id: String,
    pub side: CrossSide,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum CrossSide {
    Fast,
    Slow,
}

impl CrossDetector {
    pub fn new() -> Self {
        Self {
            prev_values: BTreeMap::new(),
        }
    }

    pub fn is_cross_above(&self, rule_id: &str, fast_curr: f64, slow_curr: f64) -> bool {
        match (
            self.get_prev(rule_id, CrossSide::Fast),
            self.get_prev(rule_id, CrossSide::Slow),
        ) {
            (Some(fast_prev), Some(slow_prev)) => fast_prev <= slow_prev && fast_curr > slow_curr,
            _ => false,
        }
    }

    pub fn is_cross_below(&self, rule_id: &str, fast_curr: f64, slow_curr: f64) -> bool {
        match (
            self.get_prev(rule_id, CrossSide::Fast),
            self.get_prev(rule_id, CrossSide::Slow),
        ) {
            (Some(fast_prev), Some(slow_prev)) => fast_prev >= slow_prev && fast_curr < slow_curr,
            _ => false,
        }
    }

    pub fn update(&mut self, rule_id: &str, fast_curr: f64, slow_curr: f64) {
        self.prev_values.insert(
            CrossStateKey {
                rule_id: rule_id.to_string(),
                side: CrossSide::Fast,
            },
            fast_curr,
        );
        self.prev_values.insert(
            CrossStateKey {
                rule_id: rule_id.to_string(),
                side: CrossSide::Slow,
            },
            slow_curr,
        );
    }

    fn get_prev(&self, rule_id: &str, side: CrossSide) -> Option<f64> {
        self.prev_values
            .get(&CrossStateKey {
                rule_id: rule_id.to_string(),
                side,
            })
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_false_with_no_previous_values() {
        let detector = CrossDetector::new();
        assert!(!detector.is_cross_above("rule_0", 50.0, 40.0));
        assert!(!detector.is_cross_below("rule_0", 40.0, 50.0));
    }

    #[test]
    fn detects_cross_above_on_exact_candle() {
        let mut detector = CrossDetector::new();
        detector.update("rule_0", 48.0, 50.0);
        assert!(detector.is_cross_above("rule_0", 52.0, 50.0));
    }

    #[test]
    fn does_not_fire_cross_above_after_crossover() {
        let mut detector = CrossDetector::new();
        detector.update("rule_0", 48.0, 50.0);
        assert!(detector.is_cross_above("rule_0", 52.0, 50.0));
        detector.update("rule_0", 52.0, 50.0);
        assert!(!detector.is_cross_above("rule_0", 55.0, 50.0));
    }

    #[test]
    fn detects_cross_below_on_exact_candle() {
        let mut detector = CrossDetector::new();
        detector.update("rule_0", 52.0, 50.0);
        assert!(detector.is_cross_below("rule_0", 48.0, 50.0));
    }

    #[test]
    fn does_not_fire_cross_below_after_crossover() {
        let mut detector = CrossDetector::new();
        detector.update("rule_0", 52.0, 50.0);
        assert!(detector.is_cross_below("rule_0", 48.0, 50.0));
        detector.update("rule_0", 48.0, 50.0);
        assert!(!detector.is_cross_below("rule_0", 45.0, 50.0));
    }

    #[test]
    fn independent_state_per_rule() {
        let mut detector = CrossDetector::new();
        detector.update("rule_0", 48.0, 50.0);
        assert!(!detector.is_cross_above("rule_1", 52.0, 50.0));
    }
}
