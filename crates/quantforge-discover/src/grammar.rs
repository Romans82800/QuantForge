use crate::FROZEN_ATR_PERIOD;
use crate::model::{
    FamilyStyle, SearchFamily, SearchRange, SearchRangeProfile, UniversalGrammarConfig,
};
use quantforge_core::{FloatPolicy, STRATEGY_IR_VERSION};
use quantforge_ir::{
    BoolExpr, ComparisonOp, ContextValue, EntryDistancePolicy, EntryOrderPolicy, EntrySignals,
    IndicatorExpr, ManagePolicy, NumericExpr, PartialExit, PriceField, ProtectiveStops, RiskPolicy,
    Side, StopLossPolicy, StrategyIr, StrategyMeta, TakeProfitPolicy, TrailingPolicy,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// Institutional indicator-period ladder (Search Families).
const PERIODS: [u16; 3] = [10, 14, 20];
/// ATR / R-multiple ladder in 0.25 steps (cross-symbol comparable).
/// Floor 1.5× matches SQX Build MinSLATRMultiple for Selected-TF-safe stops.
const ATR_STOP_MULTIPLIERS: [f64; 11] =
    [1.5, 1.75, 2.0, 2.25, 2.5, 2.75, 3.0, 3.25, 3.5, 3.75, 4.0];
/// TP floor 2.0× — SQX USDJPY builds used MinPT ≥ 60 pips / ≥2 ATR.
const ATR_TP_MULTIPLIERS: [f64; 9] = [2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0];
const ATR_ENTRY_MULTIPLIERS: [f64; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
const RISK_MULTIPLES: [f64; 9] = [1.5, 1.75, 2.0, 2.25, 2.5, 3.0, 3.5, 4.0, 4.5];
const ATR_TRAIL_MULTIPLIERS: [f64; 9] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 2.25, 2.5];
const R_ACTIVATE: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
const R_DISTANCE: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
/// Ladders for MACD, Bollinger, Ichimoku, QQE, VWAP and CCI. These stay close
/// to the conventional settings so the search explores sane neighbourhoods
/// instead of arbitrary parameter soup.
const MACD_FAST: [u16; 3] = [8, 12, 16];
const MACD_SLOW: [u16; 3] = [21, 26, 34];
const MACD_SIGNAL: [u16; 3] = [7, 9, 12];
/// Bollinger deviations in tenths (15 = 1.5σ).
const BB_DEVIATION_TENTHS: [u16; 4] = [15, 20, 25, 30];
const ICHIMOKU_TENKAN: [u16; 3] = [7, 9, 12];
const ICHIMOKU_KIJUN: [u16; 3] = [22, 26, 30];
const ICHIMOKU_SENKOU: [u16; 3] = [44, 52, 60];
const QQE_SMOOTHING: [u16; 3] = [3, 5, 8];
/// QQE Wilder factor in tenths (42 = 4.2).
const QQE_FACTOR_TENTHS: [u16; 3] = [27, 42, 61];
const VWAP_PERIODS: [u16; 3] = [20, 50, 100];
const CCI_LEVELS: [f64; 3] = [80.0, 100.0, 120.0];

pub fn generate_seed(seed: u64, sequence: u64) -> StrategyIr {
    let family = SearchFamily::ALL[(sequence as usize) % SearchFamily::ALL.len()];
    generate_seed_for_family(seed, sequence, family)
}

pub fn generate_seed_for_family(seed: u64, sequence: u64, family: SearchFamily) -> StrategyIr {
    let mut rng = rng_for(seed, 0, sequence);
    build_seed(
        family,
        &mut rng,
        format!("seed-{sequence}"),
        family.spec().max_atoms,
        true,
        false,
        &UniversalGrammarConfig::default(),
    )
}

pub fn mutate_strategy(
    strategy: &StrategyIr,
    seed: u64,
    sequence: u64,
    structural_probability: f64,
) -> StrategyIr {
    let mut rng = rng_for(seed, 1, sequence);
    mutate_with_rng(
        strategy,
        &mut rng,
        structural_probability,
        sequence,
        false,
        SearchFamily::from_style(classify_family(strategy)),
        &UniversalGrammarConfig::default(),
    )
}

pub(crate) fn rng_for(seed: u64, stream: u64, sequence: u64) -> ChaCha8Rng {
    let mixed = splitmix64(
        seed ^ stream.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ sequence.wrapping_mul(0xbf58_476d_1ce4_e5b9),
    );
    ChaCha8Rng::seed_from_u64(mixed)
}

/// Resample every numeric gene that has an explicit researcher-controlled
/// range. Called after seed/mutation construction, before evaluation.
pub(crate) fn apply_search_ranges(
    strategy: &mut StrategyIr,
    rng: &mut ChaCha8Rng,
    ranges: &SearchRangeProfile,
) {
    apply_expression_ranges(strategy.entry.long.as_mut(), rng, ranges);
    apply_expression_ranges(strategy.entry.short.as_mut(), rng, ranges);
    apply_expression_ranges(strategy.exit.as_mut(), rng, ranges);
    apply_expression_ranges(strategy.exit_long.as_mut(), rng, ranges);
    apply_expression_ranges(strategy.exit_short.as_mut(), rng, ranges);
    for filter in &mut strategy.filters {
        apply_expression_ranges(Some(filter), rng, ranges);
    }
    let atr_period = sample_u16(rng, &ranges.atr_period);
    strategy.stops.stop_loss = StopLossPolicy::AtrMultiple {
        period: atr_period,
        multiplier: sample_range(rng, &ranges.atr_stop_multiple),
    };
    strategy.stops.take_profit = if rng.gen_bool(0.7) {
        TakeProfitPolicy::RiskMultiple {
            multiple: sample_range(rng, &ranges.risk_target_multiple),
        }
    } else {
        TakeProfitPolicy::AtrMultiple {
            period: atr_period,
            multiplier: sample_range(rng, &ranges.atr_target_multiple),
        }
    };
    match &mut strategy.entry.order {
        EntryOrderPolicy::Market => {}
        EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        }
        | EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        } => {
            *distance = EntryDistancePolicy::AtrMultiple {
                period: atr_period,
                multiplier: sample_range(rng, &ranges.pending_distance_atr),
            };
            *expiry_bars = sample_u16(rng, &ranges.pending_expiry_bars);
        }
    }
    if strategy.manage.time_stop_bars.is_some() {
        strategy.manage.time_stop_bars = Some(sample_u16(rng, &ranges.time_stop_bars));
    }
    if let Some(TrailingPolicy::AtrMultiple {
        period, multiplier, ..
    }) = &mut strategy.manage.trailing
    {
        *period = atr_period;
        *multiplier = sample_range(rng, &ranges.atr_stop_multiple);
    }
}

fn apply_expression_ranges(
    expression: Option<&mut BoolExpr>,
    rng: &mut ChaCha8Rng,
    ranges: &SearchRangeProfile,
) {
    let Some(expression) = expression else { return };
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            apply_numeric_ranges(left, right, rng, ranges);
            apply_numeric_ranges(right, left, rng, ranges);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            apply_numeric_ranges(value, lower, rng, ranges);
            apply_numeric_ranges(value, upper, rng, ranges);
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                apply_expression_ranges(Some(child), rng, ranges);
            }
        }
        BoolExpr::Not { child } => apply_expression_ranges(Some(child), rng, ranges),
    }
}

fn apply_numeric_ranges(
    indicator: &mut NumericExpr,
    other: &mut NumericExpr,
    rng: &mut ChaCha8Rng,
    ranges: &SearchRangeProfile,
) {
    let NumericExpr::Indicator { value } = indicator else {
        return;
    };
    let constant = match other {
        NumericExpr::Constant { value } => Some(value),
        _ => None,
    };
    match value {
        IndicatorExpr::Sma { period, .. }
        | IndicatorExpr::Ema { period, .. }
        | IndicatorExpr::Wma { period, .. }
        | IndicatorExpr::DonchianHigh { period, .. }
        | IndicatorExpr::DonchianLow { period, .. }
        | IndicatorExpr::Highest { period, .. }
        | IndicatorExpr::Lowest { period, .. }
        | IndicatorExpr::StandardDeviation { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period)
        }
        IndicatorExpr::LiquiditySweepScore { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                let magnitude = sample_range(rng, &ranges.liquidity_sweep_threshold);
                *value = if *value < 0.0 { -magnitude } else { magnitude };
            }
        }
        IndicatorExpr::Rsi { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                *value = if *value >= 50.0 {
                    sample_range(rng, &ranges.rsi_upper)
                } else {
                    sample_range(rng, &ranges.rsi_lower)
                };
            }
        }
        IndicatorExpr::Adx { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                *value = sample_range(rng, &ranges.adx_threshold);
            }
        }
        IndicatorExpr::PlusDi { period, .. } | IndicatorExpr::MinusDi { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
        }
        IndicatorExpr::RateOfChange { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                let magnitude = sample_range(rng, &ranges.roc_threshold);
                *value = if *value < 0.0 { -magnitude } else { magnitude };
            }
        }
        IndicatorExpr::ZScore { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                let magnitude = sample_range(rng, &ranges.zscore_threshold);
                *value = if *value < 0.0 { -magnitude } else { magnitude };
            }
        }
        IndicatorExpr::PercentileInRange { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                let low = sample_range(rng, &ranges.percentile_low);
                *value = if *value > 50.0 { 100.0 - low } else { low };
            }
        }
        IndicatorExpr::Atr { period, .. } => *period = sample_u16(rng, &ranges.atr_period),
        IndicatorExpr::AtrPercentile {
            atr_period,
            lookback,
            ..
        } => {
            *atr_period = sample_u16(rng, &ranges.atr_period);
            *lookback = sample_u16(rng, &ranges.atr_percentile_lookback);
            if let Some(value) = constant {
                *value = sample_range(rng, &ranges.atr_percentile_max);
            }
        }
        IndicatorExpr::MacdMain {
            fast_period,
            slow_period,
            ..
        } => {
            *fast_period = MACD_FAST[rng.gen_range(0..MACD_FAST.len())];
            *slow_period = MACD_SLOW[rng.gen_range(0..MACD_SLOW.len())].max(*fast_period + 1);
        }
        IndicatorExpr::MacdSignal {
            fast_period,
            slow_period,
            signal_period,
            ..
        }
        | IndicatorExpr::MacdHistogram {
            fast_period,
            slow_period,
            signal_period,
            ..
        } => {
            *fast_period = MACD_FAST[rng.gen_range(0..MACD_FAST.len())];
            *slow_period = MACD_SLOW[rng.gen_range(0..MACD_SLOW.len())].max(*fast_period + 1);
            *signal_period = MACD_SIGNAL[rng.gen_range(0..MACD_SIGNAL.len())];
        }
        IndicatorExpr::BollingerMid { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period).max(10);
        }
        IndicatorExpr::BollingerUpper {
            period,
            deviation_tenths,
            ..
        }
        | IndicatorExpr::BollingerLower {
            period,
            deviation_tenths,
            ..
        } => {
            *period = sample_u16(rng, &ranges.indicator_period).max(10);
            *deviation_tenths = BB_DEVIATION_TENTHS[rng.gen_range(0..BB_DEVIATION_TENTHS.len())];
        }
        IndicatorExpr::BollingerBandwidth {
            period,
            deviation_tenths,
            ..
        } => {
            *period = sample_u16(rng, &ranges.indicator_period).max(10);
            *deviation_tenths = BB_DEVIATION_TENTHS[rng.gen_range(0..BB_DEVIATION_TENTHS.len())];
            if let Some(value) = constant {
                *value = rng.gen_range(1.0..=3.0);
            }
        }
        IndicatorExpr::IchimokuTenkan { period, .. } => {
            *period = ICHIMOKU_TENKAN[rng.gen_range(0..ICHIMOKU_TENKAN.len())];
        }
        IndicatorExpr::IchimokuKijun { period, .. } => {
            *period = ICHIMOKU_KIJUN[rng.gen_range(0..ICHIMOKU_KIJUN.len())];
        }
        IndicatorExpr::IchimokuSenkouA {
            tenkan_period,
            kijun_period,
            ..
        } => {
            *tenkan_period = ICHIMOKU_TENKAN[rng.gen_range(0..ICHIMOKU_TENKAN.len())];
            *kijun_period = ICHIMOKU_KIJUN[rng.gen_range(0..ICHIMOKU_KIJUN.len())];
        }
        IndicatorExpr::IchimokuSenkouB {
            period,
            kijun_period,
            ..
        } => {
            *period = ICHIMOKU_SENKOU[rng.gen_range(0..ICHIMOKU_SENKOU.len())];
            *kijun_period = ICHIMOKU_KIJUN[rng.gen_range(0..ICHIMOKU_KIJUN.len())];
        }
        IndicatorExpr::QqeLine {
            rsi_period,
            smoothing_period,
            ..
        } => {
            *rsi_period = sample_u16(rng, &ranges.indicator_period);
            *smoothing_period = QQE_SMOOTHING[rng.gen_range(0..QQE_SMOOTHING.len())];
            if let Some(value) = constant {
                *value = rng.gen_range(45.0..=55.0);
            }
        }
        IndicatorExpr::QqeTrail {
            rsi_period,
            smoothing_period,
            factor_tenths,
            ..
        } => {
            *rsi_period = sample_u16(rng, &ranges.indicator_period);
            *smoothing_period = QQE_SMOOTHING[rng.gen_range(0..QQE_SMOOTHING.len())];
            *factor_tenths = QQE_FACTOR_TENTHS[rng.gen_range(0..QQE_FACTOR_TENTHS.len())];
        }
        IndicatorExpr::Vwap { period, .. } => {
            *period = VWAP_PERIODS[rng.gen_range(0..VWAP_PERIODS.len())];
        }
        IndicatorExpr::Cci { period, .. } => {
            *period = sample_u16(rng, &ranges.indicator_period);
            if let Some(value) = constant {
                let magnitude = CCI_LEVELS[rng.gen_range(0..CCI_LEVELS.len())];
                *value = if *value < 0.0 {
                    -magnitude
                } else if *value == 0.0 {
                    0.0
                } else {
                    magnitude
                };
            }
        }
        IndicatorExpr::BodyRangeRatio { .. } => {
            if let Some(value) = constant {
                *value = sample_range(rng, &ranges.impulse_body_ratio);
            }
        }
        IndicatorExpr::CloseLocationInBar { .. } => {
            if let Some(value) = constant {
                let high = sample_range(rng, &ranges.impulse_close_location);
                *value = if *value < 0.5 { 1.0 - high } else { high };
            }
        }
        IndicatorExpr::SessionRangeHigh {
            start_hour,
            range_bars,
            ..
        }
        | IndicatorExpr::SessionRangeLow {
            start_hour,
            range_bars,
            ..
        } => {
            *start_hour = sample_u8(rng, &ranges.session_start_hour);
            *range_bars = sample_u16(rng, &ranges.session_range_bars);
        }
        IndicatorExpr::SwingBaseZoneHigh {
            swing_left,
            swing_right,
            base_bars,
            ..
        }
        | IndicatorExpr::SwingBaseZoneLow {
            swing_left,
            swing_right,
            base_bars,
            ..
        } => {
            let swing = sample_u16(rng, &ranges.swing_bars);
            *swing_left = swing;
            *swing_right = swing;
            *base_bars = sample_u16(rng, &ranges.base_bars);
        }
    }
}

