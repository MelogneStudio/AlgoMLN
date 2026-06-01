use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct TriggerStateMap {
    states: BTreeMap<String, bool>,
}

impl TriggerStateMap {
    pub fn new() -> Self {
        Self {
            states: BTreeMap::new(),
        }
    }

    pub fn should_fire(&mut self, rule_id: &str, is_true_now: bool) -> bool {
        let was_true = *self.states.get(rule_id).unwrap_or(&false);
        self.states.insert(rule_id.to_string(), is_true_now);
        !was_true && is_true_now
    }

    pub fn was_true(&self, rule_id: &str) -> bool {
        *self.states.get(rule_id).unwrap_or(&false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_on_false_to_true() {
        let mut state = TriggerStateMap::new();
        assert!(state.should_fire("rule_0", true));
    }

    #[test]
    fn does_not_fire_on_true_to_true() {
        let mut state = TriggerStateMap::new();
        state.should_fire("rule_0", true);
        assert!(!state.should_fire("rule_0", true));
    }

    #[test]
    fn does_not_fire_on_true_to_false() {
        let mut state = TriggerStateMap::new();
        state.should_fire("rule_0", true);
        assert!(!state.should_fire("rule_0", false));
    }

    #[test]
    fn does_not_fire_on_false_to_false() {
        let mut state = TriggerStateMap::new();
        assert!(!state.should_fire("rule_0", false));
    }

    #[test]
    fn resets_and_fires_again_after_false() {
        let mut state = TriggerStateMap::new();
        state.should_fire("rule_0", true);
        state.should_fire("rule_0", false);
        assert!(state.should_fire("rule_0", true));
    }

    #[test]
    fn independent_state_per_rule() {
        let mut state = TriggerStateMap::new();
        state.should_fire("rule_0", true);
        assert!(state.should_fire("rule_1", true));
    }
}
