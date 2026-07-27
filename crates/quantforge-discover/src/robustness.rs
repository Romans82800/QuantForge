//! M1 robustness battery on IS (before OOS1 pick) for Discover databank promotion.

use chrono::Datelike;
use quantforge_broker::{BrokerClock, SymbolSpecification};
use quantforge_core::FloatPolicy;
use quantforge_data::{BarDataset, bar_content_hash, infer_median_interval_ms};
use quantforge_ir::{BoolExpr, IndicatorExpr, NumericExpr, StrategyIr};
use quantforge_quality::{monte_carlo_from_trade_profits, perturb_strategy_parameters};
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};
use quantforge_eval::{ScoutResult, ScoutTelemetry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RobustnessReject {
    M1Fidelity,
    WalkForward,
    MonteCarlo,
    ParamNeighborhood,
}

pub(crate) struct RobustnessConfig {
    pub folds: usize,
    pub monte_carlo_trials: usize,
    pub neighborhood_samples: usize,
    pub seed: u64,
    pub initial_balance: f64,
    pub costs: quantforge_eval::CostModel,
    /// Required M1 return as a fraction of the Selected-TF return.
    pub minimum_return_retention: f64,
    pub minimum_fold_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub minimum_passing_fold_fraction: f64,
    pub minimum_neighborhood_survival_fraction: f64,
    pub parameter_perturbation_fraction: f64,
    /// Search-profile bounds used by the dedicated ADX plateau check.
    pub adx_period_min: u16,
    pub adx_period_max: u16,
    pub adx_period_step: u16,
    pub adx_threshold_min: f64,
    pub adx_threshold_max: f64,
    pub adx_threshold_step: f64,
    /// When true, folds are broker-local calendar years and every year must pass.
    pub calendar_year_folds: bool,
}

/// SQX-style RetestWithHigherPrecision defaults retained for trade count and
/// drawdown. QuantForge makes return retention configurable and defaults it to
/// 90% for promotion-grade databanks.
pub(crate) const SQX_TRADE_RETENTION: f64 = 0.80;
pub(crate) const SQX_DRAWDOWN_EXPANSION: f64 = 1.30;

/// M1 baseline → SQX retention vs H1 → WFO/MC/params.
pub(crate) fn run_m1_predeposit_robustness(
    strategy: &StrategyIr,
    is_decision: &BarDataset,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &RobustnessConfig,
    h1_metrics: &quantforge_eval::BacktestMetrics,
) -> Result<ScoutResult, RobustnessReject> {
    let judge = JudgeConfig {
        initial_balance: config.initial_balance,
        costs: config.costs.clone(),
        allow_execution_gaps: true,
    };
    let baseline = evaluate_strategy_m1(strategy, is_decision, m1_dataset, broker, &judge)
        .map_err(|_| RobustnessReject::M1Fidelity)?;
    // This is deliberately the result that leaves the robustness battery.  The
    // selected-timeframe run is a scout; the databank must retain the exact M1
    // chronology, equity path and metrics that were actually admitted.
    let baseline_result = ScoutResult {
        trades: baseline.trades.clone(),
        equity: baseline.equity.clone(),
        metrics: baseline.metrics.clone(),
        telemetry: ScoutTelemetry::default(),
    };
    // SQX-style: M1 must retain Selected-TF results, not re-clear absolute deposit gates.
    if !passes_sqx_m1_retention(
        h1_metrics,
        &baseline.metrics,
        config.minimum_return_retention,
    ) {
        return Err(RobustnessReject::M1Fidelity);
    }

    let folds = if config.calendar_year_folds {
        calendar_year_fold_ranges(is_decision, &broker.timezone)
            .map_err(|_| RobustnessReject::WalkForward)?
    } else {
        contiguous_fold_ranges(is_decision.bars.len(), config.folds)
    };
    if folds.is_empty() {
        return Err(RobustnessReject::WalkForward);
    }

    let mut passing_folds = 0usize;
    for (start, end) in &folds {
        if *end <= *start + 1 {
            continue;
        }
        let lookback = 120usize;
        let slice_start = start.saturating_sub(lookback);
        let decision_slice = slice_dataset(is_decision, slice_start, *end);
        let start_ms = is_decision.bars[*start].timestamp_ms;
        let last_open_ms = is_decision.bars[*end - 1].timestamp_ms;
        let interval_ms = infer_median_interval_ms(&is_decision.bars).unwrap_or(3_600_000);
        let end_exclusive_ms = last_open_ms.saturating_add(interval_ms);
        let m1_slice = slice_m1_covering(m1_dataset, start_ms, end_exclusive_ms);
        let fold_result =
            evaluate_strategy_m1(strategy, &decision_slice, &m1_slice, broker, &judge)
                .map_err(|_| RobustnessReject::WalkForward)?;
        let fold_trades = fold_result
            .trades
            .iter()
            .filter(|trade| {
                trade.entry_timestamp_ms >= start_ms && trade.entry_timestamp_ms <= last_open_ms
            })
            .count();
        if fold_trades >= config.minimum_fold_trades
            && fold_result.metrics.return_percent > config.minimum_return_percent
            && effective_pf(&fold_result.metrics) >= config.minimum_profit_factor
            && fold_result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent
        {
            passing_folds += 1;
        }
    }
    let required_fraction = if config.calendar_year_folds {
        1.0
    } else {
        config.minimum_passing_fold_fraction
    };
    let fold_fraction = passing_folds as f64 / folds.len().max(1) as f64;
    if fold_fraction + 1e-12 < required_fraction {
        return Err(RobustnessReject::WalkForward);
    }

    let profits: Vec<_> = baseline
        .trades
        .iter()
        .map(|trade| trade.net_profit)
        .collect();
    let mc = monte_carlo_from_trade_profits(
        &profits,
        config.initial_balance,
        config.monte_carlo_trials,
        5,
        config.seed,
        0.0,
        config.maximum_drawdown_percent.max(35.0),
    );
    if !mc.passed || mc.median_net_profit < 0.0 {
        return Err(RobustnessReject::MonteCarlo);
    }

    let mut surviving = 0usize;
    for sample in 0..config.neighborhood_samples {
        let Ok(neighbor) = perturb_strategy_parameters(
            strategy,
            config.parameter_perturbation_fraction,
            sample,
            config.seed,
        ) else {
            continue;
        };
        let Ok(result) = evaluate_strategy_m1(&neighbor, is_decision, m1_dataset, broker, &judge)
        else {
            continue;
        };
        if neighborhood_survives(&result.metrics, &baseline.metrics, config) {
            surviving += 1;
        }
    }
    let survival = surviving as f64 / config.neighborhood_samples.max(1) as f64;
    if survival + 1e-12 < config.minimum_neighborhood_survival_fraction {
        return Err(RobustnessReject::ParamNeighborhood);
    }

    // ADX gets an explicit local plateau check. The generic ±10% neighborhood
    // perturbs many genes at once; that cannot prove that ADX itself is not a
    // single lucky threshold or period. These neighbours isolate one search
    // profile step in each available direction and require 3 of 4 to survive.
    let plateau_neighbors = adx_plateau_neighbors(strategy, config);
    if !plateau_neighbors.is_empty() {
        let passing = plateau_neighbors
            .iter()
            .filter_map(|neighbor| {
                evaluate_strategy_m1(neighbor, is_decision, m1_dataset, broker, &judge)
                    .ok()
                    .map(|result| neighborhood_survives(&result.metrics, &baseline.metrics, config))
            })
            .filter(|passed| *passed)
            .count();
        let plateau_survival = passing as f64 / plateau_neighbors.len() as f64;
        if plateau_survival + 1e-12 < 0.75 {
            return Err(RobustnessReject::ParamNeighborhood);
        }
    }
    Ok(baseline_result)
}

fn neighborhood_survives(
    candidate: &quantforge_eval::BacktestMetrics,
    baseline: &quantforge_eval::BacktestMetrics,
    config: &RobustnessConfig,
) -> bool {
    let return_ratio = if baseline.return_percent > 0.0 {
        candidate.return_percent / baseline.return_percent
    } else {
        1.0
    };
    let trade_ratio = if baseline.trade_count == 0 {
        0.0
    } else {
        candidate.trade_count as f64 / baseline.trade_count as f64
    };
    let dd_limit = if baseline.max_drawdown_percent > 0.0 {
        baseline.max_drawdown_percent * 1.5
    } else {
        config.maximum_drawdown_percent
    };
    candidate.return_percent > config.minimum_return_percent
        && return_ratio >= 0.5
        && candidate.max_drawdown_percent <= dd_limit
        && candidate.max_drawdown_percent <= config.maximum_drawdown_percent
        && trade_ratio >= 0.5
}

fn adx_plateau_neighbors(strategy: &StrategyIr, config: &RobustnessConfig) -> Vec<StrategyIr> {
    if !strategy_uses_adx(strategy) {
        return Vec::new();
    }
    let mut variants = Vec::new();
    for direction in [-1_i32, 1] {
        let mut neighbor = strategy.clone();
        if adjust_adx_periods(&mut neighbor, direction, config)
            && let Ok(neighbor) = canonicalize_neighbor(neighbor)
        {
            variants.push(neighbor);
        }
    }
    for direction in [-1.0_f64, 1.0] {
        let mut neighbor = strategy.clone();
        if adjust_adx_thresholds(&mut neighbor, direction, config)
            && let Ok(neighbor) = canonicalize_neighbor(neighbor)
        {
            variants.push(neighbor);
        }
    }
    variants
}

fn canonicalize_neighbor(mut strategy: StrategyIr) -> Result<StrategyIr, ()> {
    strategy.id = format!("{}-adx-plateau", strategy.id);
    let strategy = strategy
        .canonicalized(FloatPolicy::default())
        .map_err(|_| ())?;
    strategy
        .validate_export_safe(quantforge_ir::IrLimits::default())
        .map_err(|_| ())?;
    Ok(strategy)
}

fn strategy_uses_adx(strategy: &StrategyIr) -> bool {
    strategy
        .entry
        .long
        .iter()
        .chain(strategy.entry.short.iter())
        .chain(strategy.exit.iter())
        .chain(strategy.filters.iter())
        .any(bool_uses_adx)
}

fn bool_uses_adx(expression: &BoolExpr) -> bool {
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => numeric_uses_adx(left) || numeric_uses_adx(right),
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => numeric_uses_adx(value) || numeric_uses_adx(lower) || numeric_uses_adx(upper),
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            children.iter().any(bool_uses_adx)
        }
        BoolExpr::Not { child } => bool_uses_adx(child),
    }
}

