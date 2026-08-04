use quantforge_data::Bar;

/// SqATR: expanding-then-fixed-window SMA of true range (`SqATR.mq5`).
pub fn atr_series(bars: &[Bar], period: usize) -> Vec<f64> {
    let len = bars.len();
    let mut output = vec![f64::NAN; len];
    if period == 0 || len == 0 {
        return output;
    }

    output[0] = bars[0].high - bars[0].low;
    for index in 1..len {
        let mut true_range = bars[index].high - bars[index].low;
        let previous_close = bars[index - 1].close;
        true_range = true_range
            .max((bars[index].high - previous_close).abs())
            .max((bars[index].low - previous_close).abs());
        let previous = if output[index - 1].is_finite() {
            output[index - 1]
        } else {
            0.0
        };
        let window = (index + 1).min(period) as f64;
        output[index] = ((window - 1.0) * previous + true_range) / window;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::Bar;

    fn bar(close: f64, high: f64, low: f64) -> Bar {
        Bar {
            timestamp_ms: 0,
            open: close,
            high,
            low,
            close,
            tick_volume: 0,
            real_volume: 0,
            spread_points: None,
        }
    }

    #[test]
    fn first_bar_uses_range_only() {
        let bars = vec![bar(1.0, 1.2, 0.8)];
        let atr = atr_series(&bars, 14);
        assert!((atr[0] - 0.4).abs() < 1e-9);
    }

    #[test]
    fn expanding_window_before_period() {
        let bars = vec![bar(1.0, 1.1, 0.9), bar(1.1, 1.2, 1.0), bar(1.0, 1.15, 0.95)];
        let atr = atr_series(&bars, 14);
        assert!(atr.iter().all(|value| value.is_finite() && *value > 0.0));
    }
}
