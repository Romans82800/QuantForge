use quantforge_data::Bar;

/// SqADX buffers: `(ADX, +DI, -DI)` matching `SqADX.mq5`.
pub fn directional_index(bars: &[Bar], period: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let len = bars.len();
    let mut adx = vec![0.0; len];
    let mut plus_di = vec![0.0; len];
    let mut minus_di = vec![0.0; len];
    if period == 0 || len == 0 {
        return (
            vec![f64::NAN; len],
            vec![f64::NAN; len],
            vec![f64::NAN; len],
        );
    }

    let mut sum_tr = vec![0.0; len];
    let mut sum_dm_plus = vec![0.0; len];
    let mut sum_dm_minus = vec![0.0; len];
    sum_tr[0] = bars[0].high - bars[0].low;

    for index in 1..len {
        let true_range = bars[index].high - bars[index].low;
        let delta_hh = bars[index].high - bars[index - 1].high;
        let delta_ll = bars[index - 1].low - bars[index].low;
        let delta_hc = (bars[index].high - bars[index - 1].close).abs();
        let delta_lc = (bars[index].low - bars[index - 1].close).abs();
        let tr = delta_lc.max(true_range.max(delta_hc));
        let dm_plus = if delta_hh > delta_ll {
            delta_hh.max(0.0)
        } else {
            0.0
        };
        let dm_minus = if delta_ll > delta_hh {
            delta_ll.max(0.0)
        } else {
            0.0
        };

        if index < period {
            sum_tr[index] = sum_tr[index - 1] + tr;
            sum_dm_plus[index] = sum_dm_plus[index - 1] + dm_plus;
            sum_dm_minus[index] = sum_dm_minus[index - 1] + dm_minus;
        } else {
            sum_tr[index] =
                sum_tr[index - 1] - sum_tr[index - 1] / period as f64 + tr;
            sum_dm_plus[index] =
                sum_dm_plus[index - 1] - sum_dm_plus[index - 1] / period as f64 + dm_plus;
            sum_dm_minus[index] =
                sum_dm_minus[index - 1] - sum_dm_minus[index - 1] / period as f64 + dm_minus;
        }

        plus_di[index] = if sum_tr[index] == 0.0 {
            0.0
        } else {
            100.0 * sum_dm_plus[index] / sum_tr[index]
        };
        minus_di[index] = if sum_tr[index] == 0.0 {
            0.0
        } else {
            100.0 * sum_dm_minus[index] / sum_tr[index]
        };

        let diff = (plus_di[index] - minus_di[index]).abs();
        let sum = plus_di[index] + minus_di[index];
        adx[index] = if sum == 0.0 {
            50.0
        } else {
            ((period - 1) as f64 * adx[index - 1] + 100.0 * diff / sum) / period as f64
        };
    }

    (adx, plus_di, minus_di)
}