fn sample_range(rng: &mut ChaCha8Rng, range: &SearchRange) -> f64 {
    let count = ((range.maximum - range.minimum) / range.step)
        .floor()
        .max(0.0) as u32;
    let index = rng.gen_range(0..=count) as f64;
    (range.minimum + index * range.step).min(range.maximum)
}

fn sample_u16(rng: &mut ChaCha8Rng, range: &SearchRange) -> u16 {
    sample_range(rng, range).round().clamp(1.0, u16::MAX as f64) as u16
}
fn sample_u8(rng: &mut ChaCha8Rng, range: &SearchRange) -> u8 {
    sample_range(rng, range).round().clamp(0.0, u8::MAX as f64) as u8
}

pub(crate) fn build_seed(
    family: SearchFamily,
    rng: &mut ChaCha8Rng,
    id: String,
    max_atoms: usize,
    institutional: bool,
    market_entries_only: bool,
    universal: &UniversalGrammarConfig,
) -> StrategyIr {
    // Institutional Search Families always mirror long/short on one condition set.
    let side = if institutional {
        Side::Both
    } else {
        match rng.gen_range(0..4) {
            0 => Side::LongOnly,
            1 => Side::ShortOnly,
            _ => Side::Both,
        }
    };
    let (long, short) = if family == SearchFamily::Universal {
        universal_entries(rng, universal)
    } else {
        family_entries(family, rng, max_atoms.max(1))
    };
    let (exit_long, exit_short) = if family == SearchFamily::Universal {
        let (long, short) = universal_exits(rng, universal);
        (Some(long), Some(short))
    } else {
        (None, None)
    };
    // When production forces market (simple_exits), do not sample pending
    // distances that collapse away — that only burns diversity into clones.
    let order = if institutional {
        if market_entries_only {
            EntryOrderPolicy::Market
        } else {
            random_pending_entry_order(rng)
        }
    } else {
        random_entry_order(rng)
    };
    let entry = match side {
        Side::LongOnly => EntrySignals {
            long: Some(long),
            short: None,
            order: order.clone(),
        },
        Side::ShortOnly => EntrySignals {
            long: None,
            short: Some(short),
            order: order.clone(),
        },
        Side::Both => EntrySignals {
            long: Some(long),
            short: Some(short),
            order,
        },
    };

    let mut strategy = StrategyIr {
        id,
        version: STRATEGY_IR_VERSION,
        entry,
        exit: None,
        exit_long,
        exit_short,
        filters: if rng.gen_bool(0.35) {
            vec![session_filter(rng)]
        } else {
            Vec::new()
        },
        side,
        risk: RiskPolicy::FixedCurrency {
            amount: crate::FIXED_RISK_PER_TRADE,
        },
        stops: random_stops(rng),
        manage: if institutional {
            ManagePolicy {
                time_stop_bars: Some(rng.gen_range(4..=16)),
                break_even_at_r: None,
                trailing: None,
                partial_exits: Vec::new(),
                flatten_end_of_day: false,
                max_one_entry_per_day: true,
                ..Default::default()
            }
        } else {
            random_manage(rng)
        },
        meta: StrategyMeta {
            thesis_hint: family_name(family).into(),
            complexity: 0,
            export_safe: true,
        },
    };
    normalize(&mut strategy);
    freeze_atr_period(&mut strategy);
    strategy
}

pub(crate) fn crossover(left: &StrategyIr, right: &StrategyIr, rng: &mut ChaCha8Rng) -> StrategyIr {
    let same_family = classify_family(left) == classify_family(right);
    let mut child = if rng.gen_bool(0.5) {
        left.clone()
    } else {
        right.clone()
    };

    if same_family {
        if rng.gen_bool(0.5) {
            child.entry = right.entry.clone();
            child.side = right.side;
        }
        if rng.gen_bool(0.5) {
            child.filters = right.filters.clone();
        }
        if rng.gen_bool(0.5) {
            child.exit = right.exit.clone();
            child.exit_long = right.exit_long.clone();
            child.exit_short = right.exit_short.clone();
        }
    }
    if rng.gen_bool(0.5) {
        child.stops = right.stops.clone();
    }
    if rng.gen_bool(0.5) {
        child.entry.order = right.entry.order.clone();
    }
    if rng.gen_bool(0.5) {
        child.manage = right.manage.clone();
    }
    normalize(&mut child);
    child
}

pub(crate) fn mutate_with_rng(
    strategy: &StrategyIr,
    rng: &mut ChaCha8Rng,
    structural_probability: f64,
    sequence: u64,
    allow_cross_family: bool,
    locked_family: SearchFamily,
    universal: &UniversalGrammarConfig,
) -> StrategyIr {
    let mut child = strategy.clone();
    child.id = format!("candidate-{sequence}");
    let max_atoms = locked_family.spec().max_atoms.max(1);

    if let Some(entry) = &mut child.entry.long {
        mutate_bool(entry, rng);
    }
    if let Some(entry) = &mut child.entry.short {
        mutate_bool(entry, rng);
    }
    if let Some(exit) = &mut child.exit {
        mutate_bool(exit, rng);
    }
    if let Some(exit) = &mut child.exit_long {
        mutate_bool(exit, rng);
    }
    if let Some(exit) = &mut child.exit_short {
        mutate_bool(exit, rng);
    }
    for filter in &mut child.filters {
        mutate_bool(filter, rng);
    }
    mutate_policies(&mut child, rng);

    if rng.gen_bool(structural_probability) {
        match rng.gen_range(0..6) {
            0 => {
                let family = if allow_cross_family {
                    random_family(rng)
                } else {
                    locked_family
                };
                let (long, short) = if family == SearchFamily::Universal {
                    universal_entries(rng, universal)
                } else {
                    family_entries(family, rng, max_atoms)
                };
                child.meta.thesis_hint = family_name(family).into();
                child.side = Side::Both;
                child.entry = EntrySignals {
                    long: Some(long),
                    short: Some(short),
                    order: random_pending_entry_order(rng),
                };
                if family == SearchFamily::Universal {
                    let (long_exit, short_exit) = universal_exits(rng, universal);
                    child.exit = None;
                    child.exit_long = Some(long_exit);
                    child.exit_short = Some(short_exit);
                } else {
                    child.exit_long = None;
                    child.exit_short = None;
                }
            }
            1 => {
                if child.filters.is_empty() {
                    child.filters.push(session_filter(rng));
                } else {
                    child.filters.clear();
                }
            }
            2 => child.stops = random_stops(rng),
            3 => {
                let family = locked_family;
                let (long, short) = if family == SearchFamily::Universal {
                    universal_entries(rng, universal)
                } else {
                    family_entries(family, rng, max_atoms)
                };
                child.side = Side::Both;
                child.entry = EntrySignals {
                    long: Some(long),
                    short: Some(short),
                    order: random_pending_entry_order(rng),
                };
                if family == SearchFamily::Universal {
                    let (long_exit, short_exit) = universal_exits(rng, universal);
                    child.exit = None;
                    child.exit_long = Some(long_exit);
                    child.exit_short = Some(short_exit);
                }
            }
            4 => child.entry.order = random_pending_entry_order(rng),
            _ => {
                child.manage.trailing = None;
                child.manage.break_even_at_r = None;
                child.manage.partial_exits.clear();
                if let Some(bars) = &mut child.manage.time_stop_bars {
                    *bars = (*bars as i32 + rng.gen_range(-2..=2)).clamp(4, 16) as u16;
                } else {
                    child.manage.time_stop_bars = Some(rng.gen_range(4..=16));
                }
            }
        }
    }
    normalize(&mut child);
    freeze_atr_period(&mut child);
    child
}

/// Number of top-level mirrored entry condition blocks. This is the axis the
/// methodology sweeps (2, 3 or 4 conditions), so the archive niches on it.
pub fn entry_condition_count(strategy: &StrategyIr) -> usize {
    strategy
        .entry
        .long
        .as_ref()
        .or(strategy.entry.short.as_ref())
        .map(|expression| match expression {
            BoolExpr::And { children } => children.len(),
            _ => 1,
        })
        .unwrap_or(0)
}

/// Number of top-level exit condition blocks, which the generator combines with
/// OR so any one of them can close the trade.
pub fn exit_condition_count(strategy: &StrategyIr) -> usize {
    strategy
        .exit_long
        .as_ref()
        .or(strategy.exit_short.as_ref())
        .or(strategy.exit.as_ref())
        .map(|expression| match expression {
            BoolExpr::Or { children } => children.len(),
            _ => 1,
        })
        .unwrap_or(0)
}

pub(crate) fn classify_family(strategy: &StrategyIr) -> FamilyStyle {
    match strategy.meta.thesis_hint.as_str() {
        "trend" | "trend_pullback" => FamilyStyle::TrendPullback,
        "momentum" | "momentum_burst" => FamilyStyle::MomentumBurst,
        "breakout" | "donchian_breakout" => FamilyStyle::DonchianBreakout,
        "mean_reversion" | "mean_reversion_band" => FamilyStyle::MeanReversionBand,
        "zscore_reversion" | "z_score_reversion" => FamilyStyle::ZScoreReversion,
        "session_orb" => FamilyStyle::SessionOrb,
        "impulse_candle" => FamilyStyle::ImpulseCandle,
        "vol_squeeze_break" => FamilyStyle::VolSqueezeBreak,
        "supply_demand_reclaim" => FamilyStyle::SupplyDemandReclaim,
        "sweep_reclaim" => FamilyStyle::SweepReclaim,
        "universal" | "universal_grammar" => FamilyStyle::Universal,
        _ => strategy
            .entry
            .long
            .as_ref()
            .or(strategy.entry.short.as_ref())
            .map(classify_expression)
            .unwrap_or(FamilyStyle::TrendPullback),
    }
}

