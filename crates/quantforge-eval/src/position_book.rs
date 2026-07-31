//! Multi-slot hedged position book for Scout replay.
//!
//! [`PositionAccounting::HedgedSingle`] / [`Netting`] keep at most one open.
//! [`HedgedStack`] allows concurrent positions up to `max_open_positions`.

use crate::model::{PositionAccounting, ScoutConfig};

pub(crate) fn max_open_slots(config: &ScoutConfig) -> usize {
    match config.position_accounting {
        PositionAccounting::HedgedSingle | PositionAccounting::Netting => 1,
        PositionAccounting::HedgedStack => config.max_open_positions.max(1),
    }
}

pub(crate) fn can_open_another(open_count: usize, config: &ScoutConfig) -> bool {
    open_count < max_open_slots(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PositionAccounting, ScoutConfig};

    #[test]
    fn single_and_netting_cap_at_one() {
        let mut config = ScoutConfig::default();
        config.max_open_positions = 8;
        assert_eq!(max_open_slots(&config), 1);
        config.position_accounting = PositionAccounting::Netting;
        assert_eq!(max_open_slots(&config), 1);
        assert!(!can_open_another(1, &config));
    }

    #[test]
    fn stack_honors_max_open_positions() {
        let mut config = ScoutConfig::default();
        config.position_accounting = PositionAccounting::HedgedStack;
        config.max_open_positions = 3;
        assert_eq!(max_open_slots(&config), 3);
        assert!(can_open_another(0, &config));
        assert!(can_open_another(2, &config));
        assert!(!can_open_another(3, &config));
    }
}
