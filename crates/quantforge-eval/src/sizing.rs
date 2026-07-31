//! Helper: martingale lot scale from consecutive-loss streak.

/// `base * multiplier ^ min(streak, max_steps)`.
pub fn martingale_lots(base_lots: f64, multiplier: f64, max_steps: u8, loss_streak: u8) -> f64 {
    let steps = loss_streak.min(max_steps) as i32;
    if !base_lots.is_finite() || base_lots <= 0.0 || !multiplier.is_finite() || multiplier < 1.0 {
        return 0.0;
    }
    base_lots * multiplier.powi(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_then_caps() {
        assert!((martingale_lots(0.1, 2.0, 3, 0) - 0.1).abs() < 1e-12);
        assert!((martingale_lots(0.1, 2.0, 3, 1) - 0.2).abs() < 1e-12);
        assert!((martingale_lots(0.1, 2.0, 3, 3) - 0.8).abs() < 1e-12);
        assert!((martingale_lots(0.1, 2.0, 3, 9) - 0.8).abs() < 1e-12);
    }
}