fn family_entries(
    family: SearchFamily,
    rng: &mut ChaCha8Rng,
    max_atoms: usize,
) -> (BoolExpr, BoolExpr) {
    let atoms = family_entry_atoms(family, rng);
    let upper = atoms.len().min(max_atoms).max(1);
    let count = rng.gen_range(1..=upper);
    select_entry_atoms(&atoms, rng, count)
}

fn universal_entries(
    rng: &mut ChaCha8Rng,
    config: &UniversalGrammarConfig,
) -> (BoolExpr, BoolExpr) {
    let atoms = universal_entry_atoms(rng, config);
    let count = rng.gen_range(
        config.minimum_entry_conditions..=config.maximum_entry_conditions.min(atoms.len()),
    );
    select_entry_atoms(&atoms, rng, count)
}

fn universal_exits(rng: &mut ChaCha8Rng, config: &UniversalGrammarConfig) -> (BoolExpr, BoolExpr) {
    let atoms = universal_exit_atoms(rng, config);
    let count = rng.gen_range(
        config.minimum_exit_conditions..=config.maximum_exit_conditions.min(atoms.len()),
    );
    let mut order: Vec<usize> = (0..atoms.len()).collect();
    for index in 0..count {
        let swap_with = rng.gen_range(index..order.len());
        order.swap(index, swap_with);
    }
    let longs = order
        .iter()
        .take(count)
        .map(|&index| atoms[index].0.clone())
        .collect();
    let shorts = order
        .iter()
        .take(count)
        .map(|&index| atoms[index].1.clone())
        .collect();
    (or_all(longs), or_all(shorts))
}

fn universal_entry_atoms(
    rng: &mut ChaCha8Rng,
    config: &UniversalGrammarConfig,
) -> Vec<(BoolExpr, BoolExpr)> {
    const COMPONENT_FAMILIES: [SearchFamily; 10] = [
        SearchFamily::TrendPullback,
        SearchFamily::MomentumBurst,
        SearchFamily::DonchianBreakout,
        SearchFamily::MeanReversionBand,
        SearchFamily::ZScoreReversion,
        SearchFamily::SessionOrb,
        SearchFamily::ImpulseCandle,
        SearchFamily::VolSqueezeBreak,
        SearchFamily::SupplyDemandReclaim,
        SearchFamily::SweepReclaim,
    ];
    let mut atoms = Vec::new();
    for family in COMPONENT_FAMILIES {
        for (mut long, mut short) in family_entry_atoms(family, rng) {
            let base_shift = rng.gen_range(config.minimum_shift..=config.maximum_shift);
            rebase_bool_shifts(&mut long, base_shift, config.maximum_shift);
            rebase_bool_shifts(&mut short, base_shift, config.maximum_shift);
            atoms.push((long, short));
        }
    }
    for (mut long, mut short) in extended_entry_atoms(rng) {
        let base_shift = rng.gen_range(config.minimum_shift..=config.maximum_shift);
        rebase_bool_shifts(&mut long, base_shift, config.maximum_shift);
        rebase_bool_shifts(&mut short, base_shift, config.maximum_shift);
        atoms.push((long, short));
    }
    atoms
}

fn universal_exit_atoms(
    rng: &mut ChaCha8Rng,
    config: &UniversalGrammarConfig,
) -> Vec<(BoolExpr, BoolExpr)> {
    let shift = || 1_u16;
    let fast = PERIODS[rng.gen_range(0..PERIODS.len() - 1)];
    let slow = PERIODS[rng.gen_range(1..PERIODS.len())].max(fast + 1);
    let period = choose_period(rng);
    let mut atoms = vec![
        (
            BoolExpr::CrossBelow {
                left: close(shift()),
                right: ema(period, shift()),
            },
            BoolExpr::CrossAbove {
                left: close(shift()),
                right: ema(period, shift()),
            },
        ),
        (
            compare(
                ComparisonOp::LessThan,
                rsi(period, shift()),
                NumericExpr::Constant { value: 45.0 },
            ),
            compare(
                ComparisonOp::GreaterThan,
                rsi(period, shift()),
                NumericExpr::Constant { value: 55.0 },
            ),
        ),
        (
            compare(
                ComparisonOp::LessThan,
                roc(period, shift()),
                NumericExpr::Constant { value: 0.0 },
            ),
            compare(
                ComparisonOp::GreaterThan,
                roc(period, shift()),
                NumericExpr::Constant { value: 0.0 },
            ),
        ),
        (
            compare(
                ComparisonOp::GreaterThan,
                minus_di(period, shift()),
                plus_di(period, shift()),
            ),
            compare(
                ComparisonOp::GreaterThan,
                plus_di(period, shift()),
                minus_di(period, shift()),
            ),
        ),
        (
            compare(
                ComparisonOp::LessThan,
                zscore(period, shift()),
                NumericExpr::Constant { value: 0.0 },
            ),
            compare(
                ComparisonOp::GreaterThan,
                zscore(period, shift()),
                NumericExpr::Constant { value: 0.0 },
            ),
        ),
        (
            BoolExpr::CrossBelow {
                left: ema(fast, shift()),
                right: ema(slow, shift()),
            },
            BoolExpr::CrossAbove {
                left: ema(fast, shift()),
                right: ema(slow, shift()),
            },
        ),
    ];
    atoms.extend(extended_exit_atoms(rng));
    for (long, short) in &mut atoms {
        let base_shift = rng.gen_range(config.minimum_shift..=config.maximum_shift);
        rebase_bool_shifts(long, base_shift, config.maximum_shift);
        rebase_bool_shifts(short, base_shift, config.maximum_shift);
    }
    atoms
}

/// Build mirrored long/short entries with an exact atom AND-count.
pub(crate) fn family_entries_with_count(
    family: SearchFamily,
    rng: &mut ChaCha8Rng,
    atom_count: usize,
) -> (BoolExpr, BoolExpr) {
    let atoms = family_entry_atoms(family, rng);
    let count = atom_count.min(atoms.len()).max(1);
    select_entry_atoms(&atoms, rng, count)
}

fn select_entry_atoms(
    atoms: &[(BoolExpr, BoolExpr)],
    rng: &mut ChaCha8Rng,
    count: usize,
) -> (BoolExpr, BoolExpr) {
    let count = count.min(atoms.len()).max(1);
    let mut order: Vec<usize> = (0..atoms.len()).collect();
    // Fisher–Yates shuffle prefix for selection without replacement.
    for index in 0..count {
        let swap_with = rng.gen_range(index..order.len());
        order.swap(index, swap_with);
    }
    let mut longs = Vec::with_capacity(count);
    let mut shorts = Vec::with_capacity(count);
    for &atom_index in order.iter().take(count) {
        longs.push(atoms[atom_index].0.clone());
        shorts.push(atoms[atom_index].1.clone());
    }
    (and_all(longs), and_all(shorts))
}

/// Crossover splices two parents, and component atoms can be composite, so a
/// child can exceed the export node budget. Trim top-level condition blocks until
/// it validates: a narrower child still breeds, whereas an invalid one costs a
/// wasted slot and surfaces as an evaluation error.
pub(crate) fn fit_within_ir_limits(
    mut strategy: StrategyIr,
    minimum_entry_conditions: usize,
) -> Option<StrategyIr> {
    let limits = quantforge_ir::IrLimits::default();
    loop {
        if strategy.validate_export_safe(limits).is_ok() {
            return Some(strategy);
        }
        if !trim_one_condition(&mut strategy, minimum_entry_conditions.max(1)) {
            return None;
        }
    }
}

/// Drops one mirrored top-level block, exits before entries. Exits are OR-joined,
/// so losing one only removes an exit reason; entries are AND-joined and must stay
/// above the configured floor.
fn trim_one_condition(strategy: &mut StrategyIr, entry_floor: usize) -> bool {
    if pop_top_level_child(&mut strategy.exit_long, 1) {
        pop_top_level_child(&mut strategy.exit_short, 1);
        return true;
    }
    if pop_top_level_child(&mut strategy.exit, 1) {
        return true;
    }
    if strategy.filters.pop().is_some() {
        return true;
    }
    if pop_top_level_child(&mut strategy.entry.long, entry_floor) {
        pop_top_level_child(&mut strategy.entry.short, entry_floor);
        return true;
    }
    false
}

fn pop_top_level_child(expression: &mut Option<BoolExpr>, floor: usize) -> bool {
    let Some(BoolExpr::And { children } | BoolExpr::Or { children }) = expression.as_mut() else {
        return false;
    };
    if children.len() <= floor.max(1) {
        return false;
    }
    children.pop();
    if children.len() == 1 {
        *expression = Some(children.remove(0));
    }
    true
}

/// Count leaf entry predicates (AND/OR children counted separately).
#[allow(dead_code)]
pub(crate) fn entry_atom_count(expression: &BoolExpr) -> usize {
    match expression {
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            children.iter().map(entry_atom_count).sum()
        }
        _ => 1,
    }
}

