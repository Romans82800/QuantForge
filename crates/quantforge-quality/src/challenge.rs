use crate::{CHALLENGE_PROTOCOL, DataSplitPlan, EvidenceBinding, EvidenceError};
use quantforge_broker::{BrokerSpecError, SymbolSpecification};
use quantforge_core::{ContentHash, FloatPolicy, HashError};
use quantforge_data::{Bar, BarDataset, bar_content_hash};
use quantforge_eval::{
    BacktestMetrics, EvalError, ScoutConfig, ScoutResult, evaluate_strategy, evaluate_strategy_from,
};
use quantforge_ir::{
    BoolExpr, EntryDistancePolicy, EntryOrderPolicy, IndicatorExpr, IrError, NumericExpr,
    RiskPolicy, StopLossPolicy, StrategyIr, TakeProfitPolicy, TrailingPolicy,
};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use thiserror::Error;

pub const CHALLENGE_REPORT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChallengeConfig {
    pub scout: ScoutConfig,
    pub folds: usize,
    pub purge_bars: usize,
    pub embargo_bars: usize,
    pub minimum_validation_bars: usize,
    pub minimum_baseline_trades: usize,
    pub minimum_fold_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub minimum_passing_fold_fraction: f64,
    pub cost_multipliers: Vec<f64>,
    pub minimum_cost_survival_fraction: f64,
    pub monte_carlo_trials: usize,
    pub monte_carlo_block_length: usize,
    pub monte_carlo_minimum_p05_net_profit: f64,
    pub monte_carlo_maximum_p95_drawdown_percent: f64,
    pub neighborhood_samples: usize,
    pub parameter_perturbation_fraction: f64,
    pub minimum_neighborhood_survival_fraction: f64,
    pub minimum_neighborhood_return_ratio: f64,
    pub maximum_neighborhood_drawdown_ratio: f64,
    pub minimum_neighborhood_trade_ratio: f64,
    pub minimum_deflated_trade_sharpe: Option<f64>,
    pub evaluations_touched: u64,
    pub seed: u64,
}

impl Default for ChallengeConfig {
    fn default() -> Self {
        Self {
            scout: ScoutConfig::default(),
            folds: 5,
            purge_bars: 20,
            embargo_bars: 20,
            minimum_validation_bars: 250,
            minimum_baseline_trades: 20,
            minimum_fold_trades: 3,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 30.0,
            minimum_passing_fold_fraction: 0.6,
            cost_multipliers: vec![1.0, 1.25, 1.5, 2.0],
            minimum_cost_survival_fraction: 0.75,
            monte_carlo_trials: 1_000,
            monte_carlo_block_length: 5,
            monte_carlo_minimum_p05_net_profit: 0.0,
            monte_carlo_maximum_p95_drawdown_percent: 35.0,
            neighborhood_samples: 20,
            parameter_perturbation_fraction: 0.1,
            minimum_neighborhood_survival_fraction: 0.7,
            minimum_neighborhood_return_ratio: 0.5,
            maximum_neighborhood_drawdown_ratio: 1.5,
            minimum_neighborhood_trade_ratio: 0.5,
            minimum_deflated_trade_sharpe: None,
            evaluations_touched: 1,
            seed: 42,
        }
    }
}