fn numeric_uses_adx(expression: &NumericExpr) -> bool {
    matches!(
        expression,
        NumericExpr::Indicator {
            value: IndicatorExpr::Adx { .. }
        }
    )
}

fn adjust_adx_periods(
    strategy: &mut StrategyIr,
    direction: i32,
    config: &RobustnessConfig,
) -> bool {
    let mut changed = false;
    for expression in strategy
        .entry
        .long
        .iter_mut()
        .chain(strategy.entry.short.iter_mut())
        .chain(strategy.exit.iter_mut())
        .chain(strategy.filters.iter_mut())
    {
        adjust_adx_periods_bool(expression, direction, config, &mut changed);
    }
    changed
}

fn adjust_adx_periods_bool(
    expression: &mut BoolExpr,
    direction: i32,
    config: &RobustnessConfig,
    changed: &mut bool,
) {
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            adjust_adx_periods_numeric(left, direction, config, changed);
            adjust_adx_periods_numeric(right, direction, config, changed);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            adjust_adx_periods_numeric(value, direction, config, changed);
            adjust_adx_periods_numeric(lower, direction, config, changed);
            adjust_adx_periods_numeric(upper, direction, config, changed);
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                adjust_adx_periods_bool(child, direction, config, changed);
            }
        }
        BoolExpr::Not { child } => adjust_adx_periods_bool(child, direction, config, changed),
    }
}

