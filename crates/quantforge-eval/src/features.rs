use crate::model::{EvalError, IndicatorEngine};
use chrono::{Datelike, NaiveDate, Timelike};
use quantforge_broker::BrokerClock;
use quantforge_data::Bar;
use quantforge_ir::{BoolExpr, ComparisonOp, ContextValue, IndicatorExpr, NumericExpr, PriceField};
use std::collections::HashMap;
use std::sync::Arc;

/// Default ceiling for one shared indicator cache. At 40k bars a buffer costs
/// ~320 KB, so this holds roughly 800 distinct indicator/period combinations.
pub const DEFAULT_INDICATOR_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// Indicator buffers for ONE bar series, reusable across candidates and threads.
///
/// A search population is bred from shared parents, so the same `ATR(14)` or
/// `Ema(close, 20)` buffer is otherwise recomputed once per candidate. Buffers are
/// immutable once built, so they are handed out as `Arc` and never copied.
///
/// Each instance belongs to exactly one bar series — see [`FeatureCache::with_shared_cache`].
pub struct IndicatorBufferCache {
    maximum_buffers: usize,
    state: std::sync::RwLock<IndicatorCacheState>,
}

type BufferKey = (IndicatorEngine, IndicatorExpr);

#[derive(Default)]
struct IndicatorCacheState {
    buffers: HashMap<BufferKey, Arc<Vec<f64>>>,
    /// Insertion order, used to evict the oldest buffers once the budget is hit.
    order: std::collections::VecDeque<BufferKey>,
}

impl IndicatorBufferCache {
    pub fn new(bar_count: usize) -> Self {
        Self::with_budget(bar_count, DEFAULT_INDICATOR_CACHE_BYTES)
    }

    pub fn with_budget(bar_count: usize, budget_bytes: usize) -> Self {
        let bytes_per_buffer = bar_count.max(1) * std::mem::size_of::<f64>();
        Self {
            maximum_buffers: (budget_bytes / bytes_per_buffer).max(1),
            state: std::sync::RwLock::new(IndicatorCacheState::default()),
        }
    }

    pub fn len(&self) -> usize {
        self.state.read().map(|state| state.buffers.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `compute` runs outside the lock, so a race can compute the same buffer twice.
    /// That is cheaper than serializing every miss behind a write lock, and the
    /// result is identical either way.
    fn get_or_compute(
        &self,
        key: &IndicatorExpr,
        engine: IndicatorEngine,
        compute: impl FnOnce() -> Vec<f64>,
    ) -> Arc<Vec<f64>> {
        let lookup = (engine, key.clone());
        if let Ok(state) = self.state.read() {
            if let Some(buffer) = state.buffers.get(&lookup) {
                return Arc::clone(buffer);
            }
        }
        let buffer = Arc::new(compute());
        let Ok(mut state) = self.state.write() else {
            return buffer;
        };
        if let Some(existing) = state.buffers.get(&lookup) {
            return Arc::clone(existing);
        }
        state.buffers.insert(lookup.clone(), Arc::clone(&buffer));
        state.order.push_back(lookup);
        while state.buffers.len() > self.maximum_buffers {
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            state.buffers.remove(&oldest);
        }
        buffer
    }
}

/// Cached Strategy IR feature evaluator shared by Scout and the M1 judge.
///
/// Callers provide the decision-timeframe bars; all shifts are resolved
/// against those bars, never against execution-timeframe data.
pub struct FeatureCache<'a> {
    bars: &'a [Bar],
    broker_clock: BrokerClock,
    indicator_engine: IndicatorEngine,
    /// Lock-free per-evaluation tier. Every per-bar read resolves here.
    indicators: HashMap<IndicatorExpr, Arc<Vec<f64>>>,
    /// Optional cross-candidate tier for the same bars. Consulted once per
    /// distinct indicator, never inside the per-bar path.
    shared: Option<&'a IndicatorBufferCache>,
}

impl<'a> FeatureCache<'a> {
    pub fn new(bars: &'a [Bar], timezone: &str) -> Result<Self, EvalError> {
        Self::with_engine(bars, timezone, IndicatorEngine::Sqx)
    }

