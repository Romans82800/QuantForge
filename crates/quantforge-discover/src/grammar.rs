use crate::model::FamilyStyle;
use quantforge_core::{FloatPolicy, STRATEGY_IR_VERSION};
use quantforge_ir::{
    BoolExpr, ComparisonOp, ContextValue, EntryDistancePolicy, EntryOrderPolicy, EntrySignals,
    IndicatorExpr, ManagePolicy, NumericExpr, PartialExit, PriceField, ProtectiveStops, RiskPolicy,
    Side, StopLossPolicy, StrategyIr, StrategyMeta, TakeProfitPolicy, TrailingPolicy,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

const PERIODS: [u16; 12] = [5, 8, 10, 12, 14, 20, 30, 40, 50, 64, 80, 100];

pub fn generate_seed(seed: u64, sequence: u64) -> StrategyIr {
    let mut rng = rng_for(seed, 0, sequence);
    let family = match sequence % 4 {
        0 => FamilyStyle::Trend,
        1 => FamilyStyle::Momentum,
        2 => FamilyStyle::Breakout,
        _ => FamilyStyle::MeanReversion,
    };
    build_seed(family, &mut rng, format!("seed-{sequence}"))
}

pub fn mutate_strategy(
    strategy: &StrategyIr,
    seed: u64,
    sequence: u64,
    structural_probability: f64,
) -> StrategyIr {
    let mut rng = rng_for(seed, 1, sequence);
    mutate_with_rng(strategy, &mut rng, structural_probability, sequence)
}

pub(crate) fn rng_for(seed: u64, stream: u64, sequence: u64) -> ChaCha8Rng {
    let mixed = splitmix64(
        seed ^ stream.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ sequence.wrapping_mul(0xbf58_476d_1ce4_e5b9),
    );
    ChaCha8Rng::seed_from_u64(mixed)
}

pub(crate) fn build_seed(family: FamilyStyle, rng: &mut ChaCha8Rng, id: String) -> StrategyIr {
    let side = match rng.gen_range(0..4) {
        0 => Side::LongOnly,
        1 => Side::ShortOnly,
        _ => Side::Both,
    };
    let (long, short) = family_entries(family, rng);
    let order = random_entry_order(rng);
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
        manage: random_manage(rng),
        meta: StrategyMeta {
            thesis_hint: family_name(family).into(),
            complexity: 0,
            export_safe: true,
        },
    };
    normalize(&mut strategy);
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
) -> StrategyIr {
    let mut child = strategy.clone();
    child.id = format!("candidate-{sequence}");

    if let Some(entry) = &mut child.entry.long {
        mutate_bool(entry, rng);
    }
    if let Some(entry) = &mut child.entry.short {
        mutate_bool(entry, rng);
    }
    for filter in &mut child.filters {
        mutate_bool(filter, rng);
    }
    mutate_policies(&mut child, rng);

    if rng.gen_bool(structural_probability) {
        match rng.gen_range(0..6) {
            0 => {
                let family = random_family(rng);
                let (long, short) = family_entries(family, rng);
                child.meta.thesis_hint = family_name(family).into();
                child.entry = match child.side {
                    Side::LongOnly => EntrySignals {
                        long: Some(long),
                        short: None,
                        order: child.entry.order.clone(),
                    },
                    Side::ShortOnly => EntrySignals {
                        long: None,
                        short: Some(short),
                        order: child.entry.order.clone(),
                    },
                    Side::Both => EntrySignals {
                        long: Some(long),
                        short: Some(short),
                        order: child.entry.order.clone(),
                    },
                };
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
                let family = classify_family(&child);
                let (long, short) = family_entries(family, rng);
                child.side = match rng.gen_range(0..3) {
                    0 => Side::LongOnly,
                    1 => Side::ShortOnly,
                    _ => Side::Both,
                };
                child.entry = match child.side {
                    Side::LongOnly => EntrySignals {
                        long: Some(long),
                        short: None,
                        order: child.entry.order.clone(),
                    },
                    Side::ShortOnly => EntrySignals {
                        long: None,
                        short: Some(short),
                        order: child.entry.order.clone(),
                    },
                    Side::Both => EntrySignals {
                        long: Some(long),
                        short: Some(short),
                        order: child.entry.order.clone(),
                    },
                };
            }
            4 => child.entry.order = random_entry_order(rng),
            _ => child.manage = random_manage(rng),
        }
    }
    normalize(&mut child);
    child
}

