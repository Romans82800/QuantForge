use crate::model::EvalError;
use chrono::{Datelike, NaiveDate, Timelike};
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
            let values =
                calculate_indicator_series_with_clock(self.bars, indicator, Some(&self.broker_clock));
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
        // Mirror IndicatorExpr::period_and_shift validation ladder.
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
            | Self::RateOfChange { period, shift, .. }
            | Self::LiquiditySweepScore { period, shift } => (period, shift),
            Self::SessionRangeHigh {
                range_bars, shift, ..
            }
            | Self::SessionRangeLow {
                range_bars, shift, ..
            } => (range_bars.max(2), shift),
            Self::BodyRangeRatio { shift } | Self::CloseLocationInBar { shift } => (2, shift),
            Self::AtrPercentile {
                atr_period,
                lookback,
                shift,
            } => (atr_period.max(lookback).max(2), shift),
            Self::SwingBaseZoneHigh {
                swing_left,
                swing_right,
                base_bars,
                shift,
            }
            | Self::SwingBaseZoneLow {
                swing_left,
                swing_right,
                base_bars,
                shift,
            } => (
                swing_left
                    .saturating_add(swing_right)
                    .saturating_add(base_bars)
                    .max(2),
                shift,
            ),
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
///
/// Session-range indicators require a broker clock; without one they return NaN.
pub fn calculate_indicator_series(bars: &[Bar], indicator: &IndicatorExpr) -> Vec<f64> {
    calculate_indicator_series_with_clock(bars, indicator, None)
}

pub fn calculate_indicator_series_with_clock(
    bars: &[Bar],
    indicator: &IndicatorExpr,
    clock: Option<&BrokerClock>,
) -> Vec<f64> {
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
        IndicatorExpr::SessionRangeHigh {
            start_hour,
            range_bars,
            ..
        } => session_range_series(bars, clock, start_hour, range_bars as usize, true),
        IndicatorExpr::SessionRangeLow {
            start_hour,
            range_bars,
            ..
        } => session_range_series(bars, clock, start_hour, range_bars as usize, false),
        IndicatorExpr::BodyRangeRatio { .. } => bars
            .iter()
            .map(|bar| {
                let range = bar.high - bar.low;
                if range > 0.0 {
                    (bar.close - bar.open).abs() / range
                } else {
                    f64::NAN
                }
            })
            .collect(),
        IndicatorExpr::CloseLocationInBar { .. } => bars
            .iter()
            .map(|bar| {
                let range = bar.high - bar.low;
                if range > 0.0 {
                    (bar.close - bar.low) / range
                } else {
                    f64::NAN
                }
            })
            .collect(),
        IndicatorExpr::AtrPercentile {
            atr_period,
            lookback,
            ..
        } => atr_percentile_series(bars, atr_period as usize, lookback as usize),
        IndicatorExpr::SwingBaseZoneHigh {
            swing_left,
            swing_right,
            base_bars,
            ..
        } => swing_base_zone_series(
            bars,
            swing_left as usize,
            swing_right as usize,
            base_bars as usize,
            true,
        ),
        IndicatorExpr::SwingBaseZoneLow {
            swing_left,
            swing_right,
            base_bars,
            ..
        } => swing_base_zone_series(
            bars,
            swing_left as usize,
            swing_right as usize,
            base_bars as usize,
            false,
        ),
        IndicatorExpr::LiquiditySweepScore { period, .. } => {
            liquidity_sweep_score_series(bars, period as usize)
        }
    }
}

fn session_range_series(
    bars: &[Bar],
    clock: Option<&BrokerClock>,
    start_hour: u8,
    range_bars: usize,
    high: bool,
) -> Vec<f64> {
    let mut output = vec![f64::NAN; bars.len()];
    let Some(clock) = clock else {
        return output;
    };
    if range_bars == 0 {
        return output;
    }
    let mut day_cursor: Option<NaiveDate> = None;
    let mut window_start: Option<usize> = None;
    let mut frozen: Option<f64> = None;
    for (index, bar) in bars.iter().enumerate() {
        let Ok(local) = clock.local_datetime(bar.timestamp_ms) else {
            output[index] = frozen.unwrap_or(f64::NAN);
            continue;
        };
        let day = local.date();
        if day_cursor != Some(day) {
            day_cursor = Some(day);
            window_start = None;
            frozen = None;
        }
        if window_start.is_none() && local.hour() as u8 >= start_hour {
            window_start = Some(index);
        }
        if let Some(start) = window_start {
            let end = start + range_bars - 1;
            if index < end {
                output[index] = f64::NAN;
            } else if index == end || frozen.is_none() {
                let slice = &bars[start..=end.min(index).min(bars.len() - 1)];
                if slice.len() >= range_bars {
                    let window = &bars[start..start + range_bars];
                    frozen = Some(if high {
                        window
                            .iter()
                            .map(|bar| bar.high)
                            .fold(f64::NEG_INFINITY, f64::max)
                    } else {
                        window
                            .iter()
                            .map(|bar| bar.low)
                            .fold(f64::INFINITY, f64::min)
                    });
                }
                output[index] = frozen.unwrap_or(f64::NAN);
            } else {
                output[index] = frozen.unwrap_or(f64::NAN);
            }
        } else {
            output[index] = f64::NAN;
        }
    }
    output
}