impl ChallengeConfig {
    pub fn validate(&self) -> Result<(), ChallengeError> {
        self.scout.validate()?;
        if self.folds < 2 {
            return Err(ChallengeError::InvalidConfig(
                "folds must be at least two".into(),
            ));
        }
        if self.minimum_validation_bars < self.folds * 2 {
            return Err(ChallengeError::InvalidConfig(
                "minimum_validation_bars must leave at least two bars per fold".into(),
            ));
        }
        if self.monte_carlo_trials == 0
            || self.monte_carlo_block_length == 0
            || self.neighborhood_samples == 0
        {
            return Err(ChallengeError::InvalidConfig(
                "Monte Carlo trials, block length and neighborhood samples must be positive".into(),
            ));
        }
        if self.evaluations_touched == 0 {
            return Err(ChallengeError::InvalidConfig(
                "evaluations_touched must be recorded and greater than zero".into(),
            ));
        }
        for (name, value) in [
            (
                "minimum_passing_fold_fraction",
                self.minimum_passing_fold_fraction,
            ),
            (
                "minimum_cost_survival_fraction",
                self.minimum_cost_survival_fraction,
            ),
            (
                "minimum_neighborhood_survival_fraction",
                self.minimum_neighborhood_survival_fraction,
            ),
            (
                "parameter_perturbation_fraction",
                self.parameter_perturbation_fraction,
            ),
            (
                "minimum_neighborhood_trade_ratio",
                self.minimum_neighborhood_trade_ratio,
            ),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(ChallengeError::InvalidConfig(format!(
                    "{name} must be finite and between zero and one"
                )));
            }
        }
        if self.parameter_perturbation_fraction == 0.0 {
            return Err(ChallengeError::InvalidConfig(
                "parameter_perturbation_fraction must be greater than zero".into(),
            ));
        }
        for (name, value) in [
            ("minimum_return_percent", self.minimum_return_percent),
            ("minimum_profit_factor", self.minimum_profit_factor),
            ("maximum_drawdown_percent", self.maximum_drawdown_percent),
            (
                "monte_carlo_minimum_p05_net_profit",
                self.monte_carlo_minimum_p05_net_profit,
            ),
            (
                "monte_carlo_maximum_p95_drawdown_percent",
                self.monte_carlo_maximum_p95_drawdown_percent,
            ),
            (
                "minimum_neighborhood_return_ratio",
                self.minimum_neighborhood_return_ratio,
            ),
            (
                "maximum_neighborhood_drawdown_ratio",
                self.maximum_neighborhood_drawdown_ratio,
            ),
        ] {
            if !value.is_finite() {
                return Err(ChallengeError::InvalidConfig(format!(
                    "{name} must be finite"
                )));
            }
        }
        if self.minimum_profit_factor < 0.0
            || self.maximum_drawdown_percent < 0.0
            || self.monte_carlo_maximum_p95_drawdown_percent < 0.0
            || self.minimum_neighborhood_return_ratio < 0.0
            || self.maximum_neighborhood_drawdown_ratio <= 0.0
        {
            return Err(ChallengeError::InvalidConfig(
                "profit-factor, drawdown and neighborhood thresholds are outside valid bounds"
                    .into(),
            ));
        }
        if self
            .minimum_deflated_trade_sharpe
            .is_some_and(|value| !value.is_finite())
        {
            return Err(ChallengeError::InvalidConfig(
                "minimum_deflated_trade_sharpe must be finite when supplied".into(),
            ));
        }
        if self.cost_multipliers.is_empty()
            || (self.cost_multipliers[0] - 1.0).abs() > f64::EPSILON
            || self
                .cost_multipliers
                .iter()
                .any(|value| !value.is_finite() || !(1.0..=100.0).contains(value))
            || self
                .cost_multipliers
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(ChallengeError::InvalidConfig(
                "cost_multipliers must start at 1.0 and be strictly increasing finite values no greater than 100"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PurgedFoldReport {
    pub fold: usize,
    pub test_start_timestamp_ms: i64,
    pub test_end_timestamp_ms_exclusive: i64,
    pub test_bar_count: usize,
    pub purged_before_bar_count: usize,
    pub embargo_after_bar_count: usize,
    pub remaining_training_bar_count: usize,
    pub metrics: BacktestMetrics,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostShockPoint {
    pub multiplier: f64,
    pub metrics: BacktestMetrics,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostShockReport {
    pub points: Vec<CostShockPoint>,
    pub passing_points: usize,
    pub survival_fraction: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonteCarloReport {
    pub method: String,
    pub seed: u64,
    pub trials: usize,
    pub block_length: usize,
    pub p05_net_profit: f64,
    pub median_net_profit: f64,
    pub p95_drawdown_percent: f64,
    pub worst_drawdown_percent: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterNeighbor {
    pub sample: usize,
    pub strategy_fingerprint: ContentHash,
    pub metrics: BacktestMetrics,
    pub return_ratio: Option<f64>,
    pub drawdown_ratio: f64,
    pub trade_count_ratio: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterNeighborhoodReport {
    pub perturbation_fraction: f64,
    pub neighbors: Vec<ParameterNeighbor>,
    pub passing_neighbors: usize,
    pub survival_fraction: f64,
    pub passed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionBiasLevel {
    Normal,
    Elevated,
    Severe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultipleTestingReport {
    pub evaluations_touched: u64,
    pub observed_trade_sharpe_proxy: Option<f64>,
    pub expected_max_lucky_sharpe: f64,
    pub deflated_trade_sharpe_proxy: Option<f64>,
    pub warning_level: SelectionBiasLevel,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeBlocker {
    BaselineMinimumTrades,
    BaselineReturn,
    BaselineProfitFactor,
    BaselineDrawdown,
    PurgedFoldStability,
    CostShockSurvival,
    MonteCarloRobustness,
    ParameterNeighborhoodStability,
    DeflatedTradeSharpe,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChallengeReport {
    pub schema_version: u16,
    pub protocol_version: String,
    pub binding: EvidenceBinding,
    pub split_plan_hash: ContentHash,
    pub validation_data_hash: ContentHash,
    pub validation_bar_count: usize,
    pub method: String,
    pub config: ChallengeConfig,
    pub baseline: ScoutResult,
    pub purged_folds: Vec<PurgedFoldReport>,
    pub passing_fold_fraction: f64,
    pub cost_shocks: CostShockReport,
    pub monte_carlo: MonteCarloReport,
    pub parameter_neighborhood: ParameterNeighborhoodReport,
    pub multiple_testing: MultipleTestingReport,
    pub blockers: Vec<ChallengeBlocker>,
    pub passed: bool,
}

impl ChallengeReport {
    /// Applies the Challenge baseline thresholds to another engine's metrics.
    /// This is used by promotion adapters to hold the M1 Judge to the exact
    /// validation thresholds that shortlisted the candidate.
    pub fn metrics_pass_baseline(&self, metrics: &BacktestMetrics) -> bool {
        metrics_pass(metrics, self.config.minimum_baseline_trades, &self.config)
    }

    /// Reports whether the stored validation baseline itself clears every
    /// baseline threshold, independently of the remaining robustness battery.
    pub fn baseline_passed(&self) -> bool {
        self.metrics_pass_baseline(&self.baseline.metrics)
    }

    /// Recomputes every aggregate pass flag that can be derived from the stored
    /// raw report. Artifact manifests separately bind the complete report hash.
    pub fn validate_integrity(&self) -> Result<(), ChallengeError> {
        self.config.validate()?;
        if self.schema_version != CHALLENGE_REPORT_SCHEMA_VERSION
            || self.protocol_version != CHALLENGE_PROTOCOL
            || self.method != "purged_walk_forward_and_robustness_v1"
        {
            return Err(ChallengeError::InvalidReport(
                "schema, protocol or method does not match Challenge v1".into(),
            ));
        }
        let mut expected_blockers = baseline_blockers(&self.baseline.metrics, &self.config);
        if self.purged_folds.len() != self.config.folds
            || self.purged_folds.iter().any(|fold| {
                fold.passed
                    != metrics_pass(&fold.metrics, self.config.minimum_fold_trades, &self.config)
            })
        {
            return Err(ChallengeError::InvalidReport(
                "purged-fold pass flags are inconsistent".into(),
            ));
        }
        let passing_fold_fraction = fraction(
            self.purged_folds.iter().filter(|fold| fold.passed).count(),
            self.purged_folds.len(),
        );
        if !same_float(passing_fold_fraction, self.passing_fold_fraction) {
            return Err(ChallengeError::InvalidReport(
                "passing-fold fraction is inconsistent".into(),
            ));
        }
        if passing_fold_fraction < self.config.minimum_passing_fold_fraction {
            expected_blockers.push(ChallengeBlocker::PurgedFoldStability);
        }

        let passing_costs = self
            .cost_shocks
            .points
            .iter()
            .filter(|point| point.passed)
            .count();
        let cost_survival = fraction(passing_costs, self.cost_shocks.points.len());
        let cost_passed = cost_survival >= self.config.minimum_cost_survival_fraction;
        if self.cost_shocks.points.len() != self.config.cost_multipliers.len()
            || self.cost_shocks.points.iter().any(|point| {
                point.passed
                    != metrics_pass(
                        &point.metrics,
                        self.config.minimum_baseline_trades,
                        &self.config,
                    )
            })
            || self.cost_shocks.passing_points != passing_costs
            || !same_float(self.cost_shocks.survival_fraction, cost_survival)
            || self.cost_shocks.passed != cost_passed
        {
            return Err(ChallengeError::InvalidReport(
                "cost-shock aggregates are inconsistent".into(),
            ));
        }
        if !cost_passed {
            expected_blockers.push(ChallengeBlocker::CostShockSurvival);
        }

        let monte_carlo_passed = self.baseline.metrics.trade_count > 0
            && self.monte_carlo.p05_net_profit >= self.config.monte_carlo_minimum_p05_net_profit
            && self.monte_carlo.p95_drawdown_percent
                <= self.config.monte_carlo_maximum_p95_drawdown_percent;
        if self.monte_carlo.method != "moving_block_trade_bootstrap_v1"
            || self.monte_carlo.seed != self.config.seed
            || self.monte_carlo.trials != self.config.monte_carlo_trials
            || self.monte_carlo.block_length != self.config.monte_carlo_block_length
            || self.monte_carlo.passed != monte_carlo_passed
        {
            return Err(ChallengeError::InvalidReport(
                "Monte Carlo metadata or pass flag is inconsistent".into(),
            ));
        }
        if !monte_carlo_passed {
            expected_blockers.push(ChallengeBlocker::MonteCarloRobustness);
        }

        let passing_neighbors = self
            .parameter_neighborhood
            .neighbors
            .iter()
            .filter(|neighbor| neighbor.passed)
            .count();
        let neighbor_survival = fraction(
            passing_neighbors,
            self.parameter_neighborhood.neighbors.len(),
        );
        let neighbor_passed =
            neighbor_survival >= self.config.minimum_neighborhood_survival_fraction;
        if self.parameter_neighborhood.neighbors.len() != self.config.neighborhood_samples
            || self.parameter_neighborhood.passing_neighbors != passing_neighbors
            || !same_float(
                self.parameter_neighborhood.survival_fraction,
                neighbor_survival,
            )
            || self.parameter_neighborhood.passed != neighbor_passed
        {
            return Err(ChallengeError::InvalidReport(
                "parameter-neighborhood aggregates are inconsistent".into(),
            ));
        }
        if !neighbor_passed {
            expected_blockers.push(ChallengeBlocker::ParameterNeighborhoodStability);
        }

        let expected_multiple_testing = multiple_testing_report(&self.baseline, &self.config);
        if !same_multiple_testing_report(&self.multiple_testing, &expected_multiple_testing) {
            return Err(ChallengeError::InvalidReport(
                "multiple-testing report is inconsistent".into(),
            ));
        }
        if !self.multiple_testing.passed {
            expected_blockers.push(ChallengeBlocker::DeflatedTradeSharpe);
        }
        if self.blockers != expected_blockers || self.passed != expected_blockers.is_empty() {
            return Err(ChallengeError::InvalidReport(
                "overall Challenge blockers or pass flag are inconsistent".into(),
            ));
        }
        Ok(())
    }
}

pub fn run_challenge(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    split_plan: &DataSplitPlan,
    config: ChallengeConfig,
) -> Result<ChallengeReport, ChallengeError> {
    config.validate()?;
    split_plan.validate()?;
    if dataset.data_hash != split_plan.full_data_hash {
        return Err(ChallengeError::FullDataMismatch);
    }
    let validation = validation_dataset(dataset, split_plan)?;
    if validation.bars.len() < config.minimum_validation_bars {
        return Err(ChallengeError::InsufficientValidationBars {
            actual: validation.bars.len(),
            required: config.minimum_validation_bars,
        });
    }
    if validation.bars.len() < config.folds * 2 {
        return Err(ChallengeError::InsufficientFoldBars);
    }

    let binding = EvidenceBinding {
        strategy_fingerprint: strategy.structural_fingerprint(FloatPolicy::default())?,
        broker_spec_hash: broker.content_hash()?,
    };
    let baseline = evaluate_strategy(strategy, &validation, broker, &config.scout)?;
    let mut blockers = baseline_blockers(&baseline.metrics, &config);

    let purged_folds = run_purged_folds(strategy, &validation, broker, &config)?;
    let passing_fold_fraction = fraction(
        purged_folds.iter().filter(|fold| fold.passed).count(),
        purged_folds.len(),
    );
    if passing_fold_fraction < config.minimum_passing_fold_fraction {
        blockers.push(ChallengeBlocker::PurgedFoldStability);
    }

    let cost_shocks = run_cost_shocks(strategy, &validation, broker, &config)?;
    if !cost_shocks.passed {
        blockers.push(ChallengeBlocker::CostShockSurvival);
    }
    let monte_carlo = run_monte_carlo(&baseline, &config);
    if !monte_carlo.passed {
        blockers.push(ChallengeBlocker::MonteCarloRobustness);
    }
    let parameter_neighborhood =
        run_parameter_neighborhood(strategy, &validation, broker, &baseline, &config)?;
    if !parameter_neighborhood.passed {
        blockers.push(ChallengeBlocker::ParameterNeighborhoodStability);
    }
    let multiple_testing = multiple_testing_report(&baseline, &config);
    if !multiple_testing.passed {
        blockers.push(ChallengeBlocker::DeflatedTradeSharpe);
    }

    let passed = blockers.is_empty();
    Ok(ChallengeReport {
        schema_version: CHALLENGE_REPORT_SCHEMA_VERSION,
        protocol_version: CHALLENGE_PROTOCOL.into(),
        binding,
        split_plan_hash: split_plan.content_hash()?,
        validation_data_hash: validation.data_hash,
        validation_bar_count: validation.bars.len(),
        method: "purged_walk_forward_and_robustness_v1".into(),
        config,
        baseline,
        purged_folds,
        passing_fold_fraction,
        cost_shocks,
        monte_carlo,
        parameter_neighborhood,
        multiple_testing,
        blockers,
        passed,
    })
}

fn validation_dataset(
    dataset: &BarDataset,
    split_plan: &DataSplitPlan,
) -> Result<BarDataset, ChallengeError> {
    let segment = &split_plan.validation;
    let bars: Vec<_> = dataset
        .bars
        .iter()
        .filter(|bar| {
            bar.timestamp_ms >= segment.start_timestamp_ms
                && bar.timestamp_ms < segment.end_timestamp_ms_exclusive
        })
        .cloned()
        .collect();
    if bars.len() != segment.bar_count || bar_content_hash(&bars) != segment.data_hash {
        return Err(ChallengeError::ValidationDataMismatch);
    }
    Ok(derived_dataset(dataset, bars))
}

fn derived_dataset(source: &BarDataset, bars: Vec<Bar>) -> BarDataset {
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        bars,
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: source.delimiter,
        source_timezone: source.source_timezone.clone(),
    }
}

fn baseline_blockers(metrics: &BacktestMetrics, config: &ChallengeConfig) -> Vec<ChallengeBlocker> {
    let mut blockers = Vec::new();
    if metrics.trade_count < config.minimum_baseline_trades {
        blockers.push(ChallengeBlocker::BaselineMinimumTrades);
    }
    if metrics.return_percent <= config.minimum_return_percent {
        blockers.push(ChallengeBlocker::BaselineReturn);
    }
    if effective_profit_factor(metrics) < config.minimum_profit_factor {
        blockers.push(ChallengeBlocker::BaselineProfitFactor);
    }
    if metrics.max_drawdown_percent > config.maximum_drawdown_percent {
        blockers.push(ChallengeBlocker::BaselineDrawdown);
    }
    blockers
}

fn metrics_pass(
    metrics: &BacktestMetrics,
    minimum_trades: usize,
    config: &ChallengeConfig,
) -> bool {
    metrics.trade_count >= minimum_trades
        && metrics.return_percent > config.minimum_return_percent
        && effective_profit_factor(metrics) >= config.minimum_profit_factor
        && metrics.max_drawdown_percent <= config.maximum_drawdown_percent
}

fn effective_profit_factor(metrics: &BacktestMetrics) -> f64 {
    metrics
        .profit_factor
        .unwrap_or(if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::MAX
        } else {
            0.0
        })
}

fn run_purged_folds(
    strategy: &StrategyIr,
    validation: &BarDataset,
    broker: &SymbolSpecification,
    config: &ChallengeConfig,
) -> Result<Vec<PurgedFoldReport>, ChallengeError> {
    let mut reports = Vec::with_capacity(config.folds);
    for fold in 0..config.folds {
        let start = validation.bars.len() * fold / config.folds;
        let end = validation.bars.len() * (fold + 1) / config.folds;
        let context = derived_dataset(validation, validation.bars[..end].to_vec());
        let result = evaluate_strategy_from(
            strategy,
            &context,
            broker,
            &config.scout,
            validation.bars[start].timestamp_ms,
        )?;
        let purge_start = start.saturating_sub(config.purge_bars);
        let embargo_end = end
            .saturating_add(config.embargo_bars)
            .min(validation.bars.len());
        let end_timestamp = validation.bars.get(end).map_or_else(
            || {
                validation.bars[end - 1]
                    .timestamp_ms
                    .checked_add(1)
                    .ok_or(ChallengeError::TimestampOverflow)
            },
            |bar| Ok(bar.timestamp_ms),
        )?;
        reports.push(PurgedFoldReport {
            fold,
            test_start_timestamp_ms: validation.bars[start].timestamp_ms,
            test_end_timestamp_ms_exclusive: end_timestamp,
            test_bar_count: end - start,
            purged_before_bar_count: start - purge_start,
            embargo_after_bar_count: embargo_end - end,
            remaining_training_bar_count: purge_start + validation.bars.len() - embargo_end,
            passed: metrics_pass(&result.metrics, config.minimum_fold_trades, config),
            metrics: result.metrics,
        });
    }
    Ok(reports)
}

fn run_cost_shocks(
    strategy: &StrategyIr,
    validation: &BarDataset,
    broker: &SymbolSpecification,
    config: &ChallengeConfig,
) -> Result<CostShockReport, ChallengeError> {
    let mut points = Vec::with_capacity(config.cost_multipliers.len());
    for multiplier in &config.cost_multipliers {
        let (shocked_data, shocked_broker, shocked_config) =
            shocked_inputs(validation, broker, &config.scout, *multiplier)?;
        let result = evaluate_strategy(strategy, &shocked_data, &shocked_broker, &shocked_config)?;
        points.push(CostShockPoint {
            multiplier: *multiplier,
            passed: metrics_pass(&result.metrics, config.minimum_baseline_trades, config),
            metrics: result.metrics,
        });
    }
    let passing_points = points.iter().filter(|point| point.passed).count();
    let survival_fraction = fraction(passing_points, points.len());
    Ok(CostShockReport {
        points,
        passing_points,
        survival_fraction,
        passed: survival_fraction >= config.minimum_cost_survival_fraction,
    })
}

fn shocked_inputs(
    validation: &BarDataset,
    broker: &SymbolSpecification,
    scout: &ScoutConfig,
    multiplier: f64,
) -> Result<(BarDataset, SymbolSpecification, ScoutConfig), ChallengeError> {
    let mut bars = validation.bars.clone();
    for bar in &mut bars {
        if let Some(spread) = bar.spread_points {
            let shocked = (f64::from(spread) * multiplier).ceil();
            if shocked > f64::from(u32::MAX) {
                return Err(ChallengeError::SpreadOverflow);
            }
            bar.spread_points = Some(shocked as u32);
        }
    }
    let mut shocked_broker = broker.clone();
    for window in &mut shocked_broker.synthetic_spreads {
        window.spread_points *= multiplier;
    }
    let mut shocked_config = scout.clone();
    shocked_config.costs.adverse_slippage_points_per_side *= multiplier;
    shocked_config.costs.commission_per_lot_round_turn *= multiplier;
    shocked_config.costs.fallback_spread_points = shocked_config
        .costs
        .fallback_spread_points
        .map(|spread| spread * multiplier);
    Ok((
        derived_dataset(validation, bars),
        shocked_broker,
        shocked_config,
    ))
}

fn run_monte_carlo(baseline: &ScoutResult, config: &ChallengeConfig) -> MonteCarloReport {
    let profits: Vec<_> = baseline
        .trades
        .iter()
        .map(|trade| trade.net_profit)
        .collect();
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed ^ 0xa5a5_01b0_07c5_7a11);
    let mut net_profits = Vec::with_capacity(config.monte_carlo_trials);
    let mut drawdowns = Vec::with_capacity(config.monte_carlo_trials);
    if profits.is_empty() {
        net_profits.resize(config.monte_carlo_trials, 0.0);
        drawdowns.resize(config.monte_carlo_trials, 0.0);
    } else {
        for _ in 0..config.monte_carlo_trials {
            let sampled = moving_block_sample(&profits, config.monte_carlo_block_length, &mut rng);
            let (net_profit, drawdown) =
                profit_path_metrics(config.scout.initial_balance, &sampled);
            net_profits.push(net_profit);
            drawdowns.push(drawdown);
        }
    }
    net_profits.sort_by(f64::total_cmp);
    drawdowns.sort_by(f64::total_cmp);
    let p05_net_profit = quantile(&net_profits, 0.05);
    let median_net_profit = quantile(&net_profits, 0.5);
    let p95_drawdown_percent = quantile(&drawdowns, 0.95);
    let worst_drawdown_percent = *drawdowns.last().unwrap_or(&0.0);
    MonteCarloReport {
        method: "moving_block_trade_bootstrap_v1".into(),
        seed: config.seed,
        trials: config.monte_carlo_trials,
        block_length: config.monte_carlo_block_length,
        p05_net_profit,
        median_net_profit,
        p95_drawdown_percent,
        worst_drawdown_percent,
        passed: !profits.is_empty()
            && p05_net_profit >= config.monte_carlo_minimum_p05_net_profit
            && p95_drawdown_percent <= config.monte_carlo_maximum_p95_drawdown_percent,
    }
}

fn moving_block_sample(values: &[f64], block_length: usize, rng: &mut ChaCha8Rng) -> Vec<f64> {
    let mut sample = Vec::with_capacity(values.len());
    while sample.len() < values.len() {
        let start = rng.gen_range(0..values.len());
        for offset in 0..block_length {
            if sample.len() == values.len() {
                break;
            }
            sample.push(values[(start + offset) % values.len()]);
        }
    }
    sample
}

fn profit_path_metrics(initial_balance: f64, profits: &[f64]) -> (f64, f64) {
    let mut balance = initial_balance;
    let mut peak = initial_balance;
    let mut maximum_drawdown_percent = 0.0_f64;
    for profit in profits {
        balance += profit;
        peak = peak.max(balance);
        if peak > 0.0 {
            maximum_drawdown_percent =
                maximum_drawdown_percent.max((peak - balance) / peak * 100.0);
        }
    }
    (balance - initial_balance, maximum_drawdown_percent)
}

fn run_parameter_neighborhood(
    strategy: &StrategyIr,
    validation: &BarDataset,
    broker: &SymbolSpecification,
    baseline: &ScoutResult,
    config: &ChallengeConfig,
) -> Result<ParameterNeighborhoodReport, ChallengeError> {
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed ^ 0x9e37_79b9_7f4a_7c15);
    let mut neighbors = Vec::with_capacity(config.neighborhood_samples);
    for sample in 0..config.neighborhood_samples {
        let neighbor = perturb_strategy(
            strategy,
            config.parameter_perturbation_fraction,
            sample,
            &mut rng,
        )?;
        let fingerprint = neighbor.structural_fingerprint(FloatPolicy::default())?;
        let result = evaluate_strategy(&neighbor, validation, broker, &config.scout)?;
        let return_ratio = (baseline.metrics.return_percent > 0.0)
            .then_some(result.metrics.return_percent / baseline.metrics.return_percent);
        let drawdown_denominator = baseline.metrics.max_drawdown_percent.max(1.0e-9);
        let drawdown_ratio = result.metrics.max_drawdown_percent / drawdown_denominator;
        let trade_count_ratio = if baseline.metrics.trade_count == 0 {
            0.0
        } else {
            result.metrics.trade_count as f64 / baseline.metrics.trade_count as f64
        };
        let return_survived = result.metrics.return_percent > config.minimum_return_percent
            && return_ratio.is_none_or(|ratio| ratio >= config.minimum_neighborhood_return_ratio);
        let drawdown_limit = if baseline.metrics.max_drawdown_percent > 0.0 {
            baseline.metrics.max_drawdown_percent * config.maximum_neighborhood_drawdown_ratio
        } else {
            config.maximum_drawdown_percent
        };
        let passed = return_survived
            && result.metrics.max_drawdown_percent <= drawdown_limit
            && result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent
            && trade_count_ratio >= config.minimum_neighborhood_trade_ratio;
        neighbors.push(ParameterNeighbor {
            sample,
            strategy_fingerprint: fingerprint,
            metrics: result.metrics,
            return_ratio,
            drawdown_ratio,
            trade_count_ratio,
            passed,
        });
    }
    let passing_neighbors = neighbors.iter().filter(|neighbor| neighbor.passed).count();
    let survival_fraction = fraction(passing_neighbors, neighbors.len());
    Ok(ParameterNeighborhoodReport {
        perturbation_fraction: config.parameter_perturbation_fraction,
        neighbors,
        passing_neighbors,
        survival_fraction,
        passed: survival_fraction >= config.minimum_neighborhood_survival_fraction,
    })
}

fn perturb_strategy(
    strategy: &StrategyIr,
    fraction: f64,
    sample: usize,
    rng: &mut ChaCha8Rng,
) -> Result<StrategyIr, ChallengeError> {
    let mut neighbor = strategy.clone();
    neighbor.id = format!("{}-neighbor-{sample}", strategy.id);
    if let Some(entry) = &mut neighbor.entry.long {
        perturb_bool(entry, fraction, rng);
    }
    if let Some(entry) = &mut neighbor.entry.short {
        perturb_bool(entry, fraction, rng);
    }
    if let Some(exit) = &mut neighbor.exit {
        perturb_bool(exit, fraction, rng);
    }
    for filter in &mut neighbor.filters {
        perturb_bool(filter, fraction, rng);
    }
    match &mut neighbor.risk {
        RiskPolicy::FixedCurrency { amount } => perturb_positive(amount, fraction, 0.01, rng),
        RiskPolicy::PercentBalance { percent } => perturb_positive(percent, fraction, 0.01, rng),
    }
    match &mut neighbor.stops.stop_loss {
        StopLossPolicy::FixedPoints { points } => perturb_positive(points, fraction, 0.01, rng),
        StopLossPolicy::AtrMultiple { period, multiplier }
        | StopLossPolicy::RangeMultiple { period, multiplier } => {
            perturb_period(period, fraction, rng);
            perturb_positive(multiplier, fraction, 0.01, rng);
        }
    }
    match &mut neighbor.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => {
            perturb_positive(multiple, fraction, 0.01, rng)
        }
        TakeProfitPolicy::FixedPoints { points } => perturb_positive(points, fraction, 0.01, rng),
        TakeProfitPolicy::AtrMultiple { period, multiplier } => {
            perturb_period(period, fraction, rng);
            perturb_positive(multiplier, fraction, 0.01, rng);
        }
    }
    match &mut neighbor.entry.order {
        EntryOrderPolicy::Market => {}
        EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        }
        | EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        } => {
            match distance {
                EntryDistancePolicy::FixedPoints { points } => {
                    perturb_positive(points, fraction, 0.01, rng)
                }
                EntryDistancePolicy::AtrMultiple { period, multiplier }
                | EntryDistancePolicy::RangeMultiple { period, multiplier } => {
                    perturb_period(period, fraction, rng);
                    perturb_positive(multiplier, fraction, 0.01, rng);
                }
            }
            perturb_period(expiry_bars, fraction, rng);
        }
    }
    if let Some(value) = &mut neighbor.manage.break_even_at_r {
        perturb_positive(value, fraction, 0.01, rng);
    }
    if let Some(trailing) = &mut neighbor.manage.trailing {
        match trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => {
                perturb_positive(activate_at_r, fraction, 0.01, rng);
                perturb_positive(distance_r, fraction, 0.01, rng);
            }
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                period,
                multiplier,
            } => {
                perturb_positive(activate_at_r, fraction, 0.01, rng);
                perturb_period(period, fraction, rng);
                perturb_positive(multiplier, fraction, 0.01, rng);
            }
        }
    }
    if let Some(bars) = &mut neighbor.manage.time_stop_bars {
        perturb_period(bars, fraction, rng);
    }
    for partial in &mut neighbor.manage.partial_exits {
        perturb_positive(&mut partial.at_r, fraction, 0.01, rng);
        partial.fraction = (partial.fraction * random_factor(fraction, rng)).clamp(1.0e-6, 1.0);
    }
    let total_fraction: f64 = neighbor
        .manage
        .partial_exits
        .iter()
        .map(|partial| partial.fraction)
        .sum();
    if total_fraction > 1.0 {
        for partial in &mut neighbor.manage.partial_exits {
            partial.fraction /= total_fraction;
        }
    }
    let neighbor = neighbor.canonicalized(FloatPolicy::default())?;
    neighbor.validate_export_safe(quantforge_ir::IrLimits::default())?;
    Ok(neighbor)
}

fn perturb_bool(expression: &mut BoolExpr, fraction: f64, rng: &mut ChaCha8Rng) {
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            perturb_numeric(left, fraction, rng);
            perturb_numeric(right, fraction, rng);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            perturb_numeric(value, fraction, rng);
            perturb_numeric(lower, fraction, rng);
            perturb_numeric(upper, fraction, rng);
            if let (NumericExpr::Constant { value: lower }, NumericExpr::Constant { value: upper }) =
                (lower, upper)
                && *lower > *upper
            {
                std::mem::swap(lower, upper);
            }
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                perturb_bool(child, fraction, rng);
            }
        }
        BoolExpr::Not { child } => perturb_bool(child, fraction, rng),
    }
}

