/// SqZScore: population standard deviation over a fixed window (`SqZScore.mq5`).
pub fn zscore_series(values: &[f64], period: usize) -> Vec<f64> {
    let len = values.len();
    let mut output = vec![0.0; len];
    if period == 0 || len == 0 {
        return vec![f64::NAN; len];
    }

    for index in 0..len {
        if index + 1 < period {
            output[index] = 0.0;
            continue;
        }
        let mean = values[index + 1 - period..=index].iter().sum::<f64>() / period as f64;
        let variance = values[index + 1 - period..=index]
            .iter()
            .map(|value| {
                let diff = value - mean;
                diff * diff
            })
            .sum::<f64>()
            / period as f64;
        let deviation = variance.sqrt();
        output[index] = if deviation > 0.0 {
            (values[index] - mean) / deviation
        } else {
            0.0
        };
    }
    output
}