pub(crate) fn classify_family(strategy: &StrategyIr) -> FamilyStyle {
    match strategy.meta.thesis_hint.as_str() {
        "trend" => FamilyStyle::Trend,
        "momentum" => FamilyStyle::Momentum,
        "breakout" => FamilyStyle::Breakout,
        "mean_reversion" => FamilyStyle::MeanReversion,
        _ => strategy
            .entry
            .long
            .as_ref()
            .or(strategy.entry.short.as_ref())
            .map(classify_expression)
            .unwrap_or(FamilyStyle::Trend),
    }
}

fn family_entries(family: FamilyStyle, rng: &mut ChaCha8Rng) -> (BoolExpr, BoolExpr) {
    // Each family has 4 atom pairs (original + 3 extras). Pick 1..=3 and AND them.
    let atoms = family_entry_atoms(family, rng);
    let count = rng.gen_range(1..=atoms.len().min(3));
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

/// (long_condition, short_condition) atoms available inside a family.
fn family_entry_atoms(
    family: FamilyStyle,
    rng: &mut ChaCha8Rng,
) -> Vec<(BoolExpr, BoolExpr)> {
    match family {
        FamilyStyle::Trend => {
            let fast_index = rng.gen_range(0..6);
            let slow_index = rng.gen_range((fast_index + 2)..PERIODS.len());
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
            ]
        }
        FamilyStyle::Momentum => {
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
                        NumericExpr::Constant {
                            value: -roc_level,
                        },
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
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        sma(sma_period, 1),
                    ),
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                ),
            ]
        }
        FamilyStyle::Breakout => {
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
                            value: IndicatorExpr::DonchianHigh {
                                period,
                                shift: 2,
                            },
                        },
                    ),
                    compare(
                        ComparisonOp::LessThan,
                        close(1),
                        NumericExpr::Indicator {
                            value: IndicatorExpr::DonchianLow {
                                period,
                                shift: 2,
                            },
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
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        sma(sma_period, 1),
                    ),
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
            ]
        }
        FamilyStyle::MeanReversion => {
            let period = choose_period(rng);
            let lower = rng.gen_range(20.0..=40.0);
            let upper = 100.0 - lower;
            let z_period = choose_period(rng);
            let z_level = rng.gen_range(1.0..=2.5);
            let pct_period = choose_period(rng).max(10);
            let pct_low = rng.gen_range(5.0..=25.0);
            let pct_high = 100.0 - pct_low;
            let sma_period = choose_period(rng);
            vec![
                // Original: RSI extremes
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
                // +1 Z-score
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
                // +2 Percentile in range
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
                // +3 Close below / above SMA
                (
                    compare(ComparisonOp::LessThan, close(1), sma(sma_period, 1)),
                    compare(
                        ComparisonOp::GreaterThan,
                        close(1),
                        sma(sma_period, 1),
                    ),
                ),
            ]
        }
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
    let stop_loss = match rng.gen_range(0..3) {
        0 => StopLossPolicy::FixedPoints {
            points: rng.gen_range(50.0..=300.0),
        },
        1 => StopLossPolicy::AtrMultiple {
            period: choose_period(rng).min(30),
            multiplier: rng.gen_range(1.0..=3.0),
        },
        _ => StopLossPolicy::RangeMultiple {
            period: choose_period(rng).min(30),
            multiplier: rng.gen_range(1.0..=3.0),
        },
    };
    let take_profit = if rng.gen_bool(0.75) {
        TakeProfitPolicy::RiskMultiple {
            multiple: rng.gen_range(1.0..=3.5),
        }
    } else {
        TakeProfitPolicy::AtrMultiple {
            period: choose_period(rng).min(30),
            multiplier: rng.gen_range(1.0..=4.0),
        }
    };
    ProtectiveStops {
        stop_loss,
        take_profit,
    }
}

fn random_entry_distance(rng: &mut ChaCha8Rng) -> EntryDistancePolicy {
    match rng.gen_range(0..3) {
        0 => EntryDistancePolicy::FixedPoints {
            points: rng.gen_range(10.0..=250.0),
        },
        1 => EntryDistancePolicy::AtrMultiple {
            period: choose_period(rng).min(30),
            multiplier: rng.gen_range(0.15..=1.5),
        },
        _ => EntryDistancePolicy::RangeMultiple {
            period: choose_period(rng).min(30),
            multiplier: rng.gen_range(0.15..=1.5),
        },
    }
}