fn perturb_numeric(expression: &mut NumericExpr, fraction: f64, rng: &mut ChaCha8Rng) {
    match expression {
        NumericExpr::Indicator { value } => perturb_indicator(value, fraction, rng),
        NumericExpr::Constant { value } => {
            let scale = value.abs().max(1.0);
            *value += scale * rng.gen_range(-fraction..=fraction);
        }
        NumericExpr::Price { .. } | NumericExpr::Context { .. } => {}
    }
}

fn perturb_indicator(indicator: &mut IndicatorExpr, fraction: f64, rng: &mut ChaCha8Rng) {
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
    perturb_period(period, fraction, rng);
}

fn perturb_period(period: &mut u16, fraction: f64, rng: &mut ChaCha8Rng) {
    *period = (f64::from(*period) * random_factor(fraction, rng))
        .round()
        .clamp(2.0, 2_000.0) as u16;
}

fn perturb_positive(value: &mut f64, fraction: f64, minimum: f64, rng: &mut ChaCha8Rng) {
    *value = (*value * random_factor(fraction, rng)).max(minimum);
}

fn random_factor(fraction: f64, rng: &mut ChaCha8Rng) -> f64 {
    1.0 + rng.gen_range(-fraction..=fraction)
}

fn multiple_testing_report(
    baseline: &ScoutResult,
    config: &ChallengeConfig,
) -> MultipleTestingReport {
    let profits: Vec<_> = baseline
        .trades
        .iter()
        .map(|trade| trade.net_profit)
        .collect();
    let observed = trade_sharpe_proxy(&profits);
    let expected = expected_max_lucky_sharpe(config.evaluations_touched);
    let deflated = observed.map(|value| value - expected);
    let passed = config
        .minimum_deflated_trade_sharpe
        .is_none_or(|minimum| deflated.is_some_and(|value| value >= minimum));
    MultipleTestingReport {
        evaluations_touched: config.evaluations_touched,
        observed_trade_sharpe_proxy: observed,
        expected_max_lucky_sharpe: expected,
        deflated_trade_sharpe_proxy: deflated,
        warning_level: if config.evaluations_touched > 10_000 {
            SelectionBiasLevel::Severe
        } else if config.evaluations_touched > 1_500 {
            SelectionBiasLevel::Elevated
        } else {
            SelectionBiasLevel::Normal
        },
        passed,
    }
}

