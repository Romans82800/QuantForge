//! H1 robustness battery on Development after one M1 80/130 fidelity check.

use chrono::Datelike;
use quantforge_broker::{BrokerClock, SymbolSpecification};
use quantforge_core::FloatPolicy;
use quantforge_data::{BarDataset, QuoteBarDataset, bar_content_hash, quote_bar_content_hash};
use quantforge_eval::{
    IndicatorBufferCache, SameBarPolicy, ScoutConfig, ScoutResult, ScoutTelemetry,
    evaluate_strategy_cached,
};
use quantforge_ir::{BoolExpr, IndicatorExpr, NumericExpr, StrategyIr};
use quantforge_quality::{
    DevelopmentCpcvPlan, monte_carlo_trade_resampling_with_skip, parameter_permutation_neighbors,
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
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1, evaluate_strategy_m1_with_quotes};

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
    FoldStability,
    Cpcv,
    WalkForward,
    MonteCarlo,
    ParamNeighborhood,
}

impl RobustnessReject {
    pub fn kill_bucket(self) -> &'static str {
        match self {
            Self::M1Fidelity => "m1",
            Self::FoldStability | Self::Cpcv | Self::WalkForward => "folds",
            Self::MonteCarlo => "monte_carlo",
            Self::ParamNeighborhood => "neighborhood",
        }
    }
}

impl std::fmt::Display for RobustnessReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::M1Fidelity => "M1 fidelity",
            Self::FoldStability => "calendar-year fold stability (pooled/median R, concentration)",
            Self::Cpcv => "CPCV folds",
            Self::WalkForward => "walk-forward",
            Self::MonteCarlo => "Monte Carlo",
            Self::ParamNeighborhood => "parameter neighborhood / Ret/DD 0.85–1.25",
        })
    }
}

