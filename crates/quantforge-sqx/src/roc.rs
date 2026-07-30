/// SqROC: `(close[i] - close[i-period]) / close[i-period] * 100` (`SqROC.mq5`).
pub fn rate_of_change_series(values: &[f64], period: usize) -> Vec<f64> {
    let len = values.len();
    let mut output = vec![0.0; len];
    if period == 0 || len == 0 {
        return vec![f64::NAN; len];
    }
    output[0] = 0.0;
    for index in 1..len {
        let previous = if index >= period {
            values[index - period]
        } else {
            0.0
        };
        if previous == 0.0 {
            output[index] = 0.0;
        } else {
            output[index] = (values[index] - previous) / previous * 100.0;
        }
    }
    output
}