/// (long_condition, short_condition) atoms available inside a family.
fn family_entry_atoms(family: SearchFamily, rng: &mut ChaCha8Rng) -> Vec<(BoolExpr, BoolExpr)> {
    match family {
        SearchFamily::TrendPullback => {
            // Slow MA at least one ladder step above the fast period.
            let fast_index = rng.gen_range(0..PERIODS.len().saturating_sub(1));
            let slow_index = rng.gen_range((fast_index + 1)..PERIODS.len());
            let fast_p = PERIODS[fast_index];
            let slow_p = PERIODS[slow_index];
            let ema_fast = ema(fast_p, 1);
            let ema_slow = ema(slow_p, 1);
            let sma_fast = sma(fast_p, 1);
            let sma_slow = sma(slow_p, 1);
            let trend_ema = ema(choose_period(rng), 1);
            let roc_period = choose_period(rng);
            vec![
                // Original: EMA cross
                (
                    BoolExpr::CrossAbove {
                        left: ema_fast.clone(),
                        right: ema_slow.clone(),
                    },
                    BoolExpr::CrossBelow {
                        left: ema_fast,
                        right: ema_slow,
                    },
                ),
                // +1 SMA cross
                (
                    BoolExpr::CrossAbove {
                        left: sma_fast.clone(),
                        right: sma_slow.clone(),
                    },
                    BoolExpr::CrossBelow {
                        left: sma_fast,
                        right: sma_slow,
                    },
                ),
                // +2 Close vs EMA
                (
                    compare(ComparisonOp::GreaterThan, close(1), trend_ema.clone()),
                    compare(ComparisonOp::LessThan, close(1), trend_ema),
                ),
                // +3 Rate of change sign
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        roc(roc_period, 1),
                        NumericExpr::Constant { value: 0.0 },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        roc(roc_period, 1),
                        NumericExpr::Constant { value: 0.0 },
                    ),
                ),
                // +4 ADX regime / directional confirmation
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, plus_di(14, 1), minus_di(14, 1)),
                    compare(ComparisonOp::GreaterThan, minus_di(14, 1), plus_di(14, 1)),
                ),
            ]
        }
        SearchFamily::MomentumBurst => {
            let period = choose_period(rng);
            let upper = rng.gen_range(52.0..=65.0);
            let lower = 100.0 - upper;
            let roc_period = choose_period(rng);
            let roc_level = rng.gen_range(0.1..=2.5);
            let mid_period = choose_period(rng);
            let sma_period = choose_period(rng);
            vec![
                // Original: RSI extremes
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        rsi(period, 1),
                        NumericExpr::Constant { value: upper },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        rsi(period, 1),
                        NumericExpr::Constant { value: lower },
                    ),
                ),
                // +1 ROC magnitude
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        roc(roc_period, 1),
                        NumericExpr::Constant { value: roc_level },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        roc(roc_period, 1),
                        NumericExpr::Constant { value: -roc_level },
                    ),
                ),
                // +2 RSI side of 50
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        rsi(mid_period, 1),
                        NumericExpr::Constant { value: 50.0 },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        rsi(mid_period, 1),
                        NumericExpr::Constant { value: 50.0 },
                    ),
                ),
                // +3 Close vs SMA momentum
                (
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, plus_di(14, 1), minus_di(14, 1)),
                    compare(ComparisonOp::GreaterThan, minus_di(14, 1), plus_di(14, 1)),
                ),
            ]
        }
        SearchFamily::DonchianBreakout => {
            let period = choose_period(rng).max(10);
            let high_period = choose_period(rng).max(10);
            let sma_period = choose_period(rng);
            let roc_period = choose_period(rng);
            vec![
                // Original: Donchian
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::DonchianHigh { period, shift: 2 },
                        },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::DonchianLow { period, shift: 2 },
                        },
                    ),
                ),
                // +1 Highest / Lowest channel
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::Highest {
                                source: PriceField::High,
                                period: high_period,
                                shift: 2,
                            },
                        },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::Lowest {
                                source: PriceField::Low,
                                period: high_period,
                                shift: 2,
                            },
                        },
                    ),
                ),
                // +2 Close vs SMA confirmation
                (
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                ),
                // +3 Positive / negative ROC expansion
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        roc(roc_period, 1),
                        NumericExpr::Constant { value: 0.05 },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        roc(roc_period, 1),
                        NumericExpr::Constant { value: -0.05 },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, plus_di(14, 1), minus_di(14, 1)),
                    compare(ComparisonOp::GreaterThan, minus_di(14, 1), plus_di(14, 1)),
                ),
            ]
        }
        SearchFamily::MeanReversionBand => {
            let period = choose_period(rng);
            let lower = rng.gen_range(20.0..=40.0);
            let upper = 100.0 - lower;
            let pct_period = choose_period(rng).max(10);
            let pct_low = rng.gen_range(5.0..=25.0);
            let pct_high = 100.0 - pct_low;
            let sma_period = choose_period(rng);
            vec![
                (
                    compare(
                        ComparisonOp::LessThan,
                        rsi(period, 1),
                        NumericExpr::Constant { value: lower },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        rsi(period, 1),
                        NumericExpr::Constant { value: upper },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::LessThan,
                        percentile(pct_period, 1),
                        NumericExpr::Constant { value: pct_low },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        percentile(pct_period, 1),
                        NumericExpr::Constant { value: pct_high },
                    ),
                ),
                (
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                ),
            ]
        }
        SearchFamily::ZScoreReversion => {
            let z_period = choose_period(rng);
            let z_level = rng.gen_range(1.0..=2.5);
            let soft = z_level * 0.75;
            let sma_period = choose_period(rng);
            vec![
                (
                    compare(
                        ComparisonOp::LessThan,
                        zscore(z_period, 1),
                        NumericExpr::Constant { value: -z_level },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        zscore(z_period, 1),
                        NumericExpr::Constant { value: z_level },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::LessThan,
                        zscore(z_period, 1),
                        NumericExpr::Constant { value: -soft },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        zscore(z_period, 1),
                        NumericExpr::Constant { value: soft },
                    ),
                ),
                (
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                ),
            ]
        }
        SearchFamily::SessionOrb => {
            let start_hour = [7u8, 8, 9, 13, 14][rng.gen_range(0..5)];
            let range_bars = [2u16, 3, 4][rng.gen_range(0..3)];
            let orb_high = NumericExpr::Indicator {
                value: IndicatorExpr::SessionRangeHigh {
                    start_hour,
                    range_bars,
                    shift: 1,
                },
            };
            let orb_low = NumericExpr::Indicator {
                value: IndicatorExpr::SessionRangeLow {
                    start_hour,
                    range_bars,
                    shift: 1,
                },
            };
            let sma_period = choose_period(rng);
            vec![
                (
                    compare(ComparisonOp::GreaterThan, close(1), orb_high.clone()),
                    compare(ComparisonOp::LessThan, close(1), orb_low.clone()),
                ),
                (
                    BoolExpr::CrossAbove {
                        left: close(1),
                        right: orb_high.clone(),
                    },
                    BoolExpr::CrossBelow {
                        left: close(1),
                        right: orb_low.clone(),
                    },
                ),
                (
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, plus_di(14, 1), minus_di(14, 1)),
                    compare(ComparisonOp::GreaterThan, minus_di(14, 1), plus_di(14, 1)),
                ),
            ]
        }
        SearchFamily::ImpulseCandle => {
            let body_min = rng.gen_range(0.55..=0.75);
            let loc_high = rng.gen_range(0.70..=0.90);
            let loc_low = 1.0 - loc_high;
            let body = NumericExpr::Indicator {
                value: IndicatorExpr::BodyRangeRatio { shift: 1 },
            };
            let location = NumericExpr::Indicator {
                value: IndicatorExpr::CloseLocationInBar { shift: 1 },
            };
            let sma_period = choose_period(rng);
            vec![
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        body.clone(),
                        NumericExpr::Constant { value: body_min },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        body,
                        NumericExpr::Constant { value: body_min },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        location.clone(),
                        NumericExpr::Constant { value: loc_high },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        location,
                        NumericExpr::Constant { value: loc_low },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                ),
            ]
        }
        SearchFamily::VolSqueezeBreak => {
            let lookback = [20u16, 40, 60][rng.gen_range(0..3)];
            let squeeze_max = rng.gen_range(15.0..=35.0);
            let atr_pct = NumericExpr::Indicator {
                value: IndicatorExpr::AtrPercentile {
                    atr_period: FROZEN_ATR_PERIOD,
                    lookback,
                    shift: 1,
                },
            };
            let channel = choose_period(rng).max(10);
            vec![
                (
                    compare(
                        ComparisonOp::LessThan,
                        atr_pct.clone(),
                        NumericExpr::Constant { value: squeeze_max },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        atr_pct,
                        NumericExpr::Constant { value: squeeze_max },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::DonchianHigh {
                                period: channel,
                                shift: 2,
                            },
                        },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::DonchianLow {
                                period: channel,
                                shift: 2,
                            },
                        },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        roc(choose_period(rng), 1),
                        NumericExpr::Constant { value: 0.05 },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        roc(choose_period(rng), 1),
                        NumericExpr::Constant { value: -0.05 },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        adx(14, 1),
                        NumericExpr::Constant { value: 25.0 },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, plus_di(14, 1), minus_di(14, 1)),
                    compare(ComparisonOp::GreaterThan, minus_di(14, 1), plus_di(14, 1)),
                ),
            ]
        }
        SearchFamily::SupplyDemandReclaim => {
            let swing = [2u16, 3, 4][rng.gen_range(0..3)];
            let base_bars = [2u16, 3, 4][rng.gen_range(0..3)];
            let zone_high = NumericExpr::Indicator {
                value: IndicatorExpr::SwingBaseZoneHigh {
                    swing_left: swing,
                    swing_right: swing,
                    base_bars,
                    shift: 1,
                },
            };
            let zone_low = NumericExpr::Indicator {
                value: IndicatorExpr::SwingBaseZoneLow {
                    swing_left: swing,
                    swing_right: swing,
                    base_bars,
                    shift: 1,
                },
            };
            vec![
                (
                    BoolExpr::CrossAbove {
                        left: close(1),
                        right: zone_high.clone(),
                    },
                    BoolExpr::CrossBelow {
                        left: close(1),
                        right: zone_low.clone(),
                    },
                ),
                (
                    compare(ComparisonOp::GreaterThan, close(1), zone_high.clone()),
                    compare(ComparisonOp::LessThan, close(1), zone_low.clone()),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        ema(choose_period(rng), 1),
                    ),
                    compare(ComparisonOp::LessThan, close(1), ema(choose_period(rng), 1)),
                ),
            ]
        }
        SearchFamily::SweepReclaim => {
            let period = choose_period(rng).max(10);
            let score = NumericExpr::Indicator {
                value: IndicatorExpr::LiquiditySweepScore { period, shift: 1 },
            };
            let sma_period = choose_period(rng);
            vec![
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        score.clone(),
                        NumericExpr::Constant { value: 0.0 },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        score.clone(),
                        NumericExpr::Constant { value: 0.0 },
                    ),
                ),
                (
                    compare(
                        ComparisonOp::GreaterThan,
                        score.clone(),
                        NumericExpr::Constant { value: 0.5 },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        score,
                        NumericExpr::Constant { value: -0.5 },
                    ),
                ),
                (
                    compare(ComparisonOp::GreaterThan, close(1), sma(sma_period, 1)),
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                ),
            ]
        }
        SearchFamily::Universal => universal_entry_atoms(rng, &UniversalGrammarConfig::default()),
    }
}

fn and_all(parts: Vec<BoolExpr>) -> BoolExpr {
    match parts.len() {
        0 => BoolExpr::Compare {
            comparison: ComparisonOp::GreaterThan,
            left: NumericExpr::Constant { value: 1.0 },
            right: NumericExpr::Constant { value: 0.0 },
        },
        1 => parts.into_iter().next().expect("len checked"),
        _ => BoolExpr::And { children: parts },
    }
}

fn or_all(parts: Vec<BoolExpr>) -> BoolExpr {
    match parts.len() {
        0 => BoolExpr::Compare {
            comparison: ComparisonOp::LessThan,
            left: NumericExpr::Constant { value: 1.0 },
            right: NumericExpr::Constant { value: 0.0 },
        },
        1 => parts.into_iter().next().expect("len checked"),
        _ => BoolExpr::Or { children: parts },
    }
}

fn rebase_bool_shifts(expression: &mut BoolExpr, base_shift: u16, maximum_shift: u16) {
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            rebase_numeric_shift(left, base_shift, maximum_shift);
            rebase_numeric_shift(right, base_shift, maximum_shift);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            rebase_numeric_shift(value, base_shift, maximum_shift);
            rebase_numeric_shift(lower, base_shift, maximum_shift);
            rebase_numeric_shift(upper, base_shift, maximum_shift);
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                rebase_bool_shifts(child, base_shift, maximum_shift);
            }
        }
        BoolExpr::Not { child } => rebase_bool_shifts(child, base_shift, maximum_shift),
    }
}

fn rebase_numeric_shift(expression: &mut NumericExpr, base_shift: u16, maximum_shift: u16) {
    let old_shift = match expression {
        NumericExpr::Price { shift, .. } | NumericExpr::Context { shift, .. } => shift,
        NumericExpr::Indicator { value } => match value {
            IndicatorExpr::Sma { shift, .. }
            | IndicatorExpr::Ema { shift, .. }
            | IndicatorExpr::Wma { shift, .. }
            | IndicatorExpr::Rsi { shift, .. }
            | IndicatorExpr::Atr { shift, .. }
            | IndicatorExpr::Adx { shift, .. }
            | IndicatorExpr::PlusDi { shift, .. }
            | IndicatorExpr::MinusDi { shift, .. }
            | IndicatorExpr::DonchianHigh { shift, .. }
            | IndicatorExpr::DonchianLow { shift, .. }
            | IndicatorExpr::Highest { shift, .. }
            | IndicatorExpr::Lowest { shift, .. }
            | IndicatorExpr::StandardDeviation { shift, .. }
            | IndicatorExpr::ZScore { shift, .. }
            | IndicatorExpr::PercentileInRange { shift, .. }
            | IndicatorExpr::RateOfChange { shift, .. }
            | IndicatorExpr::SessionRangeHigh { shift, .. }
            | IndicatorExpr::SessionRangeLow { shift, .. }
            | IndicatorExpr::BodyRangeRatio { shift }
            | IndicatorExpr::CloseLocationInBar { shift }
            | IndicatorExpr::AtrPercentile { shift, .. }
            | IndicatorExpr::SwingBaseZoneHigh { shift, .. }
            | IndicatorExpr::SwingBaseZoneLow { shift, .. }
            | IndicatorExpr::LiquiditySweepScore { shift, .. }
            | IndicatorExpr::MacdMain { shift, .. }
            | IndicatorExpr::MacdSignal { shift, .. }
            | IndicatorExpr::MacdHistogram { shift, .. }
            | IndicatorExpr::BollingerMid { shift, .. }
            | IndicatorExpr::BollingerUpper { shift, .. }
            | IndicatorExpr::BollingerLower { shift, .. }
            | IndicatorExpr::BollingerBandwidth { shift, .. }
            | IndicatorExpr::IchimokuTenkan { shift, .. }
            | IndicatorExpr::IchimokuKijun { shift, .. }
            | IndicatorExpr::IchimokuSenkouA { shift, .. }
            | IndicatorExpr::IchimokuSenkouB { shift, .. }
            | IndicatorExpr::QqeLine { shift, .. }
            | IndicatorExpr::QqeTrail { shift, .. }
            | IndicatorExpr::Vwap { shift, .. }
            | IndicatorExpr::Cci { shift, .. } => shift,
        },
        NumericExpr::Constant { .. } => return,
    };
    let relative = old_shift.saturating_sub(1);
    *old_shift = base_shift
        .saturating_add(relative)
        .min(maximum_shift)
        .max(1);
}
fn compare(comparison: ComparisonOp, left: NumericExpr, right: NumericExpr) -> BoolExpr {
    BoolExpr::Compare {
        comparison,
        left,
        right,
    }
}