fn adjust_adx_periods_numeric(
    expression: &mut NumericExpr,
    direction: i32,
    config: &RobustnessConfig,
    changed: &mut bool,
) {
    if let NumericExpr::Indicator {
        value: IndicatorExpr::Adx { period, .. },
    } = expression
    {
        let candidate = (*period as i32)
            .saturating_add(direction.saturating_mul(config.adx_period_step as i32));
        if candidate >= config.adx_period_min as i32 && candidate <= config.adx_period_max as i32 {
            *period = candidate as u16;
            *changed = true;
        }
    }
}

fn adjust_adx_thresholds(
    strategy: &mut StrategyIr,
    direction: f64,
    config: &RobustnessConfig,
) -> bool {
    let mut changed = false;
    for expression in strategy
        .entry
        .long
        .iter_mut()
        .chain(strategy.entry.short.iter_mut())
        .chain(strategy.exit.iter_mut())
        .chain(strategy.filters.iter_mut())
    {
        adjust_adx_thresholds_bool(expression, direction, config, &mut changed);
    }
    changed
}

fn adjust_adx_thresholds_bool(
    expression: &mut BoolExpr,
    direction: f64,
    config: &RobustnessConfig,
    changed: &mut bool,
) {
    match expression {
        BoolExpr::Compare { left, right, .. } => {
            if numeric_uses_adx(left) {
                adjust_constant(right, direction, config, changed);
            }
            if numeric_uses_adx(right) {
                adjust_constant(left, direction, config, changed);
            }
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                adjust_adx_thresholds_bool(child, direction, config, changed);
            }
        }
        BoolExpr::Not { child } => adjust_adx_thresholds_bool(child, direction, config, changed),
        BoolExpr::CrossAbove { .. } | BoolExpr::CrossBelow { .. } | BoolExpr::Between { .. } => {}
    }
}