fn atr_percentile_series(bars: &[Bar], atr_period: usize, lookback: usize) -> Vec<f64> {
    let atr_values = atr(bars, atr_period);
    let mut output = vec![f64::NAN; bars.len()];
    if lookback == 0 {
        return output;
    }
    for index in 0..bars.len() {
        if index + 1 < lookback || !atr_values[index].is_finite() {
            continue;
        }
        let start = index + 1 - lookback;
        let window = &atr_values[start..=index];
        let current = atr_values[index];
        let mut finite: Vec<f64> = window
            .iter()
            .copied()
            .filter(|value| value.is_finite())
            .collect();
        if finite.is_empty() {
            continue;
        }
        finite.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rank = finite
            .iter()
            .filter(|value| **value <= current)
            .count();
        output[index] = rank as f64 / finite.len() as f64 * 100.0;
    }
    output
}

fn swing_base_zone_series(
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
    let mut last_zone = f64::NAN;
    for (index, output_value) in output.iter_mut().enumerate() {
        // Pivot at p is confirmed once we have `swing_right` bars after it.
        if index >= swing_right {
            let pivot = index - swing_right;
            if pivot >= swing_left {
                let is_swing_low = (1..=swing_left).all(|offset| {
                    bars[pivot].low <= bars[pivot - offset].low
                }) && (1..=swing_right).all(|offset| {
                    bars[pivot].low < bars[pivot + offset].low
                });
                let is_swing_high = (1..=swing_left).all(|offset| {
                    bars[pivot].high >= bars[pivot - offset].high
                }) && (1..=swing_right).all(|offset| {
                    bars[pivot].high > bars[pivot + offset].high
                });
                // Demand base follows swing low; supply base follows swing high.
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

fn liquidity_sweep_score_series(bars: &[Bar], period: usize) -> Vec<f64> {
    let mut output = vec![0.0; bars.len()];
    if period == 0 {
        return output;
    }
    for index in period..bars.len() {
        let window = &bars[index - period..index];
        let prior_high = window
            .iter()
            .map(|bar| bar.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let prior_low = window
            .iter()
            .map(|bar| bar.low)
            .fold(f64::INFINITY, f64::min);
        let bar = &bars[index];
        if bar.low < prior_low && bar.close > prior_low {
            output[index] = 1.0;
        } else if bar.high > prior_high && bar.close < prior_high {
            output[index] = -1.0;
        } else {
            output[index] = 0.0;
        }
    }
    output
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

    #[test]
    fn body_range_and_close_location_use_completed_candle_geometry() {
        let bars = vec![Bar {
            timestamp_ms: 0,
            open: 10.0,
            high: 14.0,
            low: 8.0,
            close: 13.0,
            tick_volume: 0,
            real_volume: 0,
            spread_points: Some(0),
        }];
        let body = calculate_indicator_series(
            &bars,
            &IndicatorExpr::BodyRangeRatio { shift: 1 },
        );
        let location = calculate_indicator_series(
            &bars,
            &IndicatorExpr::CloseLocationInBar { shift: 1 },
        );
        assert_eq!(body[0], 0.5);
        assert!((location[0] - 5.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn liquidity_sweep_score_marks_wick_beyond_prior_extreme() {
        let bars = vec![
            Bar {
                timestamp_ms: 0,
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: 60_000,
                open: 10.0,
                high: 10.5,
                low: 9.5,
                close: 10.2,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: 120_000,
                open: 10.0,
                high: 10.2,
                low: 8.5,
                close: 9.6,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
        ];
        let scores = calculate_indicator_series(
            &bars,
            &IndicatorExpr::LiquiditySweepScore {
                period: 2,
                shift: 1,
            },
        );
        assert_eq!(scores[0], 0.0);
        assert_eq!(scores[1], 0.0);
        assert_eq!(scores[2], 1.0);
    }

    #[test]
    fn session_range_holds_after_opening_window() {
        let clock = BrokerClock::parse("Etc/UTC").unwrap();
        // 10:00, 11:00, 12:00 UTC on the same day.
        let base = chrono::DateTime::parse_from_rfc3339("2024-01-02T10:00:00Z")
            .unwrap()
            .timestamp_millis();
        let bars = vec![
            Bar {
                timestamp_ms: base,
                open: 1.0,
                high: 2.0,
                low: 0.5,
                close: 1.5,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: base + 3_600_000,
                open: 1.5,
                high: 3.0,
                low: 1.0,
                close: 2.5,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: base + 7_200_000,
                open: 2.5,
                high: 2.8,
                low: 2.0,
                close: 2.2,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
        ];
        let highs = calculate_indicator_series_with_clock(
            &bars,
            &IndicatorExpr::SessionRangeHigh {
                start_hour: 10,
                range_bars: 2,
                shift: 1,
            },
            Some(&clock),
        );
        assert!(highs[0].is_nan());
        assert_eq!(highs[1], 3.0);
        assert_eq!(highs[2], 3.0);
    }
}
