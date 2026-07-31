//! Position-sizing helpers shared by Scout / Judge / export.

/// `base * multiplier ^ min(streak, max_steps)`.
pub fn martingale_lots(base_lots: f64, multiplier: f64, max_steps: u8, loss_streak: u8) -> f64 {
    let steps = loss_streak.min(max_steps) as i32;
    if !base_lots.is_finite() || base_lots <= 0.0 || !multiplier.is_finite() || multiplier < 1.0 {
        return 0.0;
    }
    base_lots * multiplier.powi(steps)
}

/// SQX ATRRiskBasedSizing: risk `percent` of balance against ATR×multiplier money risk.
pub fn atr_risk_lots(
    balance: f64,
    percent: f64,
    atr_value: f64,
    atr_multiplier: f64,
    tick_size: f64,
    tick_value: f64,
    cost_risk_per_lot: f64,
    max_lots: Option<f64>,
) -> f64 {
    if !balance.is_finite()
        || balance <= 0.0
        || !percent.is_finite()
        || percent <= 0.0
        || !atr_value.is_finite()
        || atr_value <= 0.0
        || !atr_multiplier.is_finite()
        || atr_multiplier <= 0.0
        || !tick_size.is_finite()
        || tick_size <= 0.0
        || !tick_value.is_finite()
        || tick_value <= 0.0
    {
        return 0.0;
    }
    let stop_distance = atr_value * atr_multiplier;
    let price_risk_per_lot = stop_distance / tick_size * tick_value;
    let denom = price_risk_per_lot + cost_risk_per_lot.max(0.0);
    if denom <= 0.0 {
        return 0.0;
    }
    let mut lots = (balance * percent / 100.0) / denom;
    if let Some(max) = max_lots.filter(|value| value.is_finite() && *value > 0.0) {
        lots = lots.min(max);
    }
    lots
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

    #[test]
    fn atr_risk_sizes_from_volatility_distance() {
        // balance 10k, 1% risk = 100; ATR*mult = 2; tick_value=1 → 50 lots, capped to 10.
        let lots = atr_risk_lots(10_000.0, 1.0, 2.0, 1.0, 1.0, 1.0, 0.0, Some(10.0));
        assert!((lots - 10.0).abs() < 1e-12);
        let uncapped = atr_risk_lots(10_000.0, 1.0, 2.0, 1.0, 1.0, 1.0, 0.0, None);
        assert!((uncapped - 50.0).abs() < 1e-12);
    }
}