fn random_entry_order(rng: &mut ChaCha8Rng) -> EntryOrderPolicy {
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
                activate_at_r: rng.gen_range(0.75..=2.0),
                distance_r: rng.gen_range(0.4..=1.5),
            }
        } else {
            TrailingPolicy::AtrMultiple {
                activate_at_r: rng.gen_range(0.75..=2.0),
                period: choose_period(rng).min(30),
                multiplier: rng.gen_range(0.5..=2.5),
            }
        }
    });
    ManagePolicy {
        break_even_at_r: rng.gen_bool(0.3).then(|| rng.gen_range(0.5..=2.0)),
        trailing,
        time_stop_bars: rng.gen_bool(0.45).then(|| rng.gen_range(6..=80)),
        partial_exits,
        // Production applies these as immutable job policies, never genes.
        flatten_end_of_day: false,
        max_one_entry_per_day: false,
    }
}

fn mutate_policies(strategy: &mut StrategyIr, rng: &mut ChaCha8Rng) {
    match &mut strategy.stops.stop_loss {
        StopLossPolicy::FixedPoints { points } => {
            *points = (*points * rng.gen_range(0.75..=1.25)).clamp(10.0, 1_000.0);
        }
        StopLossPolicy::AtrMultiple { period, multiplier }
        | StopLossPolicy::RangeMultiple { period, multiplier } => {
            mutate_period(period, rng);
            *multiplier = (*multiplier * rng.gen_range(0.8..=1.2)).clamp(0.5, 6.0);
        }
    }
    match &mut strategy.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => {
            *multiple = (*multiple * rng.gen_range(0.8..=1.2)).clamp(0.5, 6.0);
        }
        TakeProfitPolicy::FixedPoints { points } => {
            *points = (*points * rng.gen_range(0.75..=1.25)).clamp(10.0, 1_000.0);
        }
        TakeProfitPolicy::AtrMultiple { period, multiplier } => {
            mutate_period(period, rng);
            *multiplier = (*multiplier * rng.gen_range(0.8..=1.2)).clamp(0.5, 8.0);
        }
    }
    if let Some(bars) = &mut strategy.manage.time_stop_bars {
        let delta = rng.gen_range(-4_i32..=4);
        *bars = (i32::from(*bars) + delta).clamp(2, 500) as u16;
    }
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
            mutate_entry_distance(distance, rng);
            let delta = rng.gen_range(-2_i32..=2);
            *expiry_bars = (i32::from(*expiry_bars) + delta).clamp(1, 100) as u16;
        }
    }
    if let Some(value) = &mut strategy.manage.break_even_at_r {
        *value = (*value * rng.gen_range(0.8..=1.2)).clamp(0.1, 5.0);
    }
    if let Some(trailing) = &mut strategy.manage.trailing {
        match trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => {
                *activate_at_r = (*activate_at_r * rng.gen_range(0.8..=1.2)).clamp(0.1, 5.0);
                *distance_r = (*distance_r * rng.gen_range(0.8..=1.2)).clamp(0.1, 5.0);
            }
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                period,
                multiplier,
            } => {
                *activate_at_r = (*activate_at_r * rng.gen_range(0.8..=1.2)).clamp(0.1, 5.0);
                mutate_period(period, rng);
                *multiplier = (*multiplier * rng.gen_range(0.8..=1.2)).clamp(0.1, 8.0);
            }
        }
    }
    for partial in &mut strategy.manage.partial_exits {
        partial.at_r = (partial.at_r * rng.gen_range(0.8..=1.2)).clamp(0.1, 8.0);
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
    match distance {
        EntryDistancePolicy::FixedPoints { points } => {
            *points = (*points * rng.gen_range(0.75..=1.25)).clamp(1.0, 2_000.0);
        }
        EntryDistancePolicy::AtrMultiple { period, multiplier }
        | EntryDistancePolicy::RangeMultiple { period, multiplier } => {
            mutate_period(period, rng);
            *multiplier = (*multiplier * rng.gen_range(0.8..=1.2)).clamp(0.05, 8.0);
        }
    }
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
    let period = match indicator {
        IndicatorExpr::Sma { period, .. }
        | IndicatorExpr::Ema { period, .. }
        | IndicatorExpr::Wma { period, .. }
        | IndicatorExpr::Rsi { period, .. }
        | IndicatorExpr::Atr { period, .. }
        | IndicatorExpr::DonchianHigh { period, .. }
        | IndicatorExpr::DonchianLow { period, .. }
        | IndicatorExpr::Highest { period, .. }
        | IndicatorExpr::Lowest { period, .. }
        | IndicatorExpr::StandardDeviation { period, .. }
        | IndicatorExpr::ZScore { period, .. }
        | IndicatorExpr::PercentileInRange { period, .. }
        | IndicatorExpr::RateOfChange { period, .. } => period,
    };
    mutate_period(period, rng);
}