fn trade_sharpe_proxy(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    let deviation = variance.sqrt();
    (deviation > 1.0e-12).then_some(mean / deviation * (values.len() as f64).sqrt())
}

fn expected_max_lucky_sharpe(evaluations: u64) -> f64 {
    if evaluations <= 1 {
        return 0.0;
    }
    let logarithm = (evaluations as f64).ln();
    let leading = (2.0 * logarithm).sqrt();
    leading - (logarithm.ln() + (4.0 * PI).ln()) / (2.0 * leading)
}

fn quantile(sorted: &[f64], probability: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = (probability * (sorted.len() - 1) as f64).round() as usize;
    sorted[index]
}

fn fraction(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn same_float(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-12
}

fn same_optional_float(left: Option<f64>, right: Option<f64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => same_float(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn same_multiple_testing_report(
    stored: &MultipleTestingReport,
    expected: &MultipleTestingReport,
) -> bool {
    stored.evaluations_touched == expected.evaluations_touched
        && same_optional_float(
            stored.observed_trade_sharpe_proxy,
            expected.observed_trade_sharpe_proxy,
        )
        && same_float(
            stored.expected_max_lucky_sharpe,
            expected.expected_max_lucky_sharpe,
        )
        && same_optional_float(
            stored.deflated_trade_sharpe_proxy,
            expected.deflated_trade_sharpe_proxy,
        )
        && stored.warning_level == expected.warning_level
        && stored.passed == expected.passed
}

#[derive(Debug, Error)]
pub enum ChallengeError {
    #[error("invalid Challenge configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid Challenge report: {0}")]
    InvalidReport(String),
    #[error("input dataset does not match the split plan's full-data identity")]
    FullDataMismatch,
    #[error("validation bars do not match the split plan")]
    ValidationDataMismatch,
    #[error("validation contains {actual} bars; at least {required} are required")]
    InsufficientValidationBars { actual: usize, required: usize },
    #[error("validation does not contain at least two bars per fold")]
    InsufficientFoldBars,
    #[error("a fold timestamp cannot be represented as an exclusive boundary")]
    TimestampOverflow,
    #[error("a cost shock overflows the recorded spread representation")]
    SpreadOverflow,
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    #[error(transparent)]
    Evaluation(#[from] EvalError),
    #[error(transparent)]
    Ir(#[from] IrError),
    #[error(transparent)]
    Broker(#[from] BrokerSpecError),
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_core::STRATEGY_IR_VERSION;
    use quantforge_ir::{
        ComparisonOp, EntrySignals, ManagePolicy, PriceField, ProtectiveStops, Side, StrategyMeta,
    };

    fn dataset(count: usize) -> BarDataset {
        let bars: Vec<_> = (0..count)
            .map(|index| {
                let open = 100.0 + index as f64 * 2.0;
                Bar {
                    timestamp_ms: index as i64 * 60_000,
                    open,
                    high: open + 2.0,
                    low: open - 0.1,
                    close: open + 1.0,
                    tick_volume: 100,
                    real_volume: 0,
                    spread_points: Some(0),
                }
            })
            .collect();
        BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            bars,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        }
    }

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "challenge-fixture".into(),
            symbol: "TEST".into(),
            digits: 2,
            point: 1.0,
            tick_size: 1.0,
            tick_value: 1.0,
            contract_size: 1.0,
            volume_min: 0.01,
            volume_step: 0.01,
            volume_max: 100.0,
            stops_level_points: 0,
            freeze_level_points: 0,
            filling_modes: vec![FillingMode::FillOrKill],
            trade_mode: TradeMode::Full,
            margin_initial_per_lot: None,
            swap_mode: SwapMode::Disabled,
            swap_long: 0.0,
            swap_short: 0.0,
            triple_swap_day: DayOfWeek::Wednesday,
            swap_multipliers: Vec::new(),
            sessions: Vec::new(),
            timezone: "Etc/UTC".into(),
            account_currency: "USD".into(),
            base_currency: "USD".into(),
            profit_currency: "USD".into(),
            margin_currency: "USD".into(),
            synthetic_spreads: Vec::new(),
        }
    }

    fn strategy() -> StrategyIr {
        StrategyIr {
            id: "always-long".into(),
            version: STRATEGY_IR_VERSION,
            entry: EntrySignals {
                long: Some(BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Price {
                        field: PriceField::Close,
                        shift: 1,
                    },
                    right: NumericExpr::Constant { value: 0.0 },
                }),
                short: None,
                order: Default::default(),
            },
            exit: None,
            filters: Vec::new(),
            side: Side::LongOnly,
            risk: RiskPolicy::FixedCurrency { amount: 1.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::FixedPoints { points: 1.0 },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 1.0 },
            },
            manage: ManagePolicy::default(),
            meta: StrategyMeta {
                thesis_hint: "test".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    fn config() -> ChallengeConfig {
        ChallengeConfig {
            folds: 4,
            purge_bars: 5,
            embargo_bars: 5,
            minimum_validation_bars: 50,
            minimum_baseline_trades: 20,
            minimum_fold_trades: 5,
            monte_carlo_trials: 100,
            neighborhood_samples: 8,
            evaluations_touched: 2_000,
            ..ChallengeConfig::default()
        }
    }

    #[test]
    fn complete_challenge_is_reproducible_and_uses_only_validation_identity() {
        let dataset = dataset(500);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();
        let first = run_challenge(&strategy(), &dataset, &broker(), &plan, config()).unwrap();
        let second = run_challenge(&strategy(), &dataset, &broker(), &plan, config()).unwrap();

        assert_eq!(first, second);
        assert!(first.passed, "{:?}", first.blockers);
        assert_eq!(first.validation_data_hash, plan.validation.data_hash);
        assert_eq!(first.purged_folds.len(), 4);
        assert_eq!(first.cost_shocks.points.len(), 4);
        assert_eq!(
            first.multiple_testing.warning_level,
            SelectionBiasLevel::Elevated
        );
    }

    #[test]
    fn zero_evaluation_count_is_rejected_instead_of_hidden() {
        let mut config = config();
        config.evaluations_touched = 0;
        assert!(matches!(
            config.validate(),
            Err(ChallengeError::InvalidConfig(_))
        ));
    }

    #[test]
    fn moving_block_bootstrap_is_deterministic_and_preserves_trial_length() {
        let mut left = ChaCha8Rng::seed_from_u64(9);
        let mut right = ChaCha8Rng::seed_from_u64(9);
        let values = [1.0, -2.0, 3.0, 4.0, -1.0];
        let first = moving_block_sample(&values, 3, &mut left);
        let second = moving_block_sample(&values, 3, &mut right);
        assert_eq!(first, second);
        assert_eq!(first.len(), values.len());
    }

    #[test]
    fn report_integrity_rejects_edited_aggregate_outcomes() {
        let dataset = dataset(500);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();
        let mut report = run_challenge(&strategy(), &dataset, &broker(), &plan, config()).unwrap();
        report.passing_fold_fraction = 0.0;

        assert!(matches!(
            report.validate_integrity(),
            Err(ChallengeError::InvalidReport(_))
        ));
    }

    #[test]
    fn report_integrity_tolerates_json_scale_float_rounding_only() {
        let dataset = dataset(500);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();
        let mut report = run_challenge(&strategy(), &dataset, &broker(), &plan, config()).unwrap();
        report.multiple_testing.expected_max_lucky_sharpe += 5.0e-13;
        assert!(report.validate_integrity().is_ok());

        report.multiple_testing.expected_max_lucky_sharpe += 1.0e-6;
        assert!(matches!(
            report.validate_integrity(),
            Err(ChallengeError::InvalidReport(_))
        ));
    }
}