fn adjust_constant(
    expression: &mut NumericExpr,
    direction: f64,
    config: &RobustnessConfig,
    changed: &mut bool,
) {
    if let NumericExpr::Constant { value } = expression {
        let candidate = *value + direction * config.adx_threshold_step;
        if candidate >= config.adx_threshold_min && candidate <= config.adx_threshold_max {
            *value = candidate;
            *changed = true;
        }
    }
}

fn contiguous_fold_ranges(bar_count: usize, folds: usize) -> Vec<(usize, usize)> {
    (0..folds)
        .map(|fold| {
            let start = bar_count * fold / folds;
            let end = bar_count * (fold + 1) / folds;
            (start, end)
        })
        .collect()
}

/// Broker-local calendar-year index ranges covering the IS window.
///
/// Years with fewer than `minimum_year_bars` are skipped. Boundaries use the
/// same `BrokerClock` localization as session logic.
pub(crate) fn calendar_year_fold_ranges(
    dataset: &BarDataset,
    timezone: &str,
) -> Result<Vec<(usize, usize)>, ()> {
    if dataset.bars.is_empty() {
        return Err(());
    }
    let clock = BrokerClock::parse(timezone).map_err(|_| ())?;
    let mut year_starts: Vec<(i32, usize)> = Vec::new();
    let mut last_year = i32::MIN;
    for (index, bar) in dataset.bars.iter().enumerate() {
        let local = clock.local_datetime(bar.timestamp_ms).map_err(|_| ())?;
        let year = local.year();
        if year != last_year {
            year_starts.push((year, index));
            last_year = year;
        }
    }
    let mut ranges = Vec::new();
    for window in year_starts.windows(2) {
        let start = window[0].1;
        let end = window[1].1;
        if end.saturating_sub(start) >= 50 {
            ranges.push((start, end));
        }
    }
    if let Some(&(year, start)) = year_starts.last() {
        let end = dataset.bars.len();
        let _ = year;
        if end.saturating_sub(start) >= 50 {
            ranges.push((start, end));
        }
    }
    if ranges.is_empty() {
        Err(())
    } else {
        Ok(ranges)
    }
}

/// SQX RetestWithHigherPrecision acceptance (80% net/return, 80% trades, DD < 130%).
pub(crate) fn passes_sqx_m1_retention(
    h1: &quantforge_eval::BacktestMetrics,
    m1: &quantforge_eval::BacktestMetrics,
    minimum_return_retention: f64,
) -> bool {
    let return_ok = if h1.return_percent > 0.0 {
        m1.return_percent >= minimum_return_retention * h1.return_percent
    } else {
        m1.return_percent >= h1.return_percent
    };
    let trade_ok = if h1.trade_count == 0 {
        m1.trade_count == 0
    } else {
        (m1.trade_count as f64) >= SQX_TRADE_RETENTION * (h1.trade_count as f64)
    };
    let dd_ok = if h1.max_drawdown_percent > 0.0 {
        m1.max_drawdown_percent < SQX_DRAWDOWN_EXPANSION * h1.max_drawdown_percent
    } else {
        m1.max_drawdown_percent <= 0.0
    };
    return_ok && trade_ok && dd_ok
}

#[allow(dead_code)]
fn metrics_pass(
    metrics: &quantforge_eval::BacktestMetrics,
    minimum_trades: usize,
    config: &RobustnessConfig,
) -> bool {
    metrics.trade_count >= minimum_trades
        && metrics.return_percent > config.minimum_return_percent
        && effective_pf(metrics) >= config.minimum_profit_factor
        && metrics.max_drawdown_percent <= config.maximum_drawdown_percent
}