fn close(shift: u16) -> NumericExpr {
    NumericExpr::Price {
        field: PriceField::Close,
        shift,
    }
}

fn ema(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::Ema {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn sma(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::Sma {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn rsi(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::Rsi {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn adx(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::Adx { period, shift },
    }
}

fn plus_di(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::PlusDi { period, shift },
    }
}

fn minus_di(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::MinusDi { period, shift },
    }
}

fn roc(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::RateOfChange {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn zscore(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::ZScore {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn percentile(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::PercentileInRange {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn macd_main(fast_period: u16, slow_period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::MacdMain {
            source: PriceField::Close,
            fast_period,
            slow_period,
            shift,
        },
    }
}

fn macd_signal(fast_period: u16, slow_period: u16, signal_period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::MacdSignal {
            source: PriceField::Close,
            fast_period,
            slow_period,
            signal_period,
            shift,
        },
    }
}

fn macd_histogram(
    fast_period: u16,
    slow_period: u16,
    signal_period: u16,
    shift: u16,
) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::MacdHistogram {
            source: PriceField::Close,
            fast_period,
            slow_period,
            signal_period,
            shift,
        },
    }
}

fn bollinger_mid(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::BollingerMid {
            source: PriceField::Close,
            period,
            shift,
        },
    }
}

fn bollinger_upper(period: u16, deviation_tenths: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::BollingerUpper {
            source: PriceField::Close,
            period,
            deviation_tenths,
            shift,
        },
    }
}

fn bollinger_lower(period: u16, deviation_tenths: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::BollingerLower {
            source: PriceField::Close,
            period,
            deviation_tenths,
            shift,
        },
    }
}

fn bollinger_bandwidth(period: u16, deviation_tenths: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::BollingerBandwidth {
            source: PriceField::Close,
            period,
            deviation_tenths,
            shift,
        },
    }
}

fn ichimoku_tenkan(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::IchimokuTenkan { period, shift },
    }
}

fn ichimoku_kijun(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::IchimokuKijun { period, shift },
    }
}

fn ichimoku_senkou_a(tenkan_period: u16, kijun_period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::IchimokuSenkouA {
            tenkan_period,
            kijun_period,
            shift,
        },
    }
}

fn ichimoku_senkou_b(period: u16, kijun_period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::IchimokuSenkouB {
            period,
            kijun_period,
            shift,
        },
    }
}

fn qqe_line(rsi_period: u16, smoothing_period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::QqeLine {
            rsi_period,
            smoothing_period,
            shift,
        },
    }
}

fn qqe_trail(
    rsi_period: u16,
    smoothing_period: u16,
    factor_tenths: u16,
    shift: u16,
) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::QqeTrail {
            rsi_period,
            smoothing_period,
            factor_tenths,
            shift,
        },
    }
}

fn vwap(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::Vwap { period, shift },
    }
}

fn cci(period: u16, shift: u16) -> NumericExpr {
    NumericExpr::Indicator {
        value: IndicatorExpr::Cci { period, shift },
    }
}

fn constant(value: f64) -> NumericExpr {
    NumericExpr::Constant { value }
}

/// Exit blocks for the added indicator families. Each pair closes the trade on
/// the signal that would have opened the opposite side.
fn extended_exit_atoms(rng: &mut ChaCha8Rng) -> Vec<(BoolExpr, BoolExpr)> {
    let shift = 1_u16;
    let fast = MACD_FAST[rng.gen_range(0..MACD_FAST.len())];
    let slow = MACD_SLOW[rng.gen_range(0..MACD_SLOW.len())].max(fast + 1);
    let signal = MACD_SIGNAL[rng.gen_range(0..MACD_SIGNAL.len())];
    let band_period = choose_period(rng).max(10);
    let kijun = ICHIMOKU_KIJUN[rng.gen_range(0..ICHIMOKU_KIJUN.len())];
    let qqe_rsi = choose_period(rng);
    let qqe_smoothing = QQE_SMOOTHING[rng.gen_range(0..QQE_SMOOTHING.len())];
    let qqe_factor = QQE_FACTOR_TENTHS[rng.gen_range(0..QQE_FACTOR_TENTHS.len())];
    let vwap_period = VWAP_PERIODS[rng.gen_range(0..VWAP_PERIODS.len())];
    let cci_period = choose_period(rng);

    vec![
        (
            BoolExpr::CrossBelow {
                left: macd_histogram(fast, slow, signal, shift),
                right: constant(0.0),
            },
            BoolExpr::CrossAbove {
                left: macd_histogram(fast, slow, signal, shift),
                right: constant(0.0),
            },
        ),
        (
            BoolExpr::CrossBelow {
                left: close(shift),
                right: bollinger_mid(band_period, shift),
            },
            BoolExpr::CrossAbove {
                left: close(shift),
                right: bollinger_mid(band_period, shift),
            },
        ),
        (
            BoolExpr::CrossBelow {
                left: close(shift),
                right: ichimoku_kijun(kijun, shift),
            },
            BoolExpr::CrossAbove {
                left: close(shift),
                right: ichimoku_kijun(kijun, shift),
            },
        ),
        (
            BoolExpr::CrossBelow {
                left: qqe_line(qqe_rsi, qqe_smoothing, shift),
                right: qqe_trail(qqe_rsi, qqe_smoothing, qqe_factor, shift),
            },
            BoolExpr::CrossAbove {
                left: qqe_line(qqe_rsi, qqe_smoothing, shift),
                right: qqe_trail(qqe_rsi, qqe_smoothing, qqe_factor, shift),
            },
        ),
        (
            BoolExpr::CrossBelow {
                left: close(shift),
                right: vwap(vwap_period, shift),
            },
            BoolExpr::CrossAbove {
                left: close(shift),
                right: vwap(vwap_period, shift),
            },
        ),
        (
            BoolExpr::CrossBelow {
                left: cci(cci_period, shift),
                right: constant(0.0),
            },
            BoolExpr::CrossAbove {
                left: cci(cci_period, shift),
                right: constant(0.0),
            },
        ),
    ]
}

/// Condition blocks for the indicator families added alongside the original
/// ten catalogs: MACD, Bollinger, Ichimoku, QQE, VWAP and CCI.
fn extended_entry_atoms(rng: &mut ChaCha8Rng) -> Vec<(BoolExpr, BoolExpr)> {
    let shift = 1_u16;
    let fast = MACD_FAST[rng.gen_range(0..MACD_FAST.len())];
    let slow = MACD_SLOW[rng.gen_range(0..MACD_SLOW.len())].max(fast + 1);
    let signal = MACD_SIGNAL[rng.gen_range(0..MACD_SIGNAL.len())];
    let band_period = choose_period(rng).max(10);
    let deviation = BB_DEVIATION_TENTHS[rng.gen_range(0..BB_DEVIATION_TENTHS.len())];
    let squeeze = rng.gen_range(1.0..=3.0);
    let tenkan = ICHIMOKU_TENKAN[rng.gen_range(0..ICHIMOKU_TENKAN.len())];
    let kijun = ICHIMOKU_KIJUN[rng.gen_range(0..ICHIMOKU_KIJUN.len())];
    let senkou = ICHIMOKU_SENKOU[rng.gen_range(0..ICHIMOKU_SENKOU.len())];
    let qqe_rsi = choose_period(rng);
    let qqe_smoothing = QQE_SMOOTHING[rng.gen_range(0..QQE_SMOOTHING.len())];
    let qqe_factor = QQE_FACTOR_TENTHS[rng.gen_range(0..QQE_FACTOR_TENTHS.len())];
    let vwap_period = VWAP_PERIODS[rng.gen_range(0..VWAP_PERIODS.len())];
    let cci_period = choose_period(rng);
    let cci_level = CCI_LEVELS[rng.gen_range(0..CCI_LEVELS.len())];

    vec![
        // MACD histogram sign flip.
        (
            BoolExpr::CrossAbove {
                left: macd_histogram(fast, slow, signal, shift),
                right: constant(0.0),
            },
            BoolExpr::CrossBelow {
                left: macd_histogram(fast, slow, signal, shift),
                right: constant(0.0),
            },
        ),
        // MACD main crossing its signal line.
        (
            BoolExpr::CrossAbove {
                left: macd_main(fast, slow, shift),
                right: macd_signal(fast, slow, signal, shift),
            },
            BoolExpr::CrossBelow {
                left: macd_main(fast, slow, shift),
                right: macd_signal(fast, slow, signal, shift),
            },
        ),
        // MACD main above or below the zero line.
        (
            compare(
                ComparisonOp::GreaterThan,
                macd_main(fast, slow, shift),
                constant(0.0),
            ),
            compare(
                ComparisonOp::LessThan,
                macd_main(fast, slow, shift),
                constant(0.0),
            ),
        ),
        // Bollinger breakout through the outer band.
        (
            BoolExpr::CrossAbove {
                left: close(shift),
                right: bollinger_upper(band_period, deviation, shift),
            },
            BoolExpr::CrossBelow {
                left: close(shift),
                right: bollinger_lower(band_period, deviation, shift),
            },
        ),
        // Bollinger stretch: price already outside the band (mean reversion).
        (
            compare(
                ComparisonOp::LessThan,
                close(shift),
                bollinger_lower(band_period, deviation, shift),
            ),
            compare(
                ComparisonOp::GreaterThan,
                close(shift),
                bollinger_upper(band_period, deviation, shift),
            ),
        ),
        // Bollinger middle-band reclaim.
        (
            BoolExpr::CrossAbove {
                left: close(shift),
                right: bollinger_mid(band_period, shift),
            },
            BoolExpr::CrossBelow {
                left: close(shift),
                right: bollinger_mid(band_period, shift),
            },
        ),
        // Bollinger squeeze regime, identical on both sides.
        (
            compare(
                ComparisonOp::LessThan,
                bollinger_bandwidth(band_period, deviation, shift),
                constant(squeeze),
            ),
            compare(
                ComparisonOp::LessThan,
                bollinger_bandwidth(band_period, deviation, shift),
                constant(squeeze),
            ),
        ),
        // Ichimoku conversion crossing the base line.
        (
            BoolExpr::CrossAbove {
                left: ichimoku_tenkan(tenkan, shift),
                right: ichimoku_kijun(kijun, shift),
            },
            BoolExpr::CrossBelow {
                left: ichimoku_tenkan(tenkan, shift),
                right: ichimoku_kijun(kijun, shift),
            },
        ),
        // Price clear of the whole cloud.
        (
            BoolExpr::And {
                children: vec![
                    compare(
                        ComparisonOp::GreaterThan,
                        close(shift),
                        ichimoku_senkou_a(tenkan, kijun, shift),
                    ),
                    compare(
                        ComparisonOp::GreaterThan,
                        close(shift),
                        ichimoku_senkou_b(senkou, kijun, shift),
                    ),
                ],
            },
            BoolExpr::And {
                children: vec![
                    compare(
                        ComparisonOp::LessThan,
                        close(shift),
                        ichimoku_senkou_a(tenkan, kijun, shift),
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        close(shift),
                        ichimoku_senkou_b(senkou, kijun, shift),
                    ),
                ],
            },
        ),
        // Price versus the Ichimoku base line.
        (
            compare(
                ComparisonOp::GreaterThan,
                close(shift),
                ichimoku_kijun(kijun, shift),
            ),
            compare(
                ComparisonOp::LessThan,
                close(shift),
                ichimoku_kijun(kijun, shift),
            ),
        ),
        // QQE smoothed RSI crossing its trailing level.
        (
            BoolExpr::CrossAbove {
                left: qqe_line(qqe_rsi, qqe_smoothing, shift),
                right: qqe_trail(qqe_rsi, qqe_smoothing, qqe_factor, shift),
            },
            BoolExpr::CrossBelow {
                left: qqe_line(qqe_rsi, qqe_smoothing, shift),
                right: qqe_trail(qqe_rsi, qqe_smoothing, qqe_factor, shift),
            },
        ),
        // QQE regime relative to the RSI midline.
        (
            compare(
                ComparisonOp::GreaterThan,
                qqe_line(qqe_rsi, qqe_smoothing, shift),
                constant(50.0),
            ),
            compare(
                ComparisonOp::LessThan,
                qqe_line(qqe_rsi, qqe_smoothing, shift),
                constant(50.0),
            ),
        ),
        // VWAP reclaim.
        (
            BoolExpr::CrossAbove {
                left: close(shift),
                right: vwap(vwap_period, shift),
            },
            BoolExpr::CrossBelow {
                left: close(shift),
                right: vwap(vwap_period, shift),
            },
        ),
        // Price holding one side of VWAP.
        (
            compare(
                ComparisonOp::GreaterThan,
                close(shift),
                vwap(vwap_period, shift),
            ),
            compare(
                ComparisonOp::LessThan,
                close(shift),
                vwap(vwap_period, shift),
            ),
        ),
        // CCI breaking the classic trigger level.
        (
            BoolExpr::CrossAbove {
                left: cci(cci_period, shift),
                right: constant(cci_level),
            },
            BoolExpr::CrossBelow {
                left: cci(cci_period, shift),
                right: constant(-cci_level),
            },
        ),
        // CCI zero-line momentum.
        (
            BoolExpr::CrossAbove {
                left: cci(cci_period, shift),
                right: constant(0.0),
            },
            BoolExpr::CrossBelow {
                left: cci(cci_period, shift),
                right: constant(0.0),
            },
        ),
        // CCI exhaustion (mean reversion).
        (
            compare(
                ComparisonOp::LessThan,
                cci(cci_period, shift),
                constant(-cci_level),
            ),
            compare(
                ComparisonOp::GreaterThan,
                cci(cci_period, shift),
                constant(cci_level),
            ),
        ),
    ]
}

