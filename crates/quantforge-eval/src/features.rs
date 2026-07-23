use crate::model::EvalError;
use chrono::{Datelike, Timelike};
use quantforge_broker::BrokerClock;
use quantforge_data::Bar;
use quantforge_ir::{BoolExpr, ComparisonOp, ContextValue, IndicatorExpr, NumericExpr, PriceField};
use std::collections::BTreeMap;

/// Cached Strategy IR feature evaluator shared by Scout and the M1 judge.
///
/// Callers provide the decision-timeframe bars; all shifts are resolved
/// against those bars, never against execution-timeframe data.
pub struct FeatureCache<'a> {
    bars: &'a [Bar],
    broker_clock: BrokerClock,
    indicators: BTreeMap<String, Vec<f64>>,
}

impl<'a> FeatureCache<'a> {
    pub fn new(bars: &'a [Bar], timezone: &str) -> Result<Self, EvalError> {
        let broker_clock = BrokerClock::parse(timezone)
            .map_err(|_| EvalError::InvalidBrokerTimezone(timezone.into()))?;
        Ok(Self {
            bars,
            broker_clock,
            indicators: BTreeMap::new(),
        })
    }

    pub fn evaluate_bool(
        &mut self,
        expression: &BoolExpr,
        decision_index: usize,
    ) -> Result<bool, EvalError> {
        match expression {
            BoolExpr::Compare {
                comparison,
                left,
                right,
            } => {
                let Some(left) = self.numeric_value(left, decision_index, 0)? else {
                    return Ok(false);
                };
                let Some(right) = self.numeric_value(right, decision_index, 0)? else {
                    return Ok(false);
                };
                Ok(match comparison {
                    ComparisonOp::GreaterThan => left > right,
                    ComparisonOp::LessThan => left < right,
                })
            }
            BoolExpr::CrossAbove { left, right } => {
                self.evaluate_cross(left, right, decision_index, true)
            }
            BoolExpr::CrossBelow { left, right } => {
                self.evaluate_cross(left, right, decision_index, false)
            }
            BoolExpr::Between {
                value,
                lower,
                upper,
            } => {
                let Some(value) = self.numeric_value(value, decision_index, 0)? else {
                    return Ok(false);
                };
                let Some(lower) = self.numeric_value(lower, decision_index, 0)? else {
                    return Ok(false);
                };
                let Some(upper) = self.numeric_value(upper, decision_index, 0)? else {
                    return Ok(false);
                };
                Ok(value >= lower && value <= upper)
            }
            BoolExpr::And { children } => {
                for child in children {
                    if !self.evaluate_bool(child, decision_index)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            BoolExpr::Or { children } => {
                for child in children {
                    if self.evaluate_bool(child, decision_index)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            BoolExpr::Not { child } => Ok(!self.evaluate_bool(child, decision_index)?),
        }
    }

    pub fn indicator_at_decision(
        &mut self,
        indicator: &IndicatorExpr,
        decision_index: usize,
    ) -> Result<Option<f64>, EvalError> {
        self.indicator_value(indicator, decision_index, 0)
    }

    pub fn bars_for_eval(&self) -> &'a [Bar] {
        self.bars
    }

    fn evaluate_cross(
        &mut self,
        left: &NumericExpr,
        right: &NumericExpr,
        decision_index: usize,
        above: bool,
    ) -> Result<bool, EvalError> {
        let current_left = self.numeric_value(left, decision_index, 0)?;
        let current_right = self.numeric_value(right, decision_index, 0)?;
        let previous_left = self.numeric_value(left, decision_index, 1)?;
        let previous_right = self.numeric_value(right, decision_index, 1)?;
        let (Some(current_left), Some(current_right), Some(previous_left), Some(previous_right)) =
            (current_left, current_right, previous_left, previous_right)
        else {
            return Ok(false);
        };

        Ok(if above {
            current_left > current_right && previous_left <= previous_right
        } else {
            current_left < current_right && previous_left >= previous_right
        })
    }

    fn numeric_value(
        &mut self,
        expression: &NumericExpr,
        decision_index: usize,
        extra_shift: usize,
    ) -> Result<Option<f64>, EvalError> {
        match expression {
            NumericExpr::Price { field, shift } => {
                let Some(index) = shifted_index(decision_index, *shift, extra_shift) else {
                    return Ok(None);
                };
                Ok(Some(price(&self.bars[index], *field)))
            }
            NumericExpr::Indicator { value } => {
                self.indicator_value(value, decision_index, extra_shift)
            }
            NumericExpr::Context { value, shift } => {
                let Some(index) = shifted_index(decision_index, *shift, extra_shift) else {
                    return Ok(None);
                };
                let local = self
                    .broker_clock
                    .local_datetime(self.bars[index].timestamp_ms)
                    .map_err(|_| EvalError::InvalidConfig("context timestamp is invalid".into()))?;
                Ok(Some(match value {
                    ContextValue::SessionHour => local.hour() as f64,
                    ContextValue::DayOfWeek => local.weekday().num_days_from_sunday() as f64,
                }))
            }
            NumericExpr::Constant { value } => Ok(value.is_finite().then_some(*value)),
        }
    }

    fn indicator_value(
        &mut self,
        indicator: &IndicatorExpr,
        decision_index: usize,
        extra_shift: usize,
    ) -> Result<Option<f64>, EvalError> {
        let shift = indicator.period_and_shift_for_eval().1;
        let Some(index) = shifted_index(decision_index, shift, extra_shift) else {
            return Ok(None);
        };
        let key = serde_json::to_string(indicator)?;
        if !self.indicators.contains_key(&key) {
            let values = calculate_indicator_series(self.bars, indicator);
            self.indicators.insert(key.clone(), values);
        }
        Ok(self.indicators[&key]
            .get(index)
            .copied()
            .filter(|value| value.is_finite()))
    }
}

trait IndicatorEvalFields {
    fn period_and_shift_for_eval(&self) -> (u16, u16);
}

impl IndicatorEvalFields for IndicatorExpr {
    fn period_and_shift_for_eval(&self) -> (u16, u16) {
        match *self {
            Self::Sma { period, shift, .. }
            | Self::Ema { period, shift, .. }
            | Self::Wma { period, shift, .. }
            | Self::Rsi { period, shift, .. }
            | Self::Atr { period, shift }
            | Self::DonchianHigh { period, shift }
            | Self::DonchianLow { period, shift }
            | Self::Highest { period, shift, .. }
            | Self::Lowest { period, shift, .. }
            | Self::StandardDeviation { period, shift, .. }
            | Self::ZScore { period, shift, .. }
            | Self::PercentileInRange { period, shift, .. }
            | Self::RateOfChange { period, shift, .. } => (period, shift),
        }
    }
}

fn shifted_index(decision_index: usize, shift: u16, extra_shift: usize) -> Option<usize> {
    decision_index.checked_sub(shift as usize + extra_shift)
}

fn price(bar: &Bar, field: PriceField) -> f64 {
    match field {
        PriceField::Open => bar.open,
        PriceField::High => bar.high,
        PriceField::Low => bar.low,
        PriceField::Close => bar.close,
    }
}

fn source_series(bars: &[Bar], field: PriceField) -> Vec<f64> {
    bars.iter().map(|bar| price(bar, field)).collect()
}

/// Calculate the raw, unshifted indicator buffer used by the Scout evaluator.
///
/// Undefined warm-up values are represented by `NaN`. The `shift` carried by
/// `IndicatorExpr` is deliberately not applied here: shift is a lookup concern
/// in both Scout and MQL5, while this function exposes the underlying buffer for
/// numerical parity tests.
pub fn calculate_indicator_series(bars: &[Bar], indicator: &IndicatorExpr) -> Vec<f64> {
    match *indicator {
        IndicatorExpr::Sma { source, period, .. } => {
            rolling_mean(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::Ema { source, period, .. } => {
            ema(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::Wma { source, period, .. } => {
            wma(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::Rsi { source, period, .. } => {
            rsi(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::Atr { period, .. } => atr(bars, period as usize),
        IndicatorExpr::DonchianHigh { period, .. } => rolling_extreme(
            &source_series(bars, PriceField::High),
            period as usize,
            true,
        ),
        IndicatorExpr::DonchianLow { period, .. } => rolling_extreme(
            &source_series(bars, PriceField::Low),
            period as usize,
            false,
        ),
        IndicatorExpr::Highest { source, period, .. } => {
            rolling_extreme(&source_series(bars, source), period as usize, true)
        }
        IndicatorExpr::Lowest { source, period, .. } => {
            rolling_extreme(&source_series(bars, source), period as usize, false)
        }
        IndicatorExpr::StandardDeviation { source, period, .. } => {
            rolling_standard_deviation(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::ZScore { source, period, .. } => {
            let values = source_series(bars, source);
            let means = rolling_mean(&values, period as usize);
            let deviations = rolling_standard_deviation(&values, period as usize);
            values
                .iter()
                .zip(means.iter().zip(deviations.iter()))
                .map(|(value, (mean, deviation))| {
                    if deviation.is_finite() && *deviation > 0.0 {
                        (value - mean) / deviation
                    } else {
                        f64::NAN
                    }
                })
                .collect()
        }
        IndicatorExpr::PercentileInRange { source, period, .. } => {
            let values = source_series(bars, source);
            let lows = rolling_extreme(&values, period as usize, false);
            let highs = rolling_extreme(&values, period as usize, true);
            values
                .iter()
                .zip(lows.iter().zip(highs.iter()))
                .map(|(value, (low, high))| {
                    if low.is_finite() && high.is_finite() && high > low {
                        (value - low) / (high - low) * 100.0
                    } else {
                        f64::NAN
                    }
                })
                .collect()
        }
        IndicatorExpr::RateOfChange { source, period, .. } => {
            let values = source_series(bars, source);
            let mut output = vec![f64::NAN; values.len()];
            for index in period as usize..values.len() {
                let previous = values[index - period as usize];
                if previous != 0.0 {
                    output[index] = (values[index] / previous - 1.0) * 100.0;
                }
            }
            output
        }
    }
}

fn rolling_mean(values: &[f64], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 || values.len() < period {
        return output;
    }
    let mut sum = 0.0;
    for (index, value) in values.iter().enumerate() {
        sum += value;
        if index >= period {
            sum -= values[index - period];
        }
        if index + 1 >= period {
            output[index] = sum / period as f64;
        }
    }
    output
}

fn ema(values: &[f64], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 || values.len() < period {
        return output;
    }
    let seed = values[..period].iter().sum::<f64>() / period as f64;
    output[period - 1] = seed;
    let alpha = 2.0 / (period as f64 + 1.0);
    for index in period..values.len() {
        output[index] = alpha * values[index] + (1.0 - alpha) * output[index - 1];
    }
    output
}

fn wma(values: &[f64], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 || values.len() < period {
        return output;
    }
    let denominator = (period * (period + 1) / 2) as f64;
    for index in period - 1..values.len() {
        let start = index + 1 - period;
        output[index] = values[start..=index]
            .iter()
            .enumerate()
            .map(|(offset, value)| (offset + 1) as f64 * value)
            .sum::<f64>()
            / denominator;
    }
    output
}

fn rolling_extreme(values: &[f64], period: usize, maximum: bool) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 || values.len() < period {
        return output;
    }
    for index in period - 1..values.len() {
        let window = &values[index + 1 - period..=index];
        output[index] = if maximum {
            window.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        } else {
            window.iter().copied().fold(f64::INFINITY, f64::min)
        };
    }
    output
}

fn rolling_standard_deviation(values: &[f64], period: usize) -> Vec<f64> {
    let means = rolling_mean(values, period);
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 || values.len() < period {
        return output;
    }
    for index in period - 1..values.len() {
        let mean = means[index];
        let variance = values[index + 1 - period..=index]
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / period as f64;
        output[index] = variance.sqrt();
    }
    output
}

fn rsi(values: &[f64], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 || values.len() <= period {
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

    for index in period + 1..values.len() {
        let change = values[index] - values[index - 1];
        average_gain = (average_gain * (period - 1) as f64 + change.max(0.0)) / period as f64;
        average_loss = (average_loss * (period - 1) as f64 + (-change).max(0.0)) / period as f64;
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

fn atr(bars: &[Bar], period: usize) -> Vec<f64> {
    let true_ranges: Vec<f64> = bars
        .iter()
        .enumerate()
        .map(|(index, bar)| {
            if index == 0 {
                bar.high - bar.low
            } else {
                (bar.high - bar.low)
                    .max((bar.high - bars[index - 1].close).abs())
                    .max((bar.low - bars[index - 1].close).abs())
            }
        })
        .collect();
    // MT5 iATR is the simple rolling mean of true range. Keeping this exact is
    // more important than substituting the commonly used Wilder recurrence:
    // strategies exported to MQL5 must see the same buffer as Scout.
    rolling_mean(&true_ranges, period)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars() -> Vec<Bar> {
        (0..6)
            .map(|index| Bar {
                timestamp_ms: 1_704_067_200_000 + index * 60_000,
                open: index as f64 + 1.0,
                high: index as f64 + 2.0,
                low: index as f64,
                close: index as f64 + 1.0,
                tick_volume: 1,
                real_volume: 0,
                spread_points: Some(0),
            })
            .collect()
    }

    #[test]
    fn sma_uses_only_completed_shifted_bars() {
        let bars = bars();
        let mut cache = FeatureCache::new(&bars, "Etc/UTC").unwrap();
        let indicator = IndicatorExpr::Sma {
            source: PriceField::Close,
            period: 3,
            shift: 1,
        };

        assert_eq!(
            cache.indicator_at_decision(&indicator, 4).unwrap(),
            Some(3.0)
        );
    }

    #[test]
    fn atr_matches_mt5_rolling_true_range_mean() {
        let bars = vec![
            Bar {
                timestamp_ms: 0,
                open: 9.0,
                high: 10.0,
                low: 8.0,
                close: 9.0,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: 60_000,
                open: 9.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: 120_000,
                open: 11.0,
                high: 11.5,
                low: 10.5,
                close: 11.0,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
        ];
        let values = calculate_indicator_series(
            &bars,
            &IndicatorExpr::Atr {
                period: 2,
                shift: 0,
            },
        );
        assert!(values[0].is_nan());
        assert_eq!(values[1], 2.5);
        assert_eq!(values[2], 2.0);
    }
}