fn effective_pf(metrics: &quantforge_eval::BacktestMetrics) -> f64 {
    metrics
        .profit_factor
        .unwrap_or(if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::INFINITY
        } else {
            0.0
        })
}

fn slice_dataset(source: &BarDataset, start: usize, end: usize) -> BarDataset {
    let bars = source.bars[start..end].to_vec();
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: source.delimiter,
        source_timezone: source.source_timezone.clone(),
        bars,
    }
}

/// Keep M1 bars in `[start_ms - pad, end_exclusive_ms)`.
///
/// `end_exclusive_ms` must be the open of the last decision bar **plus** that
/// bar's interval — not the open alone — or the final hour is truncated and
/// Judge rejects with an M1 aggregate mismatch.
fn slice_m1_covering(m1: &BarDataset, start_ms: i64, end_exclusive_ms: i64) -> BarDataset {
    let pad_ms = 7 * 24 * 60 * 60 * 1000;
    let from = start_ms.saturating_sub(pad_ms);
    let bars: Vec<_> = m1
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms >= from && bar.timestamp_ms < end_exclusive_ms)
        .cloned()
        .collect();
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: m1.delimiter,
        source_timezone: m1.source_timezone.clone(),
        bars,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::Bar;

    fn bar(ts: i64) -> Bar {
        Bar {
            timestamp_ms: ts,
            open: 1.0,
            high: 1.1,
            low: 0.9,
            close: 1.0,
            tick_volume: 1,
            real_volume: 0,
            spread_points: Some(10),
        }
    }

    #[test]
    fn calendar_year_folds_align_to_broker_local_year_starts() {
        // UTC timestamps that fall in broker years for Etc/UTC.
        let mut bars = Vec::new();
        // 2020-06-01, 2021-06-01, 2022-06-01, 2023-06-01 — plus fillers
        for year in 2020..=2023 {
            for day in 0..60 {
                let ts = chrono::TimeZone::with_ymd_and_hms(
                    &chrono::Utc,
                    year,
                    3,
                    1 + (day % 28),
                    day % 20,
                    0,
                    0,
                )
                .single()
                .unwrap()
                .timestamp_millis();
                bars.push(bar(ts));
            }
        }
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars,
        };
        let ranges = calendar_year_fold_ranges(&dataset, "Etc/UTC").unwrap();
        assert_eq!(ranges.len(), 4);
        // Each range should start on a different calendar year.
        let clock = BrokerClock::parse("Etc/UTC").unwrap();
        let mut years = Vec::new();
        for (start, _) in &ranges {
            let local = clock
                .local_datetime(dataset.bars[*start].timestamp_ms)
                .unwrap();
            years.push(local.year());
        }
        years.sort();
        years.dedup();
        assert_eq!(years.len(), 4);
    }

    #[test]
    fn sqx_m1_retention_matches_80_80_130_bands() {
        let h1 = quantforge_eval::BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 110_000.0,
            net_profit: 10_000.0,
            return_percent: 10.0,
            trade_count: 100,
            winning_trades: 55,
            losing_trades: 45,
            win_rate: 0.55,
            profit_factor: Some(1.4),
            max_drawdown: 5_000.0,
            max_drawdown_percent: 5.0,
            sharpe_ratio: None,
            expectancy: 100.0,
        };
        let mut m1 = h1.clone();
        m1.return_percent = 8.0;
        m1.trade_count = 80;
        m1.max_drawdown_percent = 6.4; // < 1.3 * 5
        assert!(passes_sqx_m1_retention(&h1, &m1, 0.80));

        m1.return_percent = 7.9;
        assert!(!passes_sqx_m1_retention(&h1, &m1, 0.80));
        m1.return_percent = 8.0;
        m1.trade_count = 79;
        assert!(!passes_sqx_m1_retention(&h1, &m1, 0.80));
        m1.trade_count = 80;
        m1.max_drawdown_percent = 6.5; // not < 6.5
        assert!(!passes_sqx_m1_retention(&h1, &m1, 0.80));

        m1.max_drawdown_percent = 6.4;
        m1.trade_count = 100;
        m1.return_percent = 9.4;
        assert!(!passes_sqx_m1_retention(&h1, &m1, 0.95));
        m1.return_percent = 9.5;
        assert!(passes_sqx_m1_retention(&h1, &m1, 0.95));
    }
}
