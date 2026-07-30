/// SqRSI / Java `RSICalculator`: SMA seed then Wilder smoothing.
pub fn rsi_series(values: &[f64], period: usize) -> Vec<f64> {
    let len = values.len();
    let mut output = vec![0.0; len];
    if period == 0 || len == 0 {
        return vec![f64::NAN; len];
    }

    if len <= period {
        return output;
    }

    let mut average_gain = 0.0;
    let mut average_loss = 0.0;
    for index in 1..=period {
        let change = values[index] - values[index - 1];
        average_gain += change.max(0.0);
        average_loss += (-change).max(0.0);
    }
    average_gain /= period as f64;
    average_loss /= period as f64;
    output[period] = rsi_value(average_gain, average_loss);

    for index in period + 1..len {
        let change = values[index] - values[index - 1];
        average_gain =
            (average_gain * (period - 1) as f64 + change.max(0.0)) / period as f64;
        average_loss =
            (average_loss * (period - 1) as f64 + (-change).max(0.0)) / period as f64;
        output[index] = rsi_value(average_gain, average_loss);
    }
    output
}

fn rsi_value(average_gain: f64, average_loss: f64) -> f64 {
    if average_gain == 0.0 && average_loss == 0.0 {
        50.0
    } else if average_loss == 0.0 {
        100.0
    } else {
        100.0 - 100.0 / (1.0 + average_gain / average_loss)
    }
}
