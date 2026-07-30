use quantforge_data::Bar;

fn is_swing_high(bars: &[Bar], index: usize, period: usize) -> bool {
    if index < period || index + period >= bars.len() {
        return false;
    }
    let pivot = bars[index].high;
    (1..=period).all(|offset| bars[index - offset].high < pivot)
        && (1..=period).all(|offset| bars[index + offset].high < pivot)
}

fn is_swing_low(bars: &[Bar], index: usize, period: usize) -> bool {
    if index < period || index + period >= bars.len() {
        return false;
    }
    let pivot = bars[index].low;
    (1..=period).all(|offset| bars[index - offset].low > pivot)
        && (1..=period).all(|offset| bars[index + offset].low > pivot)
}

/// SqLiquiditySweep score: `+1` bull sweep, `-1` bear sweep, else `0`.
pub fn liquidity_sweep_score_series(bars: &[Bar], period: usize) -> Vec<f64> {
    let len = bars.len();
    let mut output = vec![0.0; len];
    if period == 0 || len == 0 {
        return vec![f64::NAN; len];
    }
    let period = period.max(2);
    let mut last_swing_high = 0.0;
    let mut last_swing_low = 0.0;

    for index in period..len {
        let check = index.saturating_sub(period);
        if check >= period {
            if is_swing_high(bars, check, period) {
                last_swing_high = bars[check].high;
            }
            if is_swing_low(bars, check, period) {
                last_swing_low = bars[check].low;
            }
        }
        let bar = &bars[index];
        if last_swing_low > 0.0 && bar.low < last_swing_low && bar.close > last_swing_low {
            output[index] = 1.0;
        } else if last_swing_high > 0.0 && bar.high > last_swing_high && bar.close < last_swing_high {
            output[index] = -1.0;
        }
    }
    output
}