fn session_filter(rng: &mut ChaCha8Rng) -> BoolExpr {
    let lower = rng.gen_range(0..=12) as f64;
    let upper = rng.gen_range((lower as u16 + 6).min(23)..=23) as f64;
    BoolExpr::Between {
        value: NumericExpr::Context {
            value: ContextValue::SessionHour,
            shift: 1,
        },
        lower: NumericExpr::Constant { value: lower },
        upper: NumericExpr::Constant { value: upper },
    }
}

fn random_stops(rng: &mut ChaCha8Rng) -> ProtectiveStops {
    // FixedPoints are banned: pip/point stops are not comparable across symbols
    // (3-decimal JPY vs 5-decimal majors). ATR multiples scale with volatility.
    let stop_loss = StopLossPolicy::AtrMultiple {
        period: choose_atr_period(rng),
        multiplier: choose_from(rng, &ATR_STOP_MULTIPLIERS),
    };
    let take_profit = if rng.gen_bool(0.7) {
        TakeProfitPolicy::RiskMultiple {
            multiple: choose_from(rng, &RISK_MULTIPLES),
        }
    } else {
        TakeProfitPolicy::AtrMultiple {
            period: choose_atr_period(rng),
            multiplier: choose_from(rng, &ATR_TP_MULTIPLIERS),
        }
    };
    ProtectiveStops {
        stop_loss,
        take_profit,
    }
}

fn random_entry_distance(rng: &mut ChaCha8Rng) -> EntryDistancePolicy {
    EntryDistancePolicy::AtrMultiple {
        period: choose_atr_period(rng),
        multiplier: choose_from(rng, &ATR_ENTRY_MULTIPLIERS),
    }
}

pub(crate) fn random_entry_order(rng: &mut ChaCha8Rng) -> EntryOrderPolicy {
    match rng.gen_range(0..5) {
        0 => EntryOrderPolicy::Stop {
            distance: random_entry_distance(rng),
            expiry_bars: rng.gen_range(1..=12),
        },
        1 => EntryOrderPolicy::Limit {
            distance: random_entry_distance(rng),
            expiry_bars: rng.gen_range(1..=12),
        },
        _ => EntryOrderPolicy::Market,
    }
}

/// Institutional Search Families: stop or limit only (no market).
pub(crate) fn random_pending_entry_order(rng: &mut ChaCha8Rng) -> EntryOrderPolicy {
    let distance = random_entry_distance(rng);
    let expiry_bars = rng.gen_range(2..=8);
    if rng.gen_bool(0.5) {
        EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        }
    } else {
        EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        }
    }
}

fn random_manage(rng: &mut ChaCha8Rng) -> ManagePolicy {
    let partial_exits = if rng.gen_bool(0.25) {
        if rng.gen_bool(0.35) {
            vec![
                PartialExit {
                    at_r: rng.gen_range(0.5..=1.25),
                    fraction: 0.5,
                },
                PartialExit {
                    at_r: rng.gen_range(1.5..=3.0),
                    fraction: 0.5,
                },
            ]
        } else {
            vec![PartialExit {
                at_r: rng.gen_range(0.5..=2.0),
                fraction: rng.gen_range(0.25..=0.75),
            }]
        }
    } else {
        Vec::new()
    };
    let trailing = rng.gen_bool(0.25).then(|| {
        if rng.gen_bool(0.6) {
            TrailingPolicy::RiskMultiple {
                activate_at_r: choose_from(rng, &R_ACTIVATE),
                distance_r: choose_from(rng, &R_DISTANCE),
            }
        } else {
            TrailingPolicy::AtrMultiple {
                activate_at_r: choose_from(rng, &R_ACTIVATE),
                period: choose_atr_period(rng),
                multiplier: choose_from(rng, &ATR_TRAIL_MULTIPLIERS),
            }
        }
    });
    ManagePolicy {
        break_even_at_r: rng.gen_bool(0.3).then(|| choose_from(rng, &R_ACTIVATE)),
        trailing,
        time_stop_bars: rng.gen_bool(0.45).then(|| rng.gen_range(6..=80)),
        partial_exits,
        // Production applies these as immutable job policies, never genes.
        flatten_end_of_day: false,
        max_one_entry_per_day: false,
        ..Default::default()
    }
}

fn mutate_policies(strategy: &mut StrategyIr, rng: &mut ChaCha8Rng) {
    // Convert any legacy FixedPoints / RangeMultiple into ATR multiples.
    strategy.stops.stop_loss = match &strategy.stops.stop_loss {
        StopLossPolicy::FixedPoints { .. } | StopLossPolicy::RangeMultiple { .. } => {
            StopLossPolicy::AtrMultiple {
                period: choose_atr_period(rng),
                multiplier: choose_from(rng, &ATR_STOP_MULTIPLIERS),
            }
        }
        StopLossPolicy::AtrMultiple { period, multiplier } => StopLossPolicy::AtrMultiple {
            period: snap_period(*period),
            multiplier: snap_to_ladder(*multiplier, &ATR_STOP_MULTIPLIERS),
        },
    };
    if let StopLossPolicy::AtrMultiple { period, multiplier } = &mut strategy.stops.stop_loss {
        mutate_period(period, rng);
        *multiplier = mutate_ladder_value(*multiplier, rng, &ATR_STOP_MULTIPLIERS);
    }
    strategy.stops.take_profit = match &strategy.stops.take_profit {
        TakeProfitPolicy::FixedPoints { .. } => TakeProfitPolicy::RiskMultiple {
            multiple: choose_from(rng, &RISK_MULTIPLES),
        },
        TakeProfitPolicy::RiskMultiple { multiple } => TakeProfitPolicy::RiskMultiple {
            multiple: snap_to_ladder(*multiple, &RISK_MULTIPLES),
        },
        TakeProfitPolicy::AtrMultiple { period, multiplier } => TakeProfitPolicy::AtrMultiple {
            period: snap_period(*period),
            multiplier: snap_to_ladder(*multiplier, &ATR_TP_MULTIPLIERS),
        },
    };
    match &mut strategy.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => {
            *multiple = mutate_ladder_value(*multiple, rng, &RISK_MULTIPLES);
        }
        TakeProfitPolicy::AtrMultiple { period, multiplier } => {
            mutate_period(period, rng);
            *multiplier = mutate_ladder_value(*multiplier, rng, &ATR_TP_MULTIPLIERS);
        }
        TakeProfitPolicy::FixedPoints { .. } => {}
    }
    if let Some(bars) = &mut strategy.manage.time_stop_bars {
        let delta = rng.gen_range(-4_i32..=4);
        *bars = (i32::from(*bars) + delta).clamp(2, 500) as u16;
    }
    match &mut strategy.entry.order {
        EntryOrderPolicy::Market => {
            strategy.entry.order = random_pending_entry_order(rng);
        }
        EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        }
        | EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        } => {
            mutate_entry_distance(distance, rng);
            let delta = rng.gen_range(-2_i32..=2);
            *expiry_bars = (i32::from(*expiry_bars) + delta).clamp(1, 100) as u16;
        }
    }
    if rng.gen_bool(0.15) {
        strategy.entry.order = match &strategy.entry.order {
            EntryOrderPolicy::Stop {
                distance,
                expiry_bars,
            } => EntryOrderPolicy::Limit {
                distance: distance.clone(),
                expiry_bars: *expiry_bars,
            },
            EntryOrderPolicy::Limit {
                distance,
                expiry_bars,
            } => EntryOrderPolicy::Stop {
                distance: distance.clone(),
                expiry_bars: *expiry_bars,
            },
            EntryOrderPolicy::Market => random_pending_entry_order(rng),
        };
    }
    if let Some(value) = &mut strategy.manage.break_even_at_r {
        *value = mutate_ladder_value(*value, rng, &R_ACTIVATE);
    }
    if let Some(trailing) = &mut strategy.manage.trailing {
        match trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => {
                *activate_at_r = mutate_ladder_value(*activate_at_r, rng, &R_ACTIVATE);
                *distance_r = mutate_ladder_value(*distance_r, rng, &R_DISTANCE);
            }
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                period,
                multiplier,
            } => {
                *activate_at_r = mutate_ladder_value(*activate_at_r, rng, &R_ACTIVATE);
                mutate_period(period, rng);
                *multiplier = mutate_ladder_value(*multiplier, rng, &ATR_TRAIL_MULTIPLIERS);
            }
        }
    }
    for partial in &mut strategy.manage.partial_exits {
        partial.at_r = snap_quarter(partial.at_r * rng.gen_range(0.8..=1.2)).clamp(0.25, 4.0);
        partial.fraction = (partial.fraction * rng.gen_range(0.85..=1.15)).clamp(0.05, 1.0);
    }
    let total_fraction: f64 = strategy
        .manage
        .partial_exits
        .iter()
        .map(|partial| partial.fraction)
        .sum();
    if total_fraction > 1.0 {
        for partial in &mut strategy.manage.partial_exits {
            partial.fraction /= total_fraction;
        }
    }
}

fn mutate_entry_distance(distance: &mut EntryDistancePolicy, rng: &mut ChaCha8Rng) {
    *distance = match distance {
        EntryDistancePolicy::FixedPoints { .. } | EntryDistancePolicy::RangeMultiple { .. } => {
            EntryDistancePolicy::AtrMultiple {
                period: choose_atr_period(rng),
                multiplier: choose_from(rng, &ATR_ENTRY_MULTIPLIERS),
            }
        }
        EntryDistancePolicy::AtrMultiple { period, multiplier } => {
            let mut period = snap_period(*period);
            mutate_period(&mut period, rng);
            EntryDistancePolicy::AtrMultiple {
                period,
                multiplier: mutate_ladder_value(*multiplier, rng, &ATR_ENTRY_MULTIPLIERS),
            }
        }
    };
}

fn mutate_bool(expression: &mut BoolExpr, rng: &mut ChaCha8Rng) {
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            mutate_numeric(left, rng);
            mutate_numeric(right, rng);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            mutate_numeric(value, rng);
            mutate_numeric(lower, rng);
            mutate_numeric(upper, rng);
            if let (NumericExpr::Constant { value: lower }, NumericExpr::Constant { value: upper }) =
                (lower, upper)
                && *lower > *upper
            {
                std::mem::swap(lower, upper);
            }
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                mutate_bool(child, rng);
            }
        }
        BoolExpr::Not { child } => mutate_bool(child, rng),
    }
}