pub struct RobustnessConfig {
    pub folds: usize,
    pub monte_carlo_trials: usize,
    /// Moving-block length for trade-resampling Monte Carlo (default 5).
    pub monte_carlo_block_length: usize,
    /// Fraction of trades removed from each MC path (default 0.10).
    pub monte_carlo_skip_trade_probability: f64,
    /// P80 net-profit retention vs baseline required to pass (default 0.60).
    pub monte_carlo_minimum_p80_profit_retention: f64,
    /// P95 simulated drawdown may not exceed this multiple of baseline DD.
    pub monte_carlo_max_drawdown_ratio: f64,
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
/// Original recovery factor must sit this close to the neighbourhood median.
/// Knife-edge fits land on the tails of the ret/DD histogram and collapse in holdout.
pub const PARAM_RECOVERY_MEDIAN_LOW: f64 = 0.85;
pub const PARAM_RECOVERY_MEDIAN_HIGH: f64 = 1.25;
/// Histogram of ret/DD is too noisy below this many finite neighbour recoveries.
const PARAM_RECOVERY_BAND_MIN_SAMPLES: usize = 20;

/// M1 80/130 fidelity only. Fold stability, plateau, CPCV, and Monte Carlo are
/// the Databank battery, not Holding admission.
pub fn run_m1_holding_admission(
    strategy: &StrategyIr,
    is_decision: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
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
    let baseline = evaluate_strategy_m1_with_optional_quotes(
        strategy,
        is_decision,
        m1_dataset,
        quote_dataset,
        broker,
        &judge,
    )
    .map_err(|_| RobustnessReject::M1Fidelity)?;
    let baseline_result = ScoutResult {
        trades: baseline.trades.clone(),
        equity: baseline.equity.clone(),
        metrics: baseline.metrics.clone(),
        telemetry: ScoutTelemetry::default(),
    };
    if !passes_sqx_m1_retention(
        h1_metrics,
        &baseline.metrics,
        config.minimum_return_retention,
    ) {
        return Err(RobustnessReject::M1Fidelity);
    }
    let _retention = m1_retention_evidence(h1_metrics, &baseline.metrics, config);
    Ok(RobustnessOutcome {
        result: baseline_result,
        // Holding is pre-battery: keep the audit trail empty until the user
        // runs WFO / Monte Carlo / ±param and promotes to Databank.
        evidence: None,
    })
}

/// M1 baseline → retention vs selected timeframe → Development CPCV/MC/params.
///
/// Set `enforce_m1_retention` to false for the Holding → Databank battery: M1
/// fidelity was already the Holding admission gate, and re-scouting Selected-TF
/// here can disagree with the original pot H1 and falsely reject as M1Fidelity.
pub fn run_m1_predeposit_robustness(
    strategy: &StrategyIr,
    is_decision: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    config: &RobustnessConfig,
    h1_metrics: &quantforge_eval::BacktestMetrics,
    enforce_m1_retention: bool,
) -> Result<RobustnessOutcome, RobustnessReject> {
    let judge = JudgeConfig {
        initial_balance: config.initial_balance,
        costs: config.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: config.indicator_engine,
        entry_window: config.entry_window,
    };
    let baseline = evaluate_strategy_m1_with_optional_quotes(
        strategy,
        is_decision,
        m1_dataset,
        quote_dataset,
        broker,
        &judge,
    )
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
    if enforce_m1_retention
        && !passes_sqx_m1_retention(
            h1_metrics,
            &baseline.metrics,
            config.minimum_return_retention,
        )
    {
        return Err(RobustnessReject::M1Fidelity);
    }
    let retention_evidence = m1_retention_evidence(h1_metrics, &baseline.metrics, config);
    let h1_scout = neighborhood_scout_config(config);
    let h1_cache = IndicatorBufferCache::new(is_decision.bars.len());
    let h1_run = evaluate_strategy_cached(strategy, is_decision, broker, &h1_scout, &h1_cache)
        .map_err(|_| RobustnessReject::ParamNeighborhood)?;
    if !crate::fold_r::calendar_year_fold_r(&h1_run.trades).passes_stability() {
        return Err(RobustnessReject::FoldStability);
    }
    // H1 permutation / Ret/DD before CPCV or Monte Carlo.
    let parameter_neighborhood =
        evaluate_h1_neighborhood(strategy, is_decision, broker, config, &h1_run.metrics)?;

    let (fold_rows, fold_scheme, purge_bars, embargo_bars, required_fraction) =
        if config.calendar_year_folds {
            let ranges = calendar_year_fold_ranges(is_decision, &broker.timezone)
                .map_err(|_| RobustnessReject::Cpcv)?;
            let rows = evaluate_development_ranges(
                strategy,
                is_decision,
                m1_dataset,
                quote_dataset,
                broker,
                &judge,
                config,
                &ranges,
            )?;
            (rows, "development_calendar_year".into(), 0, 0, 1.0)
        } else {
            let contract = DevelopmentCpcvPlan::for_development_bars(is_decision.bars.len());
            let rows = evaluate_development_cpcv(
                strategy,
                is_decision,
                m1_dataset,
                quote_dataset,
                broker,
                &judge,
                config,
                &contract,
            )?;
            (
                rows,
                "development_cpcv_6_choose_2_h1".into(),
                contract.purge_bars,
                contract.embargo_bars,
                config.minimum_passing_fold_fraction,
            )
        };
    if fold_rows.is_empty() {
        return Err(RobustnessReject::Cpcv);
    }
    let passing_folds = fold_rows.iter().filter(|fold| fold.passed).count();
    let fold_fraction = passing_folds as f64 / fold_rows.len().max(1) as f64;
    if fold_fraction + 1e-12 < required_fraction {
        return Err(RobustnessReject::Cpcv);
    }
    let walk_forward_evidence = WalkForwardEvidence {
        fold_scheme,
        total_folds: fold_rows.len(),
        passing_folds,
        passing_fraction: fold_fraction,
        required_passing_fraction: required_fraction,
        purge_bars,
        embargo_bars,
        folds: fold_rows,
    };

    // CPCV deliberately permutes held-out Development groups and therefore
    // does not test chronological degradation. Follow it with distinct,
    // ordered windows so regime decay must also survive before promotion.
    let sequential_ranges =
        sequential_walk_forward_ranges(is_decision.bars.len(), config.folds.max(3))
            .ok_or(RobustnessReject::WalkForward)?;
    let sequential_rows = evaluate_development_ranges(
        strategy,
        is_decision,
        m1_dataset,
        quote_dataset,
        broker,
        &judge,
        config,
        &sequential_ranges,
    )?;
    let sequential_passing = sequential_rows.iter().filter(|fold| fold.passed).count();
    let sequential_fraction = sequential_passing as f64 / sequential_rows.len().max(1) as f64;
    if sequential_fraction + 1e-12 < config.minimum_passing_fold_fraction {
        return Err(RobustnessReject::WalkForward);
    }
    let sequential_walk_forward = WalkForwardEvidence {
        fold_scheme: "development_sequential_walk_forward_h1".into(),
        total_folds: sequential_rows.len(),
        passing_folds: sequential_passing,
        passing_fraction: sequential_fraction,
        required_passing_fraction: config.minimum_passing_fold_fraction,
        purge_bars: 0,
        embargo_bars: 0,
        folds: sequential_rows,
    };

    let profits: Vec<_> = h1_run.trades.iter().map(|trade| trade.net_profit).collect();
    let maximum_p95_drawdown_percent =
        h1_run.metrics.max_drawdown_percent * config.monte_carlo_max_drawdown_ratio;
    let mut mc = monte_carlo_trade_resampling_with_skip(
        &profits,
        config.initial_balance,
        config.monte_carlo_trials,
        config.monte_carlo_block_length.max(1),
        config.monte_carlo_skip_trade_probability,
        config.seed,
        0.0,
        maximum_p95_drawdown_percent,
        h1_run.metrics.net_profit,
        config.monte_carlo_minimum_p80_profit_retention,
    );
    mc.baseline_max_drawdown_percent = h1_run.metrics.max_drawdown_percent;
    mc.maximum_drawdown_ratio = config.monte_carlo_max_drawdown_ratio;
    // Require a non-negative median path and the configured P80 retention gate
    // encoded in `mc.passed`.
    if !mc.passed || mc.median_net_profit < 0.0 {
        return Err(RobustnessReject::MonteCarlo);
    }

    Ok(RobustnessOutcome {
        result: baseline_result,
        evidence: Some(RobustnessEvidence {
            m1_retention: retention_evidence,
            walk_forward: walk_forward_evidence,
            sequential_walk_forward: Some(sequential_walk_forward),
            monte_carlo: mc,
            parameter_neighborhood,
        }),
    })
}

/// Evaluate the reusable Development CPCV rows without running the later
/// Monte Carlo and parameter-neighborhood gates. This is intentionally a
/// diagnostic surface: it makes cross-asset gate failures inspectable instead
/// of reducing them to a single rejection counter.
pub fn development_cpcv_diagnostic(
    strategy: &StrategyIr,
    development: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    config: &RobustnessConfig,
) -> Result<Vec<WalkForwardFold>, RobustnessReject> {
    let judge = JudgeConfig {
        initial_balance: config.initial_balance,
        costs: config.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: config.indicator_engine,
        entry_window: config.entry_window,
    };
    let contract = DevelopmentCpcvPlan::for_development_bars(development.bars.len());
    evaluate_development_cpcv(
        strategy,
        development,
        m1_dataset,
        quote_dataset,
        broker,
        &judge,
        config,
        &contract,
    )
}

fn evaluate_strategy_m1_with_optional_quotes(
    strategy: &StrategyIr,
    decision_dataset: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    judge: &JudgeConfig,
) -> Result<quantforge_tick::JudgeResult, quantforge_tick::JudgeError> {
    match quote_dataset {
        Some(quotes) => evaluate_strategy_m1_with_quotes(
            strategy,
            decision_dataset,
            m1_dataset,
            quotes,
            broker,
            judge,
        ),
        None => evaluate_strategy_m1(strategy, decision_dataset, m1_dataset, broker, judge),
    }
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

fn evaluate_h1_neighborhood(
    strategy: &StrategyIr,
    is_decision: &BarDataset,
    broker: &SymbolSpecification,
    config: &RobustnessConfig,
    h1_metrics: &quantforge_eval::BacktestMetrics,
) -> Result<ParameterNeighborhoodEvidence, RobustnessReject> {
    use rayon::prelude::*;
    let scout = neighborhood_scout_config(config);
    let cache = IndicatorBufferCache::new(is_decision.bars.len());
    let mut permutation_neighbors =
        parameter_permutation_neighbors(strategy, config.parameter_perturbation_fraction)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
    let sample_rows: Vec<(usize, Option<StrategyIr>)> = (0..config.neighborhood_samples)
        .map(|sample| {
            let neighbor = permutation_neighbors
                .first()
                .cloned()
                .map(|first| {
                    permutation_neighbors.remove(0);
                    first
                })
                .or_else(|| {
                    perturb_strategy_parameters(
                        strategy,
                        config.parameter_perturbation_fraction,
                        sample,
                        config.seed,
                    )
                    .ok()
                });
            (sample, neighbor)
        })
        .collect();
    let neighborhood_samples: Vec<ParameterNeighborhoodSample> = sample_rows
        .par_iter()
        .filter_map(|(sample, neighbor)| {
            let neighbor = neighbor.as_ref()?;
            let result =
                evaluate_strategy_cached(neighbor, is_decision, broker, &scout, &cache).ok()?;
            let survived = neighborhood_survives(&result.metrics, h1_metrics, config);
            Some(neighborhood_sample(*sample, &result.metrics, survived))
        })
        .collect();
    let evaluated_samples = neighborhood_samples.len();
    let surviving = neighborhood_samples
        .iter()
        .filter(|row| row.survived)
        .count();
    if evaluated_samples * 2 < config.neighborhood_samples {
        return Err(RobustnessReject::ParamNeighborhood);
    }
    let survival = surviving as f64 / evaluated_samples.max(1) as f64;
    if survival + 1e-12 < config.minimum_neighborhood_survival_fraction {
        return Err(RobustnessReject::ParamNeighborhood);
    }
    let recoveries: Vec<f64> = neighborhood_samples
        .iter()
        .filter_map(|sample| sample.recovery_factor)
        .collect();
    let median_recovery_factor = median_finite(&recoveries);
    let original_recovery_to_median =
        recovery_to_median_ratio(h1_metrics.recovery_factor(), &recoveries);
    let passed_recovery_median_band =
        passes_recovery_median_band(original_recovery_to_median, recoveries.len());
    if !passed_recovery_median_band {
        return Err(RobustnessReject::ParamNeighborhood);
    }

    let plateau_neighbors = adx_plateau_neighbors(strategy, config);
    let mut plateau_surviving = 0usize;
    let mut plateau_survival_fraction = None;
    if !plateau_neighbors.is_empty() {
        plateau_surviving = plateau_neighbors
            .iter()
            .filter_map(|neighbor| {
                evaluate_strategy_cached(neighbor, is_decision, broker, &scout, &cache)
                    .ok()
                    .map(|result| neighborhood_survives(&result.metrics, h1_metrics, config))
            })
            .filter(|passed| *passed)
            .count();
        let plateau_survival = plateau_surviving as f64 / plateau_neighbors.len() as f64;
        plateau_survival_fraction = Some(plateau_survival);
        if plateau_survival + 1e-12 < config.minimum_neighborhood_survival_fraction {
            return Err(RobustnessReject::ParamNeighborhood);
        }
    }
    Ok(ParameterNeighborhoodEvidence {
        method: "h1_cached_axis_plus_seeded_joint".into(),
        perturbation_fraction: config.parameter_perturbation_fraction,
        samples_requested: config.neighborhood_samples,
        samples_evaluated: evaluated_samples,
        surviving_samples: surviving,
        survival_fraction: survival,
        required_survival_fraction: config.minimum_neighborhood_survival_fraction,
        plateau_neighbors: plateau_neighbors.len(),
        plateau_surviving,
        plateau_survival_fraction,
        original_metrics: Some(h1_metrics.clone()),
        median_recovery_factor,
        original_recovery_to_median,
        passed_recovery_median_band: Some(passed_recovery_median_band),
        samples: neighborhood_samples,
    })
}

fn neighborhood_scout_config(config: &RobustnessConfig) -> ScoutConfig {
    ScoutConfig {
        initial_balance: config.initial_balance,
        same_bar_policy: SameBarPolicy::Conservative,
        costs: config.costs.clone(),
        indicator_engine: config.indicator_engine,
        entry_window: config.entry_window,
        abandon_above_drawdown_percent: None,
    }
}

fn neighborhood_sample(
    sample_index: usize,
    metrics: &quantforge_eval::BacktestMetrics,
    survived: bool,
) -> ParameterNeighborhoodSample {
    let recovery = metrics.recovery_factor();
    ParameterNeighborhoodSample {
        sample_index,
        net_profit: metrics.net_profit,
        return_percent: metrics.return_percent,
        max_drawdown_percent: metrics.max_drawdown_percent,
        max_drawdown: metrics.max_drawdown,
        trade_count: metrics.trade_count,
        profit_factor: metrics.profit_factor,
        sharpe_ratio: metrics.sharpe_ratio,
        recovery_factor: recovery.is_finite().then_some(recovery),
        survived,
    }
}

fn median_finite(values: &[f64]) -> Option<f64> {
    let mut sorted: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    })
}

