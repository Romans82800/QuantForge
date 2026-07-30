use quantforge_data::Bar;

/// Swing-base zone extreme with forward carry (matches export `sqSwingBaseZoneExtreme`).
///
/// A pivot at `pivot` becomes actionable only once both are available:
/// - the right-hand swing confirmation (`swing_right` bars after the pivot)
/// - the post-pivot base window (`base_bars` bars after the pivot)
///
/// Ready delay is therefore `max(swing_right, base_bars)`. Using only
/// `swing_right` dropped zones whenever `base_bars > swing_right` (common on
/// short-side reclaim genes), which silently killed short signals in Rust
/// while MT5 still formed the zone from full history.
pub fn swing_base_zone_series(
    bars: &[Bar],
    swing_left: usize,
    swing_right: usize,
    base_bars: usize,
    zone_high: bool,
) -> Vec<f64> {
    let mut output = vec![f64::NAN; bars.len()];
    if swing_left == 0 || swing_right == 0 || base_bars == 0 {
        return output;
    }
    let ready_delay = swing_right.max(base_bars);
    let mut last_zone = f64::NAN;
    for (index, output_value) in output.iter_mut().enumerate() {
        if index >= ready_delay {
            let pivot = index - ready_delay;
            if pivot >= swing_left {
                let is_swing_low = (1..=swing_left)
                    .all(|offset| bars[pivot].low <= bars[pivot - offset].low)
                    && (1..=swing_right)
                        .all(|offset| bars[pivot].low < bars[pivot + offset].low);
                let is_swing_high = (1..=swing_left)
                    .all(|offset| bars[pivot].high >= bars[pivot - offset].high)
                    && (1..=swing_right)
                        .all(|offset| bars[pivot].high > bars[pivot + offset].high);
                let use_pivot = if zone_high {
                    is_swing_low
                } else {
                    is_swing_high
                };
                if use_pivot {
                    let base_start = pivot + 1;
                    let base_end = base_start + base_bars - 1;
                    if base_end < bars.len() && base_end <= index {
                        let window = &bars[base_start..=base_end];
                        last_zone = if zone_high {
                            window
                                .iter()
                                .map(|bar| bar.high)
                                .fold(f64::NEG_INFINITY, f64::max)
                        } else {
                            window
                                .iter()
                                .map(|bar| bar.low)
                                .fold(f64::INFINITY, f64::min)
                        };
                    }
                }
            }
        }
        *output_value = last_zone;
    }
    output
}