    pub fn with_engine(
        bars: &'a [Bar],
        timezone: &str,
        indicator_engine: IndicatorEngine,
    ) -> Result<Self, EvalError> {
        Self::with_shared_cache(bars, timezone, indicator_engine, None)
    }

    /// `shared` MUST have been created for exactly these `bars`; buffers are keyed
    /// by indicator only, so mixing datasets would return values from the wrong series.
    pub fn with_shared_cache(
        bars: &'a [Bar],
        timezone: &str,
        indicator_engine: IndicatorEngine,
        shared: Option<&'a IndicatorBufferCache>,
    ) -> Result<Self, EvalError> {
        let broker_clock = BrokerClock::parse(timezone)
            .map_err(|_| EvalError::InvalidBrokerTimezone(timezone.into()))?;
        Ok(Self {
            bars,
            broker_clock,
            indicator_engine,
            indicators: HashMap::new(),
            shared,
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
        let key = indicator.buffer_key();
        let buffer = match self.indicators.get(&key) {
            Some(buffer) => buffer,
            None => {
                let buffer = self.load_buffer(&key);
                self.indicators.entry(key).or_insert(buffer)
            }
        };
        Ok(buffer.get(index).copied().filter(|value| value.is_finite()))
    }

    /// Resolves a buffer from the shared tier when present, otherwise computes it.
    fn load_buffer(&self, key: &IndicatorExpr) -> Arc<Vec<f64>> {
        let compute = || {
            calculate_indicator_series_with_clock(
                self.bars,
                key,
                Some(&self.broker_clock),
                self.indicator_engine,
            )
        };
        match self.shared {
            Some(shared) => shared.get_or_compute(key, self.indicator_engine, compute),
            None => Arc::new(compute()),
        }
    }
}

trait IndicatorEvalFields {
    fn period_and_shift_for_eval(&self) -> (u16, u16);
}

impl IndicatorEvalFields for IndicatorExpr {
    fn period_and_shift_for_eval(&self) -> (u16, u16) {
        self.period_and_shift()
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
    calculate_indicator_series_with_clock(bars, indicator, None, IndicatorEngine::Sqx)
}

pub fn calculate_indicator_series_with_clock(
    bars: &[Bar],
    indicator: &IndicatorExpr,
    clock: Option<&BrokerClock>,
    engine: IndicatorEngine,
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
        IndicatorExpr::Rsi { source, period, .. } => match engine {
            IndicatorEngine::Sqx => {
                quantforge_sqx::rsi_series(&source_series(bars, source), period as usize)
            }
            IndicatorEngine::Mt5 => rsi(&source_series(bars, source), period as usize),
        },
        IndicatorExpr::Atr { period, .. } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::atr_series(bars, period as usize),
            IndicatorEngine::Mt5 => atr(bars, period as usize),
        },
        IndicatorExpr::Adx { period, .. } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::directional_index(bars, period as usize).0,
            IndicatorEngine::Mt5 => directional_index(bars, period as usize).0,
        },
        IndicatorExpr::PlusDi { period, .. } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::directional_index(bars, period as usize).1,
            IndicatorEngine::Mt5 => directional_index(bars, period as usize).1,
        },
        IndicatorExpr::MinusDi { period, .. } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::directional_index(bars, period as usize).2,
            IndicatorEngine::Mt5 => directional_index(bars, period as usize).2,
        },
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
            let values = source_series(bars, source);
            match engine {
                IndicatorEngine::Sqx => quantforge_sqx::highest_series(&values, period as usize),
                IndicatorEngine::Mt5 => {
                    rolling_extreme(&values, period as usize, true)
                }
            }
        }
        IndicatorExpr::Lowest { source, period, .. } => {
            let values = source_series(bars, source);
            match engine {
                IndicatorEngine::Sqx => quantforge_sqx::lowest_series(&values, period as usize),
                IndicatorEngine::Mt5 => {
                    rolling_extreme(&values, period as usize, false)
                }
            }
        }
        IndicatorExpr::StandardDeviation { source, period, .. } => {
            rolling_standard_deviation(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::ZScore { source, period, .. } => match engine {
            IndicatorEngine::Sqx => {
                quantforge_sqx::zscore_series(&source_series(bars, source), period as usize)
            }
            IndicatorEngine::Mt5 => {
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
        },
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
            match engine {
                IndicatorEngine::Sqx => {
                    quantforge_sqx::rate_of_change_series(&values, period as usize)
                }
                IndicatorEngine::Mt5 => {
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
        IndicatorExpr::SessionRangeHigh {
            start_hour,
            range_bars,
            ..
        } => match (engine, clock) {
            (IndicatorEngine::Sqx, Some(clock)) => quantforge_sqx::session_range_series(
                bars,
                clock,
                start_hour,
                range_bars as usize,
                true,
            ),
            _ => session_range_series(bars, clock, start_hour, range_bars as usize, true),
        },
        IndicatorExpr::SessionRangeLow {
            start_hour,
            range_bars,
            ..
        } => match (engine, clock) {
            (IndicatorEngine::Sqx, Some(clock)) => quantforge_sqx::session_range_series(
                bars,
                clock,
                start_hour,
                range_bars as usize,
                false,
            ),
            _ => session_range_series(bars, clock, start_hour, range_bars as usize, false),
        },
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
        } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::atr_percentile_series(
                bars,
                atr_period as usize,
                lookback as usize,
            ),
            IndicatorEngine::Mt5 => {
                atr_percentile_series(bars, atr_period as usize, lookback as usize, engine)
            }
        },
        IndicatorExpr::SwingBaseZoneHigh {
            swing_left,
            swing_right,
            base_bars,
            ..
        } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::swing_base_zone_series(
                bars,
                swing_left as usize,
                swing_right as usize,
                base_bars as usize,
                true,
            ),
            IndicatorEngine::Mt5 => swing_base_zone_series(
                bars,
                swing_left as usize,
                swing_right as usize,
                base_bars as usize,
                true,
            ),
        },
        IndicatorExpr::SwingBaseZoneLow {
            swing_left,
            swing_right,
            base_bars,
            ..
        } => match engine {
            IndicatorEngine::Sqx => quantforge_sqx::swing_base_zone_series(
                bars,
                swing_left as usize,
                swing_right as usize,
                base_bars as usize,
                false,
            ),
            IndicatorEngine::Mt5 => swing_base_zone_series(
                bars,
                swing_left as usize,
                swing_right as usize,
                base_bars as usize,
                false,
            ),
        },
        IndicatorExpr::LiquiditySweepScore { period, .. } => match engine {
            IndicatorEngine::Sqx => {
                quantforge_sqx::liquidity_sweep_score_series(bars, period as usize)
            }
            IndicatorEngine::Mt5 => liquidity_sweep_score_series(bars, period as usize),
        },
        IndicatorExpr::MacdMain {
            source,
            fast_period,
            slow_period,
            ..
        } => macd_main_series(
            &source_series(bars, source),
            fast_period as usize,
            slow_period as usize,
        ),
        IndicatorExpr::MacdSignal {
            source,
            fast_period,
            slow_period,
            signal_period,
            ..
        } => {
            let main = macd_main_series(
                &source_series(bars, source),
                fast_period as usize,
                slow_period as usize,
            );
            ema_sparse(&main, signal_period as usize)
        }
        IndicatorExpr::MacdHistogram {
            source,
            fast_period,
            slow_period,
            signal_period,
            ..
        } => {
            let main = macd_main_series(
                &source_series(bars, source),
                fast_period as usize,
                slow_period as usize,
            );
            let signal = ema_sparse(&main, signal_period as usize);
            combine(&main, &signal, |main, signal| main - signal)
        }
        IndicatorExpr::BollingerMid { source, period, .. } => {
            rolling_mean(&source_series(bars, source), period as usize)
        }
        IndicatorExpr::BollingerUpper {
            source,
            period,
            deviation_tenths,
            ..
        } => bollinger_band(
            &source_series(bars, source),
            period as usize,
            tenths(deviation_tenths),
            true,
        ),
        IndicatorExpr::BollingerLower {
            source,
            period,
            deviation_tenths,
            ..
        } => bollinger_band(
            &source_series(bars, source),
            period as usize,
            tenths(deviation_tenths),
            false,
        ),
        IndicatorExpr::BollingerBandwidth {
            source,
            period,
            deviation_tenths,
            ..
        } => {
            let values = source_series(bars, source);
            let mid = rolling_mean(&values, period as usize);
            let deviation = rolling_standard_deviation(&values, period as usize);
            let multiplier = tenths(deviation_tenths);
            mid.iter()
                .zip(deviation.iter())
                .map(|(mid, deviation)| {
                    if mid.is_finite() && deviation.is_finite() && *mid != 0.0 {
                        2.0 * multiplier * deviation / mid * 100.0
                    } else {
                        f64::NAN
                    }
                })
                .collect()
        }
        IndicatorExpr::IchimokuTenkan { period, .. }
        | IndicatorExpr::IchimokuKijun { period, .. } => midpoint_series(bars, period as usize),
        IndicatorExpr::IchimokuSenkouA {
            tenkan_period,
            kijun_period,
            ..
        } => {
            let tenkan = midpoint_series(bars, tenkan_period as usize);
            let kijun = midpoint_series(bars, kijun_period as usize);
            let span = combine(&tenkan, &kijun, |tenkan, kijun| (tenkan + kijun) / 2.0);
            displace_forward(&span, kijun_period as usize)
        }
        IndicatorExpr::IchimokuSenkouB {
            period,
            kijun_period,
            ..
        } => {
            let span = midpoint_series(bars, period as usize);
            displace_forward(&span, kijun_period as usize)
        }
        IndicatorExpr::QqeLine {
            rsi_period,
            smoothing_period,
            ..
        } => ema_sparse(
            &qqe_rsi(bars, rsi_period as usize),
            smoothing_period as usize,
        ),
        IndicatorExpr::QqeTrail {
            rsi_period,
            smoothing_period,
            factor_tenths,
            ..
        } => qqe_trail_series(
            bars,
            rsi_period as usize,
            smoothing_period as usize,
            tenths(factor_tenths),
        ),
        IndicatorExpr::Vwap { period, .. } => vwap_series(bars, period as usize),
        IndicatorExpr::Cci { period, .. } => cci_series(bars, period as usize),
    }
}

