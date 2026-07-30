use quantforge_data::Bar;

/// Expanding/simple TR mean used by `SqATRPercentile.mq5` (not SqATR recurrence).
fn atr_at(bars: &[Bar], index: usize, period: usize) -> f64 {
    let window = (index + 1).min(period);
    if window == 0 {
        return 0.0;
    }
    let mut sum = 0.0;
    for offset in 0..window {
        let bar_index = index - offset;
        let bar = &bars[bar_index];
        let mut true_range = bar.high - bar.low;
        if bar_index > 0 {
            let previous_close = bars[bar_index - 1].close;
            true_range = true_range
                .max((bar.high - previous_close).abs())
                .max((bar.low - previous_close).abs());
        }
        sum += true_range;
    }
    sum / window as f64
}

/// SqATRPercentile rank: share of lookback ATR samples `<=` current (`SqATRPercentile.mq5`).
pub fn atr_percentile_series(bars: &[Bar], atr_period: usize, lookback: usize) -> Vec<f64> {
    let len = bars.len();
    if atr_period == 0 || lookback == 0 || len == 0 {
        return vec![f64::NAN; len];
    }
    let lookback = lookback.max(10);
    // Every bar ranks its whole lookback window of ATR samples, so materializing
    // the ATR series once turns O(bars * lookback * period) into O(bars * period).
    // Each entry is summed in the same order as before, so values are unchanged.
    let atr: Vec<f64> = (0..len)
        .map(|index| atr_at(bars, index, atr_period))
        .collect();

    let mut output = vec![0.0; len];
    for index in 0..len {
        let current = atr[index];
        let window = (index + 1).min(lookback);
        if window < 2 || current <= 0.0 {
            output[index] = 0.0;
            continue;
        }
        let below_or_equal = atr[index + 1 - window..=index]
            .iter()
            .filter(|value| **value <= current)
            .count();
        output[index] = 100.0 * below_or_equal as f64 / window as f64;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-optimization form, kept verbatim as the parity oracle.
    #[allow(clippy::needless_range_loop)]
    fn reference(bars: &[Bar], atr_period: usize, lookback: usize) -> Vec<f64> {
        let len = bars.len();
        let mut output = vec![0.0; len];
        if atr_period == 0 || lookback == 0 || len == 0 {
            return vec![f64::NAN; len];
        }
        let lookback = lookback.max(10);
        for index in 0..len {
            let current = atr_at(bars, index, atr_period);
            let window = (index + 1).min(lookback);
            if window < 2 || current <= 0.0 {
                output[index] = 0.0;
                continue;
            }
            let below_or_equal = (0..window)
                .filter(|offset| atr_at(bars, index - offset, atr_period) <= current)
                .count();
            output[index] = 100.0 * below_or_equal as f64 / window as f64;
        }
        output
    }

    fn series(count: usize) -> Vec<Bar> {
        (0..count)
            .map(|index| {
                let base = 100.0 + (index as f64 * 0.37).sin() * 4.0 + index as f64 * 0.01;
                let span = 0.3 + ((index % 7) as f64) * 0.11;
                Bar {
                    timestamp_ms: index as i64 * 3_600_000,
                    open: base,
                    high: base + span,
                    low: base - span,
                    close: base + span * 0.25,
                    tick_volume: 100,
                    real_volume: 0,
                    spread_points: Some(1),
                }
            })
            .collect()
    }

    #[test]
    fn precomputed_atr_percentile_is_bit_identical_to_the_nested_form() {
        let bars = series(400);
        for (atr_period, lookback) in [(14, 60), (5, 10), (20, 120), (1, 10), (14, 3)] {
            let fast = atr_percentile_series(&bars, atr_period, lookback);
            let slow = reference(&bars, atr_period, lookback);
            assert_eq!(
                fast.len(),
                slow.len(),
                "length mismatch at {atr_period}/{lookback}"
            );
            for (index, (left, right)) in fast.iter().zip(slow.iter()).enumerate() {
                assert_eq!(
                    left.to_bits(),
                    right.to_bits(),
                    "bar {index} diverged at atr_period {atr_period}, lookback {lookback}"
                );
            }
        }
    }

    #[test]
    fn degenerate_inputs_still_return_nan_series() {
        let bars = series(12);
        assert!(atr_percentile_series(&bars, 0, 10)
            .iter()
            .all(|value| value.is_nan()));
        assert!(atr_percentile_series(&bars, 14, 0)
            .iter()
            .all(|value| value.is_nan()));
        assert!(atr_percentile_series(&[], 14, 10).is_empty());
    }
}
