//! M1 robustness battery on IS (before OOS1 pick) for Discover databank promotion.

use chrono::Datelike;
use quantforge_broker::{BrokerClock, SymbolSpecification};
use quantforge_core::FloatPolicy;
use quantforge_data::{BarDataset, bar_content_hash, infer_median_interval_ms};
use quantforge_eval::{ScoutResult, ScoutTelemetry};
use quantforge_ir::{BoolExpr, IndicatorExpr, NumericExpr, StrategyIr};
use quantforge_quality::{
    monte_carlo_trade_resampling_with_skip, parameter_permutation_neighbors,
    perturb_strategy_parameters,
};

pub use quantforge_quality::{
    MONTE_CARLO_MAX_DRAWDOWN_RATIO, MONTE_CARLO_P80_PROFIT_RETENTION,
    MONTE_CARLO_SKIP_TRADE_PROBABILITY,
};

use crate::model::{
    M1RetentionEvidence, ParameterNeighborhoodEvidence, ParameterNeighborhoodSample,
    RobustnessEvidence, WalkForwardEvidence, WalkForwardFold,
};
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};

/// M1 replay that leaves the battery plus the structured record of what the
/// battery measured. `evidence` is `None` only on the research-only path that
/// skips the battery entirely.
pub struct RobustnessOutcome {
    pub result: ScoutResult,
    pub evidence: Option<RobustnessEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobustnessReject {
    M1Fidelity,
    WalkForward,
    MonteCarlo,
    ParamNeighborhood,
}

pub struct RobustnessConfig {
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
    pub indicator_engine: quantforge_eval::IndicatorEngine,
    /// Mirrors the scout entry window so M1 retention is not measured against a
    /// different trading session than the one that admitted the candidate.
    pub entry_window: quantforge_eval::EntryWindow,
    /// When true, folds are broker-local calendar years and every year must pass.
    pub calendar_year_folds: bool,
}

/// SQX-style RetestWithHigherPrecision defaults retained for trade count and
/// drawdown. QuantForge makes return retention configurable and defaults it to
/// 90% for promotion-grade databanks.
pub(crate) const SQX_TRADE_RETENTION: f64 = 0.80;
pub(crate) const SQX_DRAWDOWN_EXPANSION: f64 = 1.30;
/// Results and promotion test the actual local plateau. ±20% matches the SQX
/// parameter-sensitivity default: wide enough to expose a knife-edge fit, narrow
/// enough that a genuinely robust plateau survives.
pub const PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION: f64 = 0.20;
/// SQX-style trade manipulation: each resampled path removes 10% of fills.

/// M1 baseline → SQX retention vs H1 → WFO/MC/params.
pub fn run_m1_predeposit_robustness(
    strategy: &StrategyIr,
    is_decision: &BarDataset,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &RobustnessConfig,
    h1_metrics: &quantforge_eval::BacktestMetrics,
) -> Result<RobustnessOutcome, RobustnessReject> {
    let judge = JudgeConfig {
        initial_balance: config.initial_balance,
        costs: config.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: config.indicator_engine,
        entry_window: config.entry_window,
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
    let retention_evidence = m1_retention_evidence(h1_metrics, &baseline.metrics, config);

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
    let mut fold_rows = Vec::with_capacity(folds.len());
    for (index, (start, end)) in folds.iter().enumerate() {
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
        let fold_passed = fold_trades >= config.minimum_fold_trades
            && fold_result.metrics.return_percent > config.minimum_return_percent
            && effective_pf(&fold_result.metrics) >= config.minimum_profit_factor
            && fold_result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent;
        if fold_passed {
            passing_folds += 1;
        }
        fold_rows.push(WalkForwardFold {
            fold: index,
            start_timestamp_ms: start_ms,
            end_timestamp_ms: last_open_ms,
            decision_bars: end.saturating_sub(*start),
            trades_in_fold: fold_trades,
            metrics: fold_result.metrics.clone(),
            passed: fold_passed,
        });
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
    let walk_forward_evidence = WalkForwardEvidence {
        fold_scheme: if config.calendar_year_folds {
            "calendar_year".into()
        } else {
            "contiguous".into()
        },
        total_folds: folds.len(),
        passing_folds,
        passing_fraction: fold_fraction,
        required_passing_fraction: required_fraction,
        folds: fold_rows,
    };

    let profits: Vec<_> = baseline
        .trades
        .iter()
        .map(|trade| trade.net_profit)
        .collect();
    let maximum_p95_drawdown_percent = baseline.metrics.max_drawdown_percent
        * MONTE_CARLO_MAX_DRAWDOWN_RATIO;
    let mut mc = monte_carlo_trade_resampling_with_skip(
        &profits,
        config.initial_balance,
        config.monte_carlo_trials,
        5,
        MONTE_CARLO_SKIP_TRADE_PROBABILITY,
        config.seed,
        0.0,
        maximum_p95_drawdown_percent,
        baseline.metrics.net_profit,
        MONTE_CARLO_P80_PROFIT_RETENTION,
    );
    mc.baseline_max_drawdown_percent = baseline.metrics.max_drawdown_percent;
    mc.maximum_drawdown_ratio = MONTE_CARLO_MAX_DRAWDOWN_RATIO;
    // Require a non-negative median path and the shared P80 retention gate
    // (p80_net_profit >= 60% of baseline net profit) encoded in `mc.passed`.
    if !mc.passed || mc.median_net_profit < 0.0 {
        return Err(RobustnessReject::MonteCarlo);
    }

    let mut surviving = 0usize;
    let mut evaluated_samples = 0usize;
    let mut neighborhood_samples = Vec::with_capacity(config.neighborhood_samples);
    // Test deterministic one-axis low/high permutations first.  A joint
    // seeded perturbation fills any remaining budget, preserving the existing
    // fast/deep runtime presets while making the result a real plateau test.
    let mut permutation_neighbors = parameter_permutation_neighbors(
        strategy,
        config.parameter_perturbation_fraction,
    )
    .unwrap_or_default()
    .into_iter();
    for sample in 0..config.neighborhood_samples {
        let neighbor = permutation_neighbors.next().or_else(|| {
            perturb_strategy_parameters(
                strategy,
                config.parameter_perturbation_fraction,
                sample,
                config.seed,
            )
            .ok()
        });
        let Some(neighbor) = neighbor else { continue };
        let Ok(result) = evaluate_strategy_m1(&neighbor, is_decision, m1_dataset, broker, &judge)
        else {
            continue;
        };
        evaluated_samples += 1;
        let survived = neighborhood_survives(&result.metrics, &baseline.metrics, config);
        if survived {
            surviving += 1;
        }
        neighborhood_samples.push(ParameterNeighborhoodSample {
            sample_index: sample,
            net_profit: result.metrics.net_profit,
            return_percent: result.metrics.return_percent,
            max_drawdown_percent: result.metrics.max_drawdown_percent,
            trade_count: result.metrics.trade_count,
            profit_factor: result.metrics.profit_factor,
            sharpe_ratio: result.metrics.sharpe_ratio,
            survived,
        });
    }
    // A neighbor that could not be built or replayed says nothing about the width of
    // the plateau, so score against the samples that actually ran. Still demand that
    // most of them ran, otherwise the evidence is too thin to promote on.
    if evaluated_samples * 2 < config.neighborhood_samples {
        return Err(RobustnessReject::ParamNeighborhood);
    }
    let survival = surviving as f64 / evaluated_samples.max(1) as f64;
    if survival + 1e-12 < config.minimum_neighborhood_survival_fraction {
        return Err(RobustnessReject::ParamNeighborhood);
    }

    // ADX gets an explicit local plateau check. The generic ±20% neighborhood
    // perturbs many genes at once; that cannot prove that ADX itself is not a
    // single lucky threshold or period. These neighbours isolate one search
    // profile step in each available direction and require 3 of 4 to survive.
    let plateau_neighbors = adx_plateau_neighbors(strategy, config);
    let mut plateau_surviving = 0usize;
    let mut plateau_survival_fraction = None;
    if !plateau_neighbors.is_empty() {
        plateau_surviving = plateau_neighbors
            .iter()
            .filter_map(|neighbor| {
                evaluate_strategy_m1(neighbor, is_decision, m1_dataset, broker, &judge)
                    .ok()
                    .map(|result| neighborhood_survives(&result.metrics, &baseline.metrics, config))
            })
            .filter(|passed| *passed)
            .count();
        let plateau_survival = plateau_surviving as f64 / plateau_neighbors.len() as f64;
        plateau_survival_fraction = Some(plateau_survival);
        if plateau_survival + 1e-12 < config.minimum_neighborhood_survival_fraction {
            return Err(RobustnessReject::ParamNeighborhood);
        }
    }
    Ok(RobustnessOutcome {
        result: baseline_result,
        evidence: Some(RobustnessEvidence {
            m1_retention: retention_evidence,
            walk_forward: walk_forward_evidence,
            monte_carlo: mc,
            parameter_neighborhood: ParameterNeighborhoodEvidence {
                method: "systematic_axis_plus_seeded_joint".into(),
                perturbation_fraction: config.parameter_perturbation_fraction,
                samples_requested: config.neighborhood_samples,
                samples_evaluated: evaluated_samples,
                surviving_samples: surviving,
                survival_fraction: survival,
                required_survival_fraction: config.minimum_neighborhood_survival_fraction,
                plateau_neighbors: plateau_neighbors.len(),
                plateau_surviving,
                plateau_survival_fraction,
                original_metrics: Some(baseline.metrics.clone()),
                samples: neighborhood_samples,
            },
        }),
    })
}

fn m1_retention_evidence(
    h1: &quantforge_eval::BacktestMetrics,
    m1: &quantforge_eval::BacktestMetrics,
    config: &RobustnessConfig,
) -> M1RetentionEvidence {
    let ratio = |numerator: f64, denominator: f64| {
        (denominator > 0.0)
            .then_some(numerator / denominator)
            .filter(|value| value.is_finite())
    };
    M1RetentionEvidence {
        selected_timeframe_metrics: h1.clone(),
        minimum_return_retention: config.minimum_return_retention,
        return_retention: ratio(m1.return_percent, h1.return_percent),
        trade_retention: ratio(m1.trade_count as f64, h1.trade_count as f64),
        drawdown_expansion: ratio(m1.max_drawdown_percent, h1.max_drawdown_percent),
    }
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
        .chain(strategy.exit_long.iter())
        .chain(strategy.exit_short.iter())
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
        .chain(strategy.exit_long.iter_mut())
        .chain(strategy.exit_short.iter_mut())
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
        .chain(strategy.exit_long.iter_mut())
        .chain(strategy.exit_short.iter_mut())
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