fn tenths(value: u16) -> f64 {
    value as f64 / 10.0
}

/// QQE is defined on Wilder RSI in both export styles. Routing it through the
/// engine switch would make the smoothed line disagree with the generated EA,
/// and MT5 is the environment that ultimately trades.
fn qqe_rsi(bars: &[Bar], period: usize) -> Vec<f64> {
    rsi(&source_series(bars, PriceField::Close), period)
}

fn combine(left: &[f64], right: &[f64], operation: impl Fn(f64, f64) -> f64) -> Vec<f64> {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| {
            if left.is_finite() && right.is_finite() {
                operation(*left, *right)
            } else {
                f64::NAN
            }
        })
        .collect()
}

fn macd_main_series(values: &[f64], fast_period: usize, slow_period: usize) -> Vec<f64> {
    let fast = ema(values, fast_period);
    let slow = ema(values, slow_period);
    combine(&fast, &slow, |fast, slow| fast - slow)
}

fn bollinger_band(values: &[f64], period: usize, multiplier: f64, upper: bool) -> Vec<f64> {
    let mid = rolling_mean(values, period);
    let deviation = rolling_standard_deviation(values, period);
    combine(&mid, &deviation, |mid, deviation| {
        if upper {
            mid + multiplier * deviation
        } else {
            mid - multiplier * deviation
        }
    })
}