/// Original recovery ÷ neighbourhood-median recovery when the histogram is dense enough.
pub fn recovery_to_median_ratio(original: f64, neighbor_recoveries: &[f64]) -> Option<f64> {
    if !original.is_finite() || original <= 0.0 {
        return None;
    }
    let median = median_finite(neighbor_recoveries)?;
    if !median.is_finite() || median.abs() < 1e-12 {
        return None;
    }
    Some(original / median)
}

pub fn passes_recovery_median_band(ratio: Option<f64>, finite_neighbors: usize) -> bool {
    if finite_neighbors < PARAM_RECOVERY_BAND_MIN_SAMPLES {
        return true;
    }
    match ratio {
        Some(value) => (PARAM_RECOVERY_MEDIAN_LOW..=PARAM_RECOVERY_MEDIAN_HIGH).contains(&value),
        None => false,
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

/// Ordered, non-overlapping Development test windows used after CPCV.
/// Indicator warm-up is supplied by `evaluate_development_window`; only trades
/// entering inside each chronological window are scored.
fn sequential_walk_forward_ranges(
    bars: usize,
    requested_folds: usize,
) -> Option<Vec<(usize, usize)>> {
    let folds = requested_folds.clamp(3, 12);
    if bars < folds.saturating_mul(50) {
        return None;
    }
    let base = bars / folds;
    let mut ranges = Vec::with_capacity(folds);
    for fold in 0..folds {
        let start = fold.saturating_mul(base);
        let end = if fold + 1 == folds {
            bars
        } else {
            (fold + 1).saturating_mul(base)
        };
        if end.saturating_sub(start) < 50 {
            return None;
        }
        ranges.push((start, end));
    }
    Some(ranges)
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

#[allow(clippy::too_many_arguments)]
fn evaluate_development_ranges(
    strategy: &StrategyIr,
    development: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    judge: &JudgeConfig,
    config: &RobustnessConfig,
    ranges: &[(usize, usize)],
) -> Result<Vec<WalkForwardFold>, RobustnessReject> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, &(start, end))| {
            evaluate_development_window(
                strategy,
                development,
                m1_dataset,
                quote_dataset,
                broker,
                judge,
                config,
                index,
                vec![index],
                start,
                end,
            )
            .map(|group| group.fold)
        })
        .collect()
}