fn mutate_numeric(expression: &mut NumericExpr, rng: &mut ChaCha8Rng) {
    match expression {
        NumericExpr::Indicator { value } => mutate_indicator(value, rng),
        NumericExpr::Constant { value } => {
            *value = (*value + rng.gen_range(-3.0..=3.0)).clamp(0.0, 100.0);
        }
        NumericExpr::Price { .. } | NumericExpr::Context { .. } => {}
    }
}

fn mutate_indicator(indicator: &mut IndicatorExpr, rng: &mut ChaCha8Rng) {
    match indicator {
        IndicatorExpr::Sma { period, .. }
        | IndicatorExpr::Ema { period, .. }
        | IndicatorExpr::Wma { period, .. }
        | IndicatorExpr::Rsi { period, .. }
        | IndicatorExpr::Atr { period, .. }
        | IndicatorExpr::Adx { period, .. }
        | IndicatorExpr::PlusDi { period, .. }
        | IndicatorExpr::MinusDi { period, .. }
        | IndicatorExpr::DonchianHigh { period, .. }
        | IndicatorExpr::DonchianLow { period, .. }
        | IndicatorExpr::Highest { period, .. }
        | IndicatorExpr::Lowest { period, .. }
        | IndicatorExpr::StandardDeviation { period, .. }
        | IndicatorExpr::ZScore { period, .. }
        | IndicatorExpr::PercentileInRange { period, .. }
        | IndicatorExpr::RateOfChange { period, .. }
        | IndicatorExpr::LiquiditySweepScore { period, .. } => mutate_period(period, rng),
        IndicatorExpr::SessionRangeHigh {
            start_hour,
            range_bars,
            ..
        }
        | IndicatorExpr::SessionRangeLow {
            start_hour,
            range_bars,
            ..
        } => {
            *start_hour = [7u8, 8, 9, 13, 14][rng.gen_range(0..5)];
            *range_bars = [2u16, 3, 4][rng.gen_range(0..3)];
        }
        IndicatorExpr::BodyRangeRatio { .. } | IndicatorExpr::CloseLocationInBar { .. } => {}
        IndicatorExpr::AtrPercentile { lookback, .. } => {
            *lookback = [20u16, 40, 60][rng.gen_range(0..3)];
        }
        IndicatorExpr::SwingBaseZoneHigh {
            swing_left,
            swing_right,
            base_bars,
            ..
        }
        | IndicatorExpr::SwingBaseZoneLow {
            swing_left,
            swing_right,
            base_bars,
            ..
        } => {
            let swing = [2u16, 3, 4][rng.gen_range(0..3)];
            *swing_left = swing;
            *swing_right = swing;
            *base_bars = [2u16, 3, 4][rng.gen_range(0..3)];
        }
        IndicatorExpr::MacdMain {
            fast_period,
            slow_period,
            ..
        } => {
            nudge_ladder(fast_period, &MACD_FAST, rng);
            nudge_ladder(slow_period, &MACD_SLOW, rng);
            *slow_period = (*slow_period).max(*fast_period + 1);
        }
        IndicatorExpr::MacdSignal {
            fast_period,
            slow_period,
            signal_period,
            ..
        }
        | IndicatorExpr::MacdHistogram {
            fast_period,
            slow_period,
            signal_period,
            ..
        } => {
            nudge_ladder(fast_period, &MACD_FAST, rng);
            nudge_ladder(slow_period, &MACD_SLOW, rng);
            *slow_period = (*slow_period).max(*fast_period + 1);
            nudge_ladder(signal_period, &MACD_SIGNAL, rng);
        }
        IndicatorExpr::BollingerMid { period, .. } => mutate_period(period, rng),
        IndicatorExpr::BollingerUpper {
            period,
            deviation_tenths,
            ..
        }
        | IndicatorExpr::BollingerLower {
            period,
            deviation_tenths,
            ..
        }
        | IndicatorExpr::BollingerBandwidth {
            period,
            deviation_tenths,
            ..
        } => {
            mutate_period(period, rng);
            nudge_ladder(deviation_tenths, &BB_DEVIATION_TENTHS, rng);
        }
        IndicatorExpr::IchimokuTenkan { period, .. } => nudge_ladder(period, &ICHIMOKU_TENKAN, rng),
        IndicatorExpr::IchimokuKijun { period, .. } => nudge_ladder(period, &ICHIMOKU_KIJUN, rng),
        IndicatorExpr::IchimokuSenkouA {
            tenkan_period,
            kijun_period,
            ..
        } => {
            nudge_ladder(tenkan_period, &ICHIMOKU_TENKAN, rng);
            nudge_ladder(kijun_period, &ICHIMOKU_KIJUN, rng);
        }
        IndicatorExpr::IchimokuSenkouB {
            period,
            kijun_period,
            ..
        } => {
            nudge_ladder(period, &ICHIMOKU_SENKOU, rng);
            nudge_ladder(kijun_period, &ICHIMOKU_KIJUN, rng);
        }
        IndicatorExpr::QqeLine {
            rsi_period,
            smoothing_period,
            ..
        } => {
            mutate_period(rsi_period, rng);
            nudge_ladder(smoothing_period, &QQE_SMOOTHING, rng);
        }
        IndicatorExpr::QqeTrail {
            rsi_period,
            smoothing_period,
            factor_tenths,
            ..
        } => {
            mutate_period(rsi_period, rng);
            nudge_ladder(smoothing_period, &QQE_SMOOTHING, rng);
            nudge_ladder(factor_tenths, &QQE_FACTOR_TENTHS, rng);
        }
        IndicatorExpr::Vwap { period, .. } => nudge_ladder(period, &VWAP_PERIODS, rng),
        IndicatorExpr::Cci { period, .. } => mutate_period(period, rng),
    }
}

/// Step one rung along a discrete gene ladder, mirroring `mutate_period` so
/// structural mutation explores neighbours rather than jumping arbitrarily.
fn nudge_ladder(value: &mut u16, ladder: &[u16], rng: &mut ChaCha8Rng) {
    if ladder.is_empty() {
        return;
    }
    let index = ladder
        .iter()
        .position(|candidate| candidate == value)
        .unwrap_or(ladder.len() / 2);
    let delta = rng.gen_range(-1_i32..=1);
    let next = (index as i32 + delta).clamp(0, ladder.len() as i32 - 1) as usize;
    *value = ladder[next];
}

fn mutate_period(period: &mut u16, rng: &mut ChaCha8Rng) {
    let current = snap_period(*period);
    let index = PERIODS
        .iter()
        .position(|&value| value == current)
        .unwrap_or(PERIODS.len() / 2);
    let delta = rng.gen_range(-1_i32..=1);
    let next = (index as i32 + delta).clamp(0, PERIODS.len() as i32 - 1) as usize;
    *period = PERIODS[next];
}

fn classify_expression(expression: &BoolExpr) -> FamilyStyle {
    let serialized = serde_json::to_string(expression).unwrap_or_default();
    if serialized.contains("liquidity_sweep_score") {
        FamilyStyle::SweepReclaim
    } else if serialized.contains("swing_base_zone") {
        FamilyStyle::SupplyDemandReclaim
    } else if serialized.contains("session_range") {
        FamilyStyle::SessionOrb
    } else if serialized.contains("body_range_ratio")
        || serialized.contains("close_location_in_bar")
    {
        FamilyStyle::ImpulseCandle
    } else if serialized.contains("atr_percentile") {
        FamilyStyle::VolSqueezeBreak
    } else if serialized.contains("z_score") {
        FamilyStyle::ZScoreReversion
    } else if serialized.contains("donchian") || serialized.contains("highest") {
        FamilyStyle::DonchianBreakout
    } else if serialized.contains("percentile_in_range") {
        FamilyStyle::MeanReversionBand
    } else if serialized.contains("rsi") || serialized.contains("rate_of_change") {
        FamilyStyle::MomentumBurst
    } else {
        FamilyStyle::TrendPullback
    }
}

fn random_family(rng: &mut ChaCha8Rng) -> SearchFamily {
    SearchFamily::ALL[rng.gen_range(0..SearchFamily::ALL.len())]
}

fn family_name(family: SearchFamily) -> &'static str {
    match family {
        SearchFamily::TrendPullback => "trend_pullback",
        SearchFamily::MomentumBurst => "momentum_burst",
        SearchFamily::DonchianBreakout => "donchian_breakout",
        SearchFamily::MeanReversionBand => "mean_reversion_band",
        SearchFamily::ZScoreReversion => "zscore_reversion",
        SearchFamily::SessionOrb => "session_orb",
        SearchFamily::ImpulseCandle => "impulse_candle",
        SearchFamily::VolSqueezeBreak => "vol_squeeze_break",
        SearchFamily::SupplyDemandReclaim => "supply_demand_reclaim",
        SearchFamily::SweepReclaim => "sweep_reclaim",
        SearchFamily::Universal => "universal",
    }
}

fn choose_period(rng: &mut ChaCha8Rng) -> u16 {
    PERIODS[rng.gen_range(0..PERIODS.len())]
}

fn choose_atr_period(_rng: &mut ChaCha8Rng) -> u16 {
    // Institutional Search Families freeze ATR lookback.
    FROZEN_ATR_PERIOD
}

fn freeze_atr_period(strategy: &mut StrategyIr) {
    if let StopLossPolicy::AtrMultiple { period, .. } = &mut strategy.stops.stop_loss {
        *period = FROZEN_ATR_PERIOD;
    }
    if let TakeProfitPolicy::AtrMultiple { period, .. } = &mut strategy.stops.take_profit {
        *period = FROZEN_ATR_PERIOD;
    }
    match &mut strategy.entry.order {
        EntryOrderPolicy::Market => {}
        EntryOrderPolicy::Stop { distance, .. } | EntryOrderPolicy::Limit { distance, .. } => {
            if let EntryDistancePolicy::AtrMultiple { period, .. } = distance {
                *period = FROZEN_ATR_PERIOD;
            }
        }
    }
    if let Some(TrailingPolicy::AtrMultiple { period, .. }) = &mut strategy.manage.trailing {
        *period = FROZEN_ATR_PERIOD;
    }
}

fn choose_from(rng: &mut ChaCha8Rng, ladder: &[f64]) -> f64 {
    ladder[rng.gen_range(0..ladder.len())]
}

fn snap_period(period: u16) -> u16 {
    PERIODS
        .iter()
        .copied()
        .min_by_key(|&candidate| (i32::from(candidate) - i32::from(period)).unsigned_abs())
        .unwrap_or(14)
}

fn snap_to_ladder(value: f64, ladder: &[f64]) -> f64 {
    ladder
        .iter()
        .copied()
        .min_by(|left, right| {
            (left - value)
                .abs()
                .partial_cmp(&(right - value).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(value)
}

fn snap_quarter(value: f64) -> f64 {
    (value * 4.0).round() / 4.0
}

fn mutate_ladder_value(value: f64, rng: &mut ChaCha8Rng, ladder: &[f64]) -> f64 {
    let snapped = snap_to_ladder(value, ladder);
    let index = ladder
        .iter()
        .position(|&step| (step - snapped).abs() < 1.0e-9)
        .unwrap_or(ladder.len() / 2);
    let delta = rng.gen_range(-1_i32..=1);
    let next = (index as i32 + delta).clamp(0, ladder.len() as i32 - 1) as usize;
    ladder[next]
}

fn enforce_atr_relative_policies(strategy: &mut StrategyIr) {
    strategy.stops.stop_loss = match &strategy.stops.stop_loss {
        StopLossPolicy::AtrMultiple { multiplier, .. } => StopLossPolicy::AtrMultiple {
            period: FROZEN_ATR_PERIOD,
            multiplier: snap_to_ladder(*multiplier, &ATR_STOP_MULTIPLIERS),
        },
        StopLossPolicy::FixedPoints { .. } | StopLossPolicy::RangeMultiple { .. } => {
            StopLossPolicy::AtrMultiple {
                period: FROZEN_ATR_PERIOD,
                multiplier: 2.0,
            }
        }
    };
    strategy.stops.take_profit = match &strategy.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => TakeProfitPolicy::RiskMultiple {
            multiple: snap_to_ladder(*multiple, &RISK_MULTIPLES),
        },
        TakeProfitPolicy::AtrMultiple { multiplier, .. } => TakeProfitPolicy::AtrMultiple {
            period: FROZEN_ATR_PERIOD,
            multiplier: snap_to_ladder(*multiplier, &ATR_TP_MULTIPLIERS),
        },
        TakeProfitPolicy::FixedPoints { .. } => TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
    };
    match &mut strategy.entry.order {
        EntryOrderPolicy::Market => {}
        EntryOrderPolicy::Stop { distance, .. } | EntryOrderPolicy::Limit { distance, .. } => {
            *distance = match &*distance {
                EntryDistancePolicy::AtrMultiple { period, multiplier } => {
                    EntryDistancePolicy::AtrMultiple {
                        period: snap_period(*period).clamp(8, 20),
                        multiplier: snap_to_ladder(*multiplier, &ATR_ENTRY_MULTIPLIERS),
                    }
                }
                EntryDistancePolicy::FixedPoints { .. }
                | EntryDistancePolicy::RangeMultiple { .. } => EntryDistancePolicy::AtrMultiple {
                    period: 14,
                    multiplier: 0.5,
                },
            };
        }
    }
    if let Some(value) = &mut strategy.manage.break_even_at_r {
        *value = snap_to_ladder(*value, &R_ACTIVATE);
    }
    if let Some(trailing) = &mut strategy.manage.trailing {
        *trailing = match trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => TrailingPolicy::RiskMultiple {
                activate_at_r: snap_to_ladder(*activate_at_r, &R_ACTIVATE),
                distance_r: snap_to_ladder(*distance_r, &R_DISTANCE),
            },
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                period,
                multiplier,
            } => TrailingPolicy::AtrMultiple {
                activate_at_r: snap_to_ladder(*activate_at_r, &R_ACTIVATE),
                period: snap_period(*period).clamp(8, 20),
                multiplier: snap_to_ladder(*multiplier, &ATR_TRAIL_MULTIPLIERS),
            },
        };
    }
}