/// EMA over a buffer that may carry a `NaN` warm-up prefix. The average is
/// seeded from the first `period` finite values so a derived series (MACD main,
/// smoothed RSI) starts where its input becomes valid.
fn ema_sparse(values: &[f64], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    if period == 0 {
        return output;
    }
    let Some(first) = values.iter().position(|value| value.is_finite()) else {
        return output;
    };
    if first + period > values.len() {
        return output;
    }
    let seed = values[first..first + period].iter().sum::<f64>() / period as f64;
    if !seed.is_finite() {
        return output;
    }
    let mut previous = seed;
    output[first + period - 1] = seed;
    let alpha = 2.0 / (period as f64 + 1.0);
    for index in first + period..values.len() {
        if !values[index].is_finite() {
            continue;
        }
        previous = alpha * values[index] + (1.0 - alpha) * previous;
        output[index] = previous;
    }
    output
}

/// Midpoint of the highest high and lowest low over `period` bars — the shared
/// shape of Ichimoku's Tenkan, Kijun and Senkou B lines.
fn midpoint_series(bars: &[Bar], period: usize) -> Vec<f64> {
    let highs = rolling_extreme(&source_series(bars, PriceField::High), period, true);
    let lows = rolling_extreme(&source_series(bars, PriceField::Low), period, false);
    combine(&highs, &lows, |high, low| (high + low) / 2.0)
}

