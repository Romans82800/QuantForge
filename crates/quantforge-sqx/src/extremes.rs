use quantforge_data::Bar;
use std::collections::VecDeque;

/// SqHighest: zero warm-up, inclusive rolling maximum (`SqHighest.mq5`).
pub fn highest_series(values: &[f64], period: usize) -> Vec<f64> {
    rolling_extreme(values, period, true)
}

/// SqLowest: zero warm-up, inclusive rolling minimum (`SqLowest.mq5`).
pub fn lowest_series(values: &[f64], period: usize) -> Vec<f64> {
    rolling_extreme(values, period, false)
}

fn rolling_extreme(values: &[f64], period: usize, maximum: bool) -> Vec<f64> {
    let len = values.len();
    if period == 0 || len == 0 {
        return vec![f64::NAN; len];
    }
    let mut output = vec![0.0; len];
    rolling_extreme_into(values, period, maximum, &mut output);
    output
}

/// Monotonic-deque rolling extreme, writing only full windows (`index >= period - 1`).
///
/// Warm-up entries are left untouched because the SQX and MT5 indicator engines
/// disagree on them (`0.0` versus `NaN`), so each caller pre-fills its own.
///
/// Output is bit-identical to a per-bar window scan: the minimum or maximum of a set
/// does not depend on visit order, and neither operation accumulates error. NaN is the
/// one exception — `f64::max` skips it while `<=` comparisons reject it — so a
/// NaN-bearing input falls back to the scan rather than risking a silent divergence.
pub fn rolling_extreme_into(values: &[f64], period: usize, maximum: bool, output: &mut [f64]) {
    if period == 0 || values.len() < period {
        return;
    }
    if values.iter().any(|value| value.is_nan()) {
        scan_extreme_into(values, period, maximum, output);
        return;
    }
    // Holds indices whose values are candidates for the window extreme, ordered
    // so the front is always the current answer.
    let mut candidates: VecDeque<usize> = VecDeque::with_capacity(period);
    for index in 0..values.len() {
        while candidates
            .front()
            .is_some_and(|oldest| oldest + period <= index)
        {
            candidates.pop_front();
        }
        while candidates.back().is_some_and(|last| {
            if maximum {
                values[*last] <= values[index]
            } else {
                values[*last] >= values[index]
            }
        }) {
            candidates.pop_back();
        }
        candidates.push_back(index);
        if index + 1 >= period {
            let front = *candidates.front().expect("just pushed the current index");
            output[index] = values[front];
        }
    }
}

fn scan_extreme_into(values: &[f64], period: usize, maximum: bool, output: &mut [f64]) {
    for index in period - 1..values.len() {
        let window = &values[index + 1 - period..=index];
        output[index] = if maximum {
            window.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        } else {
            window.iter().copied().fold(f64::INFINITY, f64::min)
        };
    }
}

#[allow(dead_code)]
pub fn highest_from_bars(bars: &[Bar], high: bool, period: usize) -> Vec<f64> {
    let values: Vec<f64> = bars
        .iter()
        .map(|bar| if high { bar.high } else { bar.low })
        .collect();
    if high {
        highest_series(&values, period)
    } else {
        lowest_series(&values, period)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-optimization form, kept as the parity oracle.
    fn reference(values: &[f64], period: usize, maximum: bool) -> Vec<f64> {
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
            let start = index + 1 - period;
            let mut extreme = if maximum {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
            for value in &values[start..=index] {
                extreme = if maximum {
                    extreme.max(*value)
                } else {
                    extreme.min(*value)
                };
            }
            output[index] = extreme;
        }
        output
    }

    fn wave(count: usize) -> Vec<f64> {
        (0..count)
            .map(|index| {
                let index = index as f64;
                100.0 + (index * 0.41).sin() * 6.0 - (index * 0.13).cos() * 2.5 + index * 0.02
            })
            .collect()
    }

    fn assert_bit_identical(values: &[f64], period: usize) {
        for maximum in [true, false] {
            let fast = rolling_extreme(values, period, maximum);
            let slow = reference(values, period, maximum);
            assert_eq!(fast.len(), slow.len());
            for (index, (left, right)) in fast.iter().zip(slow.iter()).enumerate() {
                assert_eq!(
                    left.to_bits(),
                    right.to_bits(),
                    "index {index} diverged at period {period}, maximum {maximum}"
                );
            }
        }
    }

    #[test]
    fn deque_extreme_is_bit_identical_to_the_window_scan() {
        let values = wave(500);
        for period in [1, 2, 3, 5, 14, 20, 60, 199, 500] {
            assert_bit_identical(&values, period);
        }
    }

    #[test]
    fn plateaus_and_monotonic_runs_stay_identical() {
        let flat = vec![100.0; 40];
        assert_bit_identical(&flat, 7);
        let rising: Vec<f64> = (0..40).map(|index| index as f64).collect();
        assert_bit_identical(&rising, 7);
        let falling: Vec<f64> = (0..40).map(|index| -(index as f64)).collect();
        assert_bit_identical(&falling, 7);
    }

    #[test]
    fn nan_inputs_fall_back_to_the_scan_semantics() {
        let mut values = wave(60);
        values[10] = f64::NAN;
        values[41] = f64::NAN;
        assert_bit_identical(&values, 14);
    }

    #[test]
    fn window_longer_than_the_series_keeps_the_zero_warmup() {
        assert_eq!(rolling_extreme(&wave(5), 10, true), vec![0.0; 5]);
        assert!(
            rolling_extreme(&wave(5), 0, true)
                .iter()
                .all(|value| value.is_nan())
        );
    }
}