fn normalize(strategy: &mut StrategyIr) {
    strategy.risk = RiskPolicy::FixedCurrency {
        amount: crate::FIXED_RISK_PER_TRADE,
    };
    enforce_atr_relative_policies(strategy);
    if let Ok(canonical) = strategy.canonicalized(FloatPolicy::default()) {
        *strategy = canonical;
    } else {
        strategy.meta.complexity = strategy.complexity().score.min(u16::MAX as usize) as u16;
    }
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut result = value;
    result = (result ^ (result >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    result = (result ^ (result >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    result ^ (result >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_ir::IrLimits;

    #[test]
    fn generation_is_reproducible_and_spans_all_seed_families() {
        let first: Vec<_> = (0..SearchFamily::ALL.len() as u64)
            .map(|index| generate_seed(42, index))
            .collect();
        let second: Vec<_> = (0..SearchFamily::ALL.len() as u64)
            .map(|index| generate_seed(42, index))
            .collect();
        assert_eq!(first, second);
        for (index, family) in SearchFamily::ALL.iter().enumerate() {
            assert_eq!(classify_family(&first[index]), family.style());
        }
        for strategy in first {
            assert_eq!(
                strategy.risk,
                RiskPolicy::FixedCurrency {
                    amount: crate::FIXED_RISK_PER_TRADE
                }
            );
            strategy.validate_export_safe(IrLimits::default()).unwrap();
        }
    }

    #[test]
    fn oversized_children_are_trimmed_until_the_ir_validates() {
        let limits = IrLimits::default();
        let mut oversized = generate_seed(11, 2);
        // Stack mirrored entry blocks until the node budget is blown, the way a
        // crossover between two elaborate parents does.
        let long = oversized.entry.long.clone().expect("seed has a long entry");
        let short = oversized.entry.short.clone();
        let mut long_children = vec![long.clone(); 8];
        long_children.push(long);
        oversized.entry.long = Some(BoolExpr::And {
            children: long_children,
        });
        if let Some(short) = short {
            oversized.entry.short = Some(BoolExpr::And {
                children: vec![short; 9],
            });
        }
        assert!(
            oversized.validate_export_safe(limits).is_err(),
            "fixture should exceed the node budget"
        );

        let fitted = fit_within_ir_limits(oversized, 2).expect("trimming should converge");
        fitted
            .validate_export_safe(limits)
            .expect("trimmed child must validate");
        assert!(
            entry_condition_count(&fitted) >= 2,
            "trimming must respect the minimum entry condition floor"
        );
    }

    #[test]
    fn already_valid_strategies_pass_through_trimming_untouched() {
        let seed = generate_seed(11, 2);
        let fitted = fit_within_ir_limits(seed.clone(), 2).expect("valid seed is returned");
        assert_eq!(fitted, seed);
    }

    #[test]
    fn mutation_remains_valid_and_deterministic() {
        let seed = generate_seed(7, 3);
        let first = mutate_strategy(&seed, 7, 99, 1.0);
        let second = mutate_strategy(&seed, 7, 99, 1.0);
        assert_eq!(first, second);
        assert_eq!(
            first.risk,
            RiskPolicy::FixedCurrency {
                amount: crate::FIXED_RISK_PER_TRADE
            }
        );
        first.validate_export_safe(IrLimits::default()).unwrap();
    }

    #[test]
    fn locked_mutation_cannot_leave_search_family() {
        let seed = generate_seed_for_family(11, 0, SearchFamily::DonchianBreakout);
        for sequence in 0..40 {
            let mut rng = rng_for(11, 3, sequence);
            let child = mutate_with_rng(
                &seed,
                &mut rng,
                1.0,
                sequence,
                false,
                SearchFamily::DonchianBreakout,
                &UniversalGrammarConfig::default(),
            );
            assert_eq!(classify_family(&child), FamilyStyle::DonchianBreakout);
            assert!(matches!(
                child.stops.stop_loss,
                StopLossPolicy::AtrMultiple {
                    period: FROZEN_ATR_PERIOD,
                    ..
                }
            ));
            assert!(matches!(
                child.entry.order,
                EntryOrderPolicy::Stop { .. } | EntryOrderPolicy::Limit { .. }
            ));
        }
    }

    #[test]
    fn institutional_seeds_are_pending_only_with_frozen_atr() {
        let population: Vec<_> = (0..64)
            .map(|index| generate_seed_for_family(91, index, SearchFamily::TrendPullback))
            .collect();
        assert!(population.iter().all(|value| {
            matches!(
                value.entry.order,
                EntryOrderPolicy::Stop { .. } | EntryOrderPolicy::Limit { .. }
            ) && value.side == Side::Both
                && value.manage.trailing.is_none()
                && value.manage.break_even_at_r.is_none()
                && value.manage.partial_exits.is_empty()
                && matches!(
                    value.stops.stop_loss,
                    StopLossPolicy::AtrMultiple {
                        period: FROZEN_ATR_PERIOD,
                        ..
                    }
                )
        }));
        let stops = population
            .iter()
            .filter(|value| matches!(value.entry.order, EntryOrderPolicy::Stop { .. }))
            .count();
        let limits = population
            .iter()
            .filter(|value| matches!(value.entry.order, EntryOrderPolicy::Limit { .. }))
            .count();
        assert!(stops > 0 && limits > 0, "both pending kinds should appear");
        for strategy in &population {
            assert_eq!(classify_family(strategy), FamilyStyle::TrendPullback);
            assert_eq!(
                strategy.risk,
                RiskPolicy::FixedCurrency {
                    amount: crate::FIXED_RISK_PER_TRADE
                }
            );
            strategy.validate_export_safe(IrLimits::default()).unwrap();
        }
    }

    #[test]
    fn generated_strategies_never_use_fixed_point_distances() {
        let population: Vec<_> = (0..400).map(|index| generate_seed(23, index)).collect();
        for strategy in &population {
            assert!(
                matches!(strategy.stops.stop_loss, StopLossPolicy::AtrMultiple { .. }),
                "stop_loss must be ATR multiple, got {:?}",
                strategy.stops.stop_loss
            );
            assert!(
                !matches!(
                    strategy.stops.take_profit,
                    TakeProfitPolicy::FixedPoints { .. }
                ),
                "take_profit must not be FixedPoints"
            );
            match &strategy.entry.order {
                EntryOrderPolicy::Market => {}
                EntryOrderPolicy::Stop { distance, .. }
                | EntryOrderPolicy::Limit { distance, .. } => {
                    assert!(
                        matches!(distance, EntryDistancePolicy::AtrMultiple { .. }),
                        "entry distance must be ATR multiple, got {distance:?}"
                    );
                }
            }
            if let StopLossPolicy::AtrMultiple { multiplier, .. } = strategy.stops.stop_loss {
                let steps = (multiplier * 4.0).round() / 4.0;
                assert!(
                    (multiplier - steps).abs() < 1.0e-9,
                    "ATR stop multiplier {multiplier} is not on 0.25 ladder"
                );
            }
        }
    }

    #[test]
    fn entry_signals_use_one_to_three_family_atoms() {
        let population: Vec<_> = (0..400).map(|index| generate_seed(17, index)).collect();
        let mut saw_single = false;
        let mut saw_and = false;
        let mut max_children = 0usize;
        for strategy in &population {
            let Some(entry) = strategy
                .entry
                .long
                .as_ref()
                .or(strategy.entry.short.as_ref())
            else {
                continue;
            };
            match entry {
                BoolExpr::And { children } => {
                    saw_and = true;
                    max_children = max_children.max(children.len());
                    assert!((2..=4).contains(&children.len()));
                }
                BoolExpr::Compare { .. }
                | BoolExpr::CrossAbove { .. }
                | BoolExpr::CrossBelow { .. } => {
                    saw_single = true;
                }
                _ => {}
            }
        }
        assert!(saw_single, "expected some single-atom entries");
        assert!(saw_and, "expected some And-combined entries");
        assert!(
            max_children >= 2,
            "expected multi-atom And entries, max={max_children}"
        );
    }

    #[test]
    fn universal_grammar_builds_bounded_entries_exits_and_closed_bar_shifts() {
        fn collect_shifts(value: &serde_json::Value, shifts: &mut Vec<u64>) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (name, value) in fields {
                        if name == "shift" {
                            shifts.push(value.as_u64().expect("shift is an integer"));
                        } else {
                            collect_shifts(value, shifts);
                        }
                    }
                }
                serde_json::Value::Array(values) => {
                    for value in values {
                        collect_shifts(value, shifts);
                    }
                }
                _ => {}
            }
        }

        let mut saw_two_entries = false;
        let mut saw_three_entries = false;
        let mut saw_one_exit = false;
        let mut saw_three_exits = false;
        for sequence in 0..200 {
            let strategy = generate_seed_for_family(91, sequence, SearchFamily::Universal);
            assert_eq!(classify_family(&strategy), FamilyStyle::Universal);
            // Component atoms can themselves be composite, so the configured bound
            // is on top-level condition blocks, not on leaf predicates.
            let entry_count = entry_condition_count(&strategy);
            let exit_count = exit_condition_count(&strategy);
            assert!((2..=4).contains(&entry_count));
            assert!((1..=3).contains(&exit_count));
            saw_two_entries |= entry_count == 2;
            saw_three_entries |= entry_count == 3;
            saw_one_exit |= exit_count == 1;
            saw_three_exits |= exit_count == 3;

            let mut shifts = Vec::new();
            collect_shifts(
                &serde_json::to_value(&strategy).expect("strategy serializes"),
                &mut shifts,
            );
            assert!(!shifts.is_empty());
            assert!(
                shifts.iter().all(|shift| (1..=3).contains(shift)),
                "universal shifts escaped the configured closed-bar range: {shifts:?}"
            );
            strategy.validate_export_safe(IrLimits::default()).unwrap();
        }
        assert!(saw_two_entries && saw_three_entries);
        assert!(saw_one_exit && saw_three_exits);
    }

    #[test]
    fn universal_grammar_honors_a_sealed_custom_contract() {
        let contract = UniversalGrammarConfig {
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 2,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 1,
            minimum_shift: 2,
            maximum_shift: 2,
        };
        let mut rng = rng_for(7, 0, 0);
        let strategy = build_seed(
            SearchFamily::Universal,
            &mut rng,
            "universal-contract".into(),
            3,
            true,
            true,
            &contract,
        );
        assert_eq!(entry_atom_count(strategy.entry.long.as_ref().unwrap()), 2);
        assert_eq!(entry_atom_count(strategy.exit_long.as_ref().unwrap()), 1);
        assert!(matches!(strategy.entry.order, EntryOrderPolicy::Market));
        strategy.validate_export_safe(IrLimits::default()).unwrap();
    }
}