/// Shift a buffer forward so the value read at bar `i` was computed from bar
/// `i - offset`, matching how MT5 plots the Ichimoku cloud ahead of price.
fn displace_forward(values: &[f64], offset: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; values.len()];
    for index in offset..values.len() {
        output[index] = values[index - offset];
    }
    output
}

fn qqe_trail_series(
    bars: &[Bar],
    rsi_period: usize,
    smoothing_period: usize,
    factor: f64,
) -> Vec<f64> {
    let length = bars.len();
    let mut output = vec![f64::NAN; length];
    if rsi_period == 0 || smoothing_period == 0 {
        return output;
    }
    let smoothed = ema_sparse(&qqe_rsi(bars, rsi_period), smoothing_period);
    let wilder = rsi_period.saturating_mul(2).saturating_sub(1).max(1);

    let mut absolute = vec![f64::NAN; length];
    for index in 1..length {
        if smoothed[index].is_finite() && smoothed[index - 1].is_finite() {
            absolute[index] = (smoothed[index - 1] - smoothed[index]).abs();
        }
    }
    let smoothed_absolute = ema_sparse(&absolute, wilder);
    let deviation = ema_sparse(&smoothed_absolute, wilder);

    let mut long_band = f64::NAN;
    let mut short_band = f64::NAN;
    let mut trend_up = true;
    for index in 1..length {
        let (Some(current), Some(previous), Some(band)) = (
            finite(smoothed[index]),
            finite(smoothed[index - 1]),
            finite(deviation[index]),
        ) else {
            continue;
        };
        let room = band * factor;
        let new_long = current - room;
        let new_short = current + room;

        long_band = if long_band.is_finite() && previous > long_band && current > long_band {
            long_band.max(new_long)
        } else {
            new_long
        };
        short_band = if short_band.is_finite() && previous < short_band && current < short_band {
            short_band.min(new_short)
        } else {
            new_short
        };

        if current > short_band {
            trend_up = true;
        } else if current < long_band {
            trend_up = false;
        }
        output[index] = if trend_up { long_band } else { short_band };
    }
    output
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn typical_price_series(bars: &[Bar]) -> Vec<f64> {
    bars.iter()
        .map(|bar| (bar.high + bar.low + bar.close) / 3.0)
        .collect()
}

/// Rolling volume-weighted average price. Tick volume is the weight; feeds
/// without volume fall back to an equally weighted typical price so the buffer
/// stays defined instead of collapsing to NaN.
fn vwap_series(bars: &[Bar], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; bars.len()];
    if period == 0 || bars.len() < period {
        return output;
    }
    let typical = typical_price_series(bars);
    for index in period - 1..bars.len() {
        let start = index + 1 - period;
        let mut weighted = 0.0;
        let mut total = 0.0;
        for offset in start..=index {
            let weight = if bars[offset].tick_volume > 0 {
                bars[offset].tick_volume as f64
            } else {
                1.0
            };
            weighted += typical[offset] * weight;
            total += weight;
        }
        if total > 0.0 {
            output[index] = weighted / total;
        }
    }
    output
}