struct DevelopmentGroupEvaluation {
    fold: WalkForwardFold,
    trade_profits: Vec<f64>,
    relative_equity: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
fn evaluate_development_cpcv(
    strategy: &StrategyIr,
    development: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    judge: &JudgeConfig,
    config: &RobustnessConfig,
    contract: &DevelopmentCpcvPlan,
) -> Result<Vec<WalkForwardFold>, RobustnessReject> {
    let ranges = contract.group_ranges(development.bars.len());
    let mut groups = Vec::with_capacity(ranges.len());
    for (group, &(raw_start, raw_end)) in ranges.iter().enumerate() {
        // Boundary observations are not scored. The lookback prefix is still
        // supplied for indicator warmup but entries there are excluded below.
        let start = if group == 0 {
            raw_start
        } else {
            raw_start.saturating_add(contract.purge_bars)
        };
        let end = if group + 1 == ranges.len() {
            raw_end
        } else {
            raw_end.saturating_sub(contract.embargo_bars)
        };
        groups.push(evaluate_development_window(
            strategy,
            development,
            m1_dataset,
            quote_dataset,
            broker,
            judge,
            config,
            group,
            vec![group],
            start,
            end,
        )?);
    }

    Ok(contract
        .test_group_combinations()
        .into_iter()
        .enumerate()
        .map(|(combination, test_groups)| {
            let selected = test_groups
                .iter()
                .map(|group| &groups[*group])
                .collect::<Vec<_>>();
            let metrics = combine_group_metrics(&selected, config.initial_balance);
            WalkForwardFold {
                fold: combination,
                test_groups,
                start_timestamp_ms: selected
                    .first()
                    .map(|group| group.fold.start_timestamp_ms)
                    .unwrap_or_default(),
                end_timestamp_ms: selected
                    .last()
                    .map(|group| group.fold.end_timestamp_ms)
                    .unwrap_or_default(),
                decision_bars: selected.iter().map(|group| group.fold.decision_bars).sum(),
                trades_in_fold: selected.iter().map(|group| group.fold.trades_in_fold).sum(),
                // A CPCV row is one concatenated held-out path. Requiring both
                // component groups to pass independently silently turned a
                // 60% combination threshold into an approximately 5-of-6
                // regime threshold and unfairly rejected lumpier instruments.
                passed: metrics_pass(&metrics, config.minimum_fold_trades, config),
                metrics,
            }
        })
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn evaluate_development_window(
    strategy: &StrategyIr,
    development: &BarDataset,
    _m1_dataset: &BarDataset,
    _quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    _judge: &JudgeConfig,
    config: &RobustnessConfig,
    fold: usize,
    test_groups: Vec<usize>,
    start: usize,
    end: usize,
) -> Result<DevelopmentGroupEvaluation, RobustnessReject> {
    if end <= start + 1 || end > development.bars.len() {
        return Err(RobustnessReject::WalkForward);
    }
    let lookback = 320usize;
    let decision_slice = slice_dataset(development, start.saturating_sub(lookback), end);
    let start_ms = development.bars[start].timestamp_ms;
    let last_open_ms = development.bars[end - 1].timestamp_ms;
    let _ = decision_slice
        .bars
        .first()
        .map(|bar| bar.timestamp_ms)
        .ok_or(RobustnessReject::WalkForward)?;
    let scout = neighborhood_scout_config(config);
    let cache = IndicatorBufferCache::new(decision_slice.bars.len());
    let result = evaluate_strategy_cached(strategy, &decision_slice, broker, &scout, &cache)
        .map_err(|_| RobustnessReject::WalkForward)?;
    let trades = result
        .trades
        .iter()
        .filter(|trade| {
            trade.entry_timestamp_ms >= start_ms && trade.entry_timestamp_ms <= last_open_ms
        })
        .count();
    let passed = trades >= config.minimum_fold_trades
        && result.metrics.return_percent > config.minimum_return_percent
        && effective_pf(&result.metrics) >= config.minimum_profit_factor
        && result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent;
    let trade_profits = result.trades.iter().map(|trade| trade.net_profit).collect();
    let relative_equity = result
        .equity
        .iter()
        .map(|point| point.equity - config.initial_balance)
        .collect();
    Ok(DevelopmentGroupEvaluation {
        fold: WalkForwardFold {
            fold,
            test_groups,
            start_timestamp_ms: start_ms,
            end_timestamp_ms: last_open_ms,
            decision_bars: end - start,
            trades_in_fold: trades,
            metrics: result.metrics,
            passed,
        },
        trade_profits,
        relative_equity,
    })
}

fn combine_group_metrics(
    groups: &[&DevelopmentGroupEvaluation],
    initial_balance: f64,
) -> quantforge_eval::BacktestMetrics {
    let net_profit = groups
        .iter()
        .map(|group| group.fold.metrics.net_profit)
        .sum::<f64>();
    let trade_count = groups
        .iter()
        .map(|group| group.fold.trades_in_fold)
        .sum::<usize>();
    let winning_trades = groups
        .iter()
        .flat_map(|group| group.trade_profits.iter())
        .filter(|profit| **profit > 0.0)
        .count();
    let losing_trades = groups
        .iter()
        .flat_map(|group| group.trade_profits.iter())
        .filter(|profit| **profit < 0.0)
        .count();
    let gross_wins = groups
        .iter()
        .flat_map(|group| group.trade_profits.iter())
        .filter(|profit| **profit > 0.0)
        .sum::<f64>();
    let gross_losses = -groups
        .iter()
        .flat_map(|group| group.trade_profits.iter())
        .filter(|profit| **profit < 0.0)
        .sum::<f64>();
    let profit_factor = (gross_losses > 0.0).then_some(gross_wins / gross_losses);

    // Rebase each independently replayed Development group onto the balance
    // left by the previous group. This creates the actual concatenated CPCV
    // path instead of taking the worst component metric as a proxy.
    let mut offset = 0.0;
    let mut peak = initial_balance;
    let mut max_drawdown = 0.0_f64;
    let mut max_drawdown_percent = 0.0_f64;
    for group in groups {
        for relative in &group.relative_equity {
            let equity = initial_balance + offset + relative;
            peak = peak.max(equity);
            let drawdown = peak - equity;
            max_drawdown = max_drawdown.max(drawdown);
            if peak > 0.0 {
                max_drawdown_percent = max_drawdown_percent.max(drawdown / peak * 100.0);
            }
        }
        offset += group.fold.metrics.net_profit;
    }
    quantforge_eval::BacktestMetrics {
        initial_balance,
        ending_balance: initial_balance + net_profit,
        net_profit,
        return_percent: net_profit / initial_balance * 100.0,
        trade_count,
        winning_trades,
        losing_trades,
        win_rate: if trade_count == 0 {
            0.0
        } else {
            winning_trades as f64 / trade_count as f64 * 100.0
        },
        profit_factor,
        max_drawdown,
        max_drawdown_percent,
        sharpe_ratio: groups
            .iter()
            .filter_map(|group| group.fold.metrics.sharpe_ratio)
            .min_by(f64::total_cmp),
        expectancy: if trade_count == 0 {
            0.0
        } else {
            net_profit / trade_count as f64
        },
        expectancy_r: 0.0,
        median_r: 0.0,
    }
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

/// Keep M1 bars in `[start_ms, end_exclusive_ms)`.
///
/// The caller derives `start_ms` from the exact decision slice (including its
/// warm-up prefix). Using a calendar-duration guess here was both incorrect
/// across weekend/session calendars and a source of cross-asset CPCV bias.
/// `end_exclusive_ms` must be the open of the last decision bar **plus** that
/// bar's interval, or the final decision bar is truncated.
#[allow(dead_code)]
fn slice_m1_covering(m1: &BarDataset, start_ms: i64, end_exclusive_ms: i64) -> BarDataset {
    let bars: Vec<_> = m1
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms >= start_ms && bar.timestamp_ms < end_exclusive_ms)
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

#[allow(dead_code)]
fn slice_quotes_covering(quotes: &QuoteBarDataset, m1: &BarDataset) -> QuoteBarDataset {
    let (Some(first), Some(last)) = (m1.bars.first(), m1.bars.last()) else {
        return QuoteBarDataset {
            bars: Vec::new(),
            source_rows: 0,
            source_timezone: quotes.source_timezone.clone(),
            data_hash: quote_bar_content_hash(&[]),
            schema_version: QuoteBarDataset::SCHEMA_VERSION,
            source_model: quotes.source_model,
        };
    };
    let start_ms = first.timestamp_ms;
    let end_exclusive_ms = last.timestamp_ms.saturating_add(60_000);
    let bars = quotes
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms >= start_ms && bar.timestamp_ms < end_exclusive_ms)
        .cloned()
        .collect::<Vec<_>>();
    QuoteBarDataset {
        source_rows: bars.len(),
        data_hash: quote_bar_content_hash(&bars),
        bars,
        source_timezone: quotes.source_timezone.clone(),
        schema_version: quotes.schema_version,
        source_model: quotes.source_model,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::Bar;

    fn group(net_profits: &[f64], passed: bool) -> DevelopmentGroupEvaluation {
        let net_profit = net_profits.iter().sum::<f64>();
        let winning_trades = net_profits.iter().filter(|profit| **profit > 0.0).count();
        let losing_trades = net_profits.iter().filter(|profit| **profit < 0.0).count();
        let metrics = quantforge_eval::BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 100_000.0 + net_profit,
            net_profit,
            return_percent: net_profit / 1_000.0,
            trade_count: net_profits.len(),
            winning_trades,
            losing_trades,
            win_rate: winning_trades as f64 / net_profits.len() as f64 * 100.0,
            profit_factor: None,
            max_drawdown: 0.0,
            max_drawdown_percent: 0.0,
            sharpe_ratio: None,
            expectancy: net_profit / net_profits.len() as f64,
            expectancy_r: 0.0,
            median_r: 0.0,
        };
        DevelopmentGroupEvaluation {
            fold: WalkForwardFold {
                fold: 0,
                test_groups: vec![0],
                start_timestamp_ms: 0,
                end_timestamp_ms: 1,
                decision_bars: 10,
                trades_in_fold: net_profits.len(),
                metrics,
                passed,
            },
            trade_profits: net_profits.to_vec(),
            relative_equity: net_profits
                .iter()
                .scan(0.0, |equity, profit| {
                    *equity += profit;
                    Some(*equity)
                })
                .collect(),
        }
    }

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
    fn development_cpcv_contract_has_fifteen_purged_combinations() {
        let contract = DevelopmentCpcvPlan::for_development_bars(6_000);
        let combinations = contract.test_group_combinations();
        assert_eq!(contract.groups, 6);
        assert_eq!(contract.test_groups, 2);
        assert_eq!(contract.purge_bars, 16);
        assert_eq!(contract.embargo_bars, 3);
        assert_eq!(combinations.len(), 15);
        assert_eq!(combinations.first(), Some(&vec![0, 1]));
        assert_eq!(combinations.last(), Some(&vec![4, 5]));
    }

    #[test]
    fn sequential_walk_forward_is_chronological_and_non_overlapping() {
        let ranges = sequential_walk_forward_ranges(1_000, 5).unwrap();
        assert_eq!(ranges.len(), 5);
        assert_eq!(ranges.first(), Some(&(0, 200)));
        assert_eq!(ranges.last(), Some(&(800, 1_000)));
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1, pair[1].0);
        }
    }

    #[test]
    fn cpcv_combination_metrics_pool_component_trade_outcomes() {
        // The first regime loses on its own, but the two-group CPCV path is
        // profitable with PF > 1. A combination must be judged as that pooled
        // path, not rejected merely because one component's flag is false.
        let losing = group(&[-100.0, 25.0], false);
        let winning = group(&[200.0, 50.0], true);
        let metrics = combine_group_metrics(&[&losing, &winning], 100_000.0);

        assert_eq!(metrics.trade_count, 4);
        assert_eq!(metrics.net_profit, 175.0);
        assert!((metrics.profit_factor.unwrap() - 2.75).abs() < 1.0e-12);
        assert_eq!(metrics.winning_trades, 3);
        assert_eq!(metrics.losing_trades, 1);
    }

    #[test]
    fn m1_fold_slice_starts_at_the_exact_decision_prefix() {
        let bars = (0..10)
            .map(|minute| bar(minute * 60_000))
            .collect::<Vec<_>>();
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars,
        };

        let sliced = slice_m1_covering(&dataset, 3 * 60_000, 8 * 60_000);
        assert_eq!(sliced.bars.len(), 5);
        assert_eq!(sliced.bars.first().unwrap().timestamp_ms, 3 * 60_000);
        assert_eq!(sliced.bars.last().unwrap().timestamp_ms, 7 * 60_000);
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
            expectancy_r: 0.0,
            median_r: 0.0,
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

    #[test]
    fn recovery_median_band_skips_thin_histograms() {
        let recoveries = vec![1.0; 10];
        let ratio = recovery_to_median_ratio(2.0, &recoveries);
        assert!((ratio.unwrap() - 2.0).abs() < 1e-12);
        assert!(passes_recovery_median_band(ratio, recoveries.len()));
    }

    #[test]
    fn recovery_median_band_keeps_original_near_the_plateau() {
        let recoveries: Vec<f64> = (0..40).map(|i| 1.0 + (i as f64 - 20.0) * 0.01).collect();
        let ratio = recovery_to_median_ratio(1.05, &recoveries);
        assert!(passes_recovery_median_band(ratio, recoveries.len()));
    }

    #[test]
    fn recovery_median_band_rejects_original_on_the_tail() {
        let recoveries = vec![1.0; 40];
        let high = recovery_to_median_ratio(1.40, &recoveries);
        let low = recovery_to_median_ratio(0.70, &recoveries);
        assert!(!passes_recovery_median_band(high, recoveries.len()));
        assert!(!passes_recovery_median_band(low, recoveries.len()));
    }
}