fn mutate_period(period: &mut u16, rng: &mut ChaCha8Rng) {
    let delta = rng.gen_range(-5_i32..=5);
    *period = (i32::from(*period) + delta).clamp(2, 500) as u16;
}

fn classify_expression(expression: &BoolExpr) -> FamilyStyle {
    let serialized = serde_json::to_string(expression).unwrap_or_default();
    if serialized.contains("donchian") || serialized.contains("highest") {
        FamilyStyle::Breakout
    } else if serialized.contains("z_score") || serialized.contains("percentile_in_range") {
        FamilyStyle::MeanReversion
    } else if serialized.contains("rsi") || serialized.contains("rate_of_change") {
        FamilyStyle::Momentum
    } else {
        FamilyStyle::Trend
    }
}

fn random_family(rng: &mut ChaCha8Rng) -> FamilyStyle {
    match rng.gen_range(0..4) {
        0 => FamilyStyle::Trend,
        1 => FamilyStyle::Momentum,
        2 => FamilyStyle::Breakout,
        _ => FamilyStyle::MeanReversion,
    }
}

fn family_name(family: FamilyStyle) -> &'static str {
    match family {
        FamilyStyle::Trend => "trend",
        FamilyStyle::Momentum => "momentum",
        FamilyStyle::Breakout => "breakout",
        FamilyStyle::MeanReversion => "mean_reversion",
    }
}

fn choose_period(rng: &mut ChaCha8Rng) -> u16 {
    PERIODS[rng.gen_range(0..PERIODS.len())]
}

fn normalize(strategy: &mut StrategyIr) {
    strategy.risk = RiskPolicy::FixedCurrency {
        amount: crate::FIXED_RISK_PER_TRADE,
    };
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
        let first: Vec<_> = (0..8).map(|index| generate_seed(42, index)).collect();
        let second: Vec<_> = (0..8).map(|index| generate_seed(42, index)).collect();
        assert_eq!(first, second);
        assert_eq!(classify_family(&first[0]), FamilyStyle::Trend);
        assert_eq!(classify_family(&first[1]), FamilyStyle::Momentum);
        assert_eq!(classify_family(&first[2]), FamilyStyle::Breakout);
        assert_eq!(classify_family(&first[3]), FamilyStyle::MeanReversion);
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
    fn searchable_population_contains_every_entry_and_management_gene() {
        let population: Vec<_> = (0..256).map(|index| generate_seed(91, index)).collect();
        assert!(
            population
                .iter()
                .any(|value| matches!(value.entry.order, EntryOrderPolicy::Market))
        );
        assert!(
            population
                .iter()
                .any(|value| matches!(value.entry.order, EntryOrderPolicy::Stop { .. }))
        );
        assert!(
            population
                .iter()
                .any(|value| matches!(value.entry.order, EntryOrderPolicy::Limit { .. }))
        );
        assert!(
            population
                .iter()
                .any(|value| value.manage.break_even_at_r.is_some())
        );
        assert!(
            population
                .iter()
                .any(|value| value.manage.trailing.is_some())
        );
        assert!(
            population
                .iter()
                .any(|value| !value.manage.partial_exits.is_empty())
        );
        assert!(
            population
                .iter()
                .all(|value| !value.manage.flatten_end_of_day)
        );
        assert!(
            population
                .iter()
                .all(|value| !value.manage.max_one_entry_per_day)
        );
        for strategy in population {
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
    fn entry_signals_use_one_to_three_family_atoms() {
        let population: Vec<_> = (0..400).map(|index| generate_seed(17, index)).collect();
        let mut saw_single = false;
        let mut saw_and = false;
        let mut max_children = 0usize;
        for strategy in &population {
            let Some(entry) = strategy.entry.long.as_ref().or(strategy.entry.short.as_ref()) else {
                continue;
            };
            match entry {
                BoolExpr::And { children } => {
                    saw_and = true;
                    max_children = max_children.max(children.len());
                    assert!((2..=3).contains(&children.len()));
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
}