fn cci_series(bars: &[Bar], period: usize) -> Vec<f64> {
    let mut output = vec![f64::NAN; bars.len()];
    if period == 0 || bars.len() < period {
        return output;
    }
    let typical = typical_price_series(bars);
    let means = rolling_mean(&typical, period);
    for index in period - 1..bars.len() {
        let mean = means[index];
        if !mean.is_finite() {
            continue;
        }
        let deviation = typical[index + 1 - period..=index]
            .iter()
            .map(|value| (value - mean).abs())
            .sum::<f64>()
            / period as f64;
        if deviation > 0.0 {
            output[index] = (typical[index] - mean) / (0.015 * deviation);
        }
    }
    output
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

fn atr_percentile_series(
    bars: &[Bar],
    atr_period: usize,
    lookback: usize,
    engine: IndicatorEngine,
) -> Vec<f64> {
    let atr_values = match engine {
        IndicatorEngine::Sqx => quantforge_sqx::atr_series(bars, atr_period),
        IndicatorEngine::Mt5 => atr(bars, atr_period),
    };
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
        let rank = finite.iter().filter(|value| **value <= current).count();
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
    let ready_delay = swing_right.max(base_bars);
    let mut last_zone = f64::NAN;
    for (index, output_value) in output.iter_mut().enumerate() {
        // Pivot is ready once swing-right and the post-pivot base both exist.
        if index >= ready_delay {
            let pivot = index - ready_delay;
            if pivot >= swing_left {
                let is_swing_low = (1..=swing_left)
                    .all(|offset| bars[pivot].low <= bars[pivot - offset].low)
                    && (1..=swing_right).all(|offset| bars[pivot].low < bars[pivot + offset].low);
                let is_swing_high = (1..=swing_left)
                    .all(|offset| bars[pivot].high >= bars[pivot - offset].high)
                    && (1..=swing_right).all(|offset| bars[pivot].high > bars[pivot + offset].high);
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
    // NaN warm-up here, unlike the SQX engine's zero warm-up.
    let mut output = vec![f64::NAN; values.len()];
    quantforge_sqx::rolling_extreme_into(values, period, maximum, &mut output);
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

/// Wilder DMI/ADX buffers matching MT5 `iADX`: `(ADX, +DI, -DI)`.
fn directional_index(bars: &[Bar], period: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let len = bars.len();
    let mut adx = vec![f64::NAN; len];
    let mut plus_di = vec![f64::NAN; len];
    let mut minus_di = vec![f64::NAN; len];
    if period == 0 || len <= period {
        return (adx, plus_di, minus_di);
    }
    let mut trs = vec![0.0; len];
    let mut plus = vec![0.0; len];
    let mut minus = vec![0.0; len];
    for index in 1..len {
        let up = bars[index].high - bars[index - 1].high;
        let down = bars[index - 1].low - bars[index].low;
        plus[index] = if up > down && up > 0.0 { up } else { 0.0 };
        minus[index] = if down > up && down > 0.0 { down } else { 0.0 };
        trs[index] = (bars[index].high - bars[index].low)
            .max((bars[index].high - bars[index - 1].close).abs())
            .max((bars[index].low - bars[index - 1].close).abs());
    }
    let mut smooth_tr: f64 = trs[1..=period].iter().sum();
    let mut smooth_plus: f64 = plus[1..=period].iter().sum();
    let mut smooth_minus: f64 = minus[1..=period].iter().sum();
    let mut dx = vec![f64::NAN; len];
    for index in period..len {
        if index > period {
            smooth_tr = smooth_tr - smooth_tr / period as f64 + trs[index];
            smooth_plus = smooth_plus - smooth_plus / period as f64 + plus[index];
            smooth_minus = smooth_minus - smooth_minus / period as f64 + minus[index];
        }
        if smooth_tr > 0.0 {
            plus_di[index] = 100.0 * smooth_plus / smooth_tr;
            minus_di[index] = 100.0 * smooth_minus / smooth_tr;
            let total = plus_di[index] + minus_di[index];
            if total > 0.0 {
                dx[index] = 100.0 * (plus_di[index] - minus_di[index]).abs() / total;
            }
        }
    }
    let first_adx = period.saturating_mul(2).saturating_sub(1);
    if first_adx < len {
        adx[first_adx] = dx[period..=first_adx].iter().sum::<f64>() / period as f64;
        for index in first_adx + 1..len {
            adx[index] = (adx[index - 1] * (period - 1) as f64 + dx[index]) / period as f64;
        }
    }
    (adx, plus_di, minus_di)
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
        let values = calculate_indicator_series_with_clock(
            &bars,
            &IndicatorExpr::Atr {
                period: 2,
                shift: 0,
            },
            None,
            IndicatorEngine::Mt5,
        );
        assert!(values[0].is_nan());
        assert_eq!(values[1], 2.5);
        assert_eq!(values[2], 2.0);
    }

    #[test]
    fn sqx_atr_uses_expanding_then_fixed_window() {
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
                open: 9.0,
                high: 12.0,
                low: 9.0,
                close: 11.0,
                tick_volume: 0,
                real_volume: 0,
                spread_points: Some(0),
            },
        ];
        let values = calculate_indicator_series(
            &bars,
            &IndicatorExpr::Atr {
                period: 14,
                shift: 0,
            },
        );
        assert_eq!(values[0], 2.0);
        assert!((values[1] - 2.5).abs() < 1e-9);
    }

    #[test]
    fn directional_index_exposes_finite_adx_and_directional_buffers_after_warmup() {
        let bars: Vec<_> = (0..80)
            .map(|index| {
                let base = 100.0 + index as f64 * 0.2;
                Bar {
                    timestamp_ms: index as i64 * 60_000,
                    open: base,
                    high: base + 1.0,
                    low: base - 0.25,
                    close: base + 0.75,
                    tick_volume: 1,
                    real_volume: 0,
                    spread_points: Some(0),
                }
            })
            .collect();
        let period = 14;
        let adx = calculate_indicator_series(&bars, &IndicatorExpr::Adx { period, shift: 0 });
        let plus = calculate_indicator_series(&bars, &IndicatorExpr::PlusDi { period, shift: 0 });
        let minus = calculate_indicator_series(&bars, &IndicatorExpr::MinusDi { period, shift: 0 });
        let index = 2 * period as usize;
        assert!(adx[index].is_finite());
        assert!(plus[index].is_finite());
        assert!(minus[index].is_finite());
        assert!(plus[index] > minus[index]);
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
        let body = calculate_indicator_series(&bars, &IndicatorExpr::BodyRangeRatio { shift: 1 });
        let location =
            calculate_indicator_series(&bars, &IndicatorExpr::CloseLocationInBar { shift: 1 });
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
        let indicator = IndicatorExpr::LiquiditySweepScore {
            period: 2,
            shift: 1,
        };
        let legacy = calculate_indicator_series_with_clock(
            &bars,
            &indicator,
            None,
            IndicatorEngine::Mt5,
        );
        assert_eq!(legacy[0], 0.0);
        assert_eq!(legacy[1], 0.0);
        assert_eq!(legacy[2], 1.0);

        let sqx = calculate_indicator_series_with_clock(
            &bars,
            &indicator,
            None,
            IndicatorEngine::Sqx,
        );
        assert!(sqx.iter().all(|value| *value == 0.0 || *value == 1.0 || *value == -1.0));
    }

    fn trending_bars(count: usize) -> Vec<Bar> {
        (0..count)
            .map(|index| {
                let base = 100.0 + index as f64 * 0.3 + ((index % 7) as f64) * 0.1;
                Bar {
                    timestamp_ms: index as i64 * 3_600_000,
                    open: base,
                    high: base + 0.6,
                    low: base - 0.4,
                    close: base + 0.2,
                    tick_volume: 10 + (index % 5) as u64,
                    real_volume: 0,
                    spread_points: Some(0),
                }
            })
            .collect()
    }

    #[test]
    fn macd_histogram_is_main_minus_signal() {
        let bars = trending_bars(200);
        let main = calculate_indicator_series(
            &bars,
            &IndicatorExpr::MacdMain {
                source: PriceField::Close,
                fast_period: 12,
                slow_period: 26,
                shift: 1,
            },
        );
        let signal = calculate_indicator_series(
            &bars,
            &IndicatorExpr::MacdSignal {
                source: PriceField::Close,
                fast_period: 12,
                slow_period: 26,
                signal_period: 9,
                shift: 1,
            },
        );
        let histogram = calculate_indicator_series(
            &bars,
            &IndicatorExpr::MacdHistogram {
                source: PriceField::Close,
                fast_period: 12,
                slow_period: 26,
                signal_period: 9,
                shift: 1,
            },
        );
        let index = bars.len() - 1;
        assert!(main[index].is_finite());
        assert!(signal[index].is_finite());
        assert!((histogram[index] - (main[index] - signal[index])).abs() < 1e-12);
    }

    #[test]
    fn bollinger_bands_straddle_the_middle_band() {
        let bars = trending_bars(120);
        let period = 20;
        let mid = calculate_indicator_series(
            &bars,
            &IndicatorExpr::BollingerMid {
                source: PriceField::Close,
                period,
                shift: 1,
            },
        );
        let upper = calculate_indicator_series(
            &bars,
            &IndicatorExpr::BollingerUpper {
                source: PriceField::Close,
                period,
                deviation_tenths: 20,
                shift: 1,
            },
        );
        let lower = calculate_indicator_series(
            &bars,
            &IndicatorExpr::BollingerLower {
                source: PriceField::Close,
                period,
                deviation_tenths: 20,
                shift: 1,
            },
        );
        let bandwidth = calculate_indicator_series(
            &bars,
            &IndicatorExpr::BollingerBandwidth {
                source: PriceField::Close,
                period,
                deviation_tenths: 20,
                shift: 1,
            },
        );
        let index = bars.len() - 1;
        assert!(upper[index] > mid[index] && mid[index] > lower[index]);
        assert!(((upper[index] + lower[index]) / 2.0 - mid[index]).abs() < 1e-9);
        let expected = (upper[index] - lower[index]) / mid[index] * 100.0;
        assert!((bandwidth[index] - expected).abs() < 1e-9);
    }

    #[test]
    fn ichimoku_spans_are_displaced_by_the_base_period() {
        let bars = trending_bars(200);
        let tenkan_period = 9;
        let kijun_period = 26;
        let tenkan = calculate_indicator_series(
            &bars,
            &IndicatorExpr::IchimokuTenkan {
                period: tenkan_period,
                shift: 1,
            },
        );
        let kijun = calculate_indicator_series(
            &bars,
            &IndicatorExpr::IchimokuKijun {
                period: kijun_period,
                shift: 1,
            },
        );
        let senkou_a = calculate_indicator_series(
            &bars,
            &IndicatorExpr::IchimokuSenkouA {
                tenkan_period,
                kijun_period,
                shift: 1,
            },
        );
        let index = bars.len() - 1;
        let source = index - kijun_period as usize;
        let expected = (tenkan[source] + kijun[source]) / 2.0;
        assert!((senkou_a[index] - expected).abs() < 1e-12);
        // Tenkan tracks a shorter window, so it leads Kijun in an uptrend.
        assert!(tenkan[index] > kijun[index]);
    }

    #[test]
    fn vwap_collapses_to_typical_price_mean_with_flat_volume() {
        let mut bars = trending_bars(60);
        for bar in &mut bars {
            bar.tick_volume = 5;
        }
        let period = 10;
        let vwap = calculate_indicator_series(&bars, &IndicatorExpr::Vwap { period, shift: 1 });
        let index = bars.len() - 1;
        let expected = bars[index + 1 - period as usize..=index]
            .iter()
            .map(|bar| (bar.high + bar.low + bar.close) / 3.0)
            .sum::<f64>()
            / period as f64;
        assert!((vwap[index] - expected).abs() < 1e-9);
    }

    #[test]
    fn cci_is_positive_while_price_trends_up() {
        let bars = trending_bars(120);
        let cci = calculate_indicator_series(&bars, &IndicatorExpr::Cci { period: 20, shift: 1 });
        let index = bars.len() - 1;
        assert!(cci[index].is_finite());
        assert!(cci[index] > 0.0);
    }

    #[test]
    fn qqe_trail_sits_below_the_line_in_an_uptrend() {
        let bars = trending_bars(400);
        let line = calculate_indicator_series(
            &bars,
            &IndicatorExpr::QqeLine {
                rsi_period: 14,
                smoothing_period: 5,
                shift: 1,
            },
        );
        let trail = calculate_indicator_series(
            &bars,
            &IndicatorExpr::QqeTrail {
                rsi_period: 14,
                smoothing_period: 5,
                factor_tenths: 42,
                shift: 1,
            },
        );
        let index = bars.len() - 1;
        assert!(line[index].is_finite());
        assert!(trail[index].is_finite());
        assert!(trail[index] < line[index]);
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
            IndicatorEngine::Sqx,
        );
        assert!(highs[0].is_nan());
        assert_eq!(highs[1], 3.0);
        assert_eq!(highs[2], 3.0);
    }
}
