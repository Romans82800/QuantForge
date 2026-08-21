//! Paired decision-timeframe comparison on Development and OOS1.
//!
//! Each deterministic candidate is evaluated on both lanes. That keeps the
//! comparison about timeframe behaviour rather than giving one lane a
//! different random strategy population.

use crate::fold_r::{FoldRStats, calendar_year_fold_r};
use crate::methodology::{FactorRecipe, build_factor_strategy, passes_screen};
use crate::model::{DiscoverError, UniversalGrammarConfig};
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::{BacktestMetrics, ScoutConfig, evaluate_strategy};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeBakeoffConfig {
    pub seed: u64,
    pub draws_per_cell: usize,
    pub entry_condition_counts: Vec<usize>,
    pub exit_condition_counts: Vec<usize>,
    pub scout: ScoutConfig,
    pub minimum_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub oos1_retention: f64,
}

impl Default for TimeframeBakeoffConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            draws_per_cell: 20,
            entry_condition_counts: vec![2],
            exit_condition_counts: vec![1],
            scout: ScoutConfig::default(),
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeBakeoffLaneRow {
    pub timeframe: String,
    pub draws: usize,
    pub screened: usize,
    pub oos1_survivors: usize,
    pub oos1_survival_rate: f64,
    pub median_is_expectancy_r: Option<f64>,
    pub median_oos1_expectancy_r: Option<f64>,
    pub median_retention: Option<f64>,
    pub median_trade_count: Option<f64>,
    pub median_drawdown_percent: Option<f64>,
    pub median_recovery_factor: Option<f64>,
    pub median_sharpe: Option<f64>,
    pub selected_oos1_expectancy_r: Option<f64>,
    pub unselected_oos1_expectancy_r: Option<f64>,
    pub selected_future_expectancy_lift_r: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeBakeoffPair {
    pub paired_comparisons: usize,
    pub h1_oos1_wins: usize,
    pub h4_oos1_wins: usize,
    pub h4_retention_lift: Option<f64>,
    pub h4_pass_rate_lift: f64,
    pub h4_selected_future_expectancy_lift_r: Option<f64>,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeBakeoffReport {
    pub evaluations: usize,
    pub rows: Vec<TimeframeBakeoffLaneRow>,
    pub pair: TimeframeBakeoffPair,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeGateConfig {
    pub minimum_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub oos1_retention: f64,
}

impl Default for TimeframeGateConfig {
    fn default() -> Self {
        Self {
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeframeSelectionMode {
    NoScreen,
    SharedGates,
    TimeframeSpecificGates,
    TradesOnly,
    ReturnOnly,
    ProfitFactorOnly,
    DrawdownOnly,
    ExpectancyTopK,
    MedianFoldExpectancyTopK,
    ExpectancyTimesTradesTopK,
    ExpectancyTimesSqrtTradesTopK,
    DrawdownTopK,
    RandomTopK,
    TradesTopK,
    ReturnTopK,
    ProfitFactorTopK,
    RecoveryFactorTopK,
    SharpeTopK,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeAblationConfig {
    pub seed: u64,
    pub draws_per_cell: usize,
    pub entry_condition_counts: Vec<usize>,
    pub exit_condition_counts: Vec<usize>,
    pub scout: ScoutConfig,
    pub shared_gates: TimeframeGateConfig,
    pub h1_gates: TimeframeGateConfig,
    pub h4_gates: TimeframeGateConfig,
}

impl Default for TimeframeAblationConfig {
    fn default() -> Self {
        let gates = TimeframeGateConfig::default();
        Self {
            seed: 42,
            draws_per_cell: 100,
            entry_condition_counts: vec![2],
            exit_condition_counts: vec![1],
            scout: ScoutConfig::default(),
            shared_gates: gates.clone(),
            h1_gates: gates.clone(),
            h4_gates: gates,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeAblationRow {
    pub selection_mode: TimeframeSelectionMode,
    pub timeframe: String,
    pub draws: usize,
    pub selected: usize,
    pub selected_rate: f64,
    pub oos1_positive: usize,
    pub oos1_positive_rate: f64,
    pub oos1_survivors: Option<usize>,
    pub oos1_survival_rate: Option<f64>,
    pub median_is_expectancy_r: Option<f64>,
    pub median_oos1_expectancy_r: Option<f64>,
    pub selected_oos1_expectancy_r: Option<f64>,
    pub unselected_oos1_expectancy_r: Option<f64>,
    pub selected_future_expectancy_lift_r: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeAblationComparison {
    pub selection_mode: TimeframeSelectionMode,
    pub paired_comparisons: usize,
    pub h1_oos1_wins: usize,
    pub h4_oos1_wins: usize,
    pub h4_selected_oos1_lift_r: Option<f64>,
    pub h4_selected_future_expectancy_lift_r: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeAblationReport {
    pub evaluations: usize,
    pub rows: Vec<TimeframeAblationRow>,
    pub comparisons: Vec<TimeframeAblationComparison>,
}

#[derive(Debug, Clone)]
pub struct TimeframeRollingWindow<'a> {
    pub label: String,
    pub h1: &'a BarDataset,
    pub h4: &'a BarDataset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeRollingRow {
    pub window: String,
    pub selection_mode: TimeframeSelectionMode,
    pub timeframe: String,
    pub draws: usize,
    pub eligible: usize,
    pub selected: usize,
    pub selected_rate: f64,
    pub future_positive: usize,
    pub future_positive_rate: f64,
    pub selected_future_trade_count: Option<f64>,
    pub unselected_future_trade_count: Option<f64>,
    pub selected_future_expectancy_r: Option<f64>,
    pub unselected_future_expectancy_r: Option<f64>,
    pub selected_future_expectancy_lift_r: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeRollingReport {
    pub evaluations: usize,
    pub rows: Vec<TimeframeRollingRow>,
}

#[derive(Debug, Clone)]
struct ArmResult {
    is_metrics: BacktestMetrics,
    oos1_metrics: BacktestMetrics,
    passed_is: bool,
    passed_oos1: bool,
    retention: Option<f64>,
}

#[derive(Debug, Clone)]
struct PairedResult {
    h1: Option<ArmResult>,
    h4: Option<ArmResult>,
}

#[derive(Debug, Clone)]
struct RawArmResult {
    is_metrics: BacktestMetrics,
    oos1_metrics: BacktestMetrics,
    retention: Option<f64>,
    fold_r: FoldRStats,
}

#[derive(Debug, Clone)]
struct RawPairedResult {
    h1: Option<RawArmResult>,
    h4: Option<RawArmResult>,
}

#[derive(Debug, Clone)]
struct RawRollingArmResult {
    is_metrics: BacktestMetrics,
    future_metrics: Vec<BacktestMetrics>,
    fold_r: FoldRStats,
}

#[derive(Debug, Clone)]
struct RawRollingPairedResult {
    h4: Option<RawRollingArmResult>,
}

pub fn run_timeframe_bakeoff(
    h1_is: &BarDataset,
    h1_oos1: &BarDataset,
    h4_is: &BarDataset,
    h4_oos1: &BarDataset,
    broker: &SymbolSpecification,
    config: TimeframeBakeoffConfig,
) -> Result<TimeframeBakeoffReport, DiscoverError> {
    validate_config(&config)?;

    let mut jobs = Vec::new();
    for &entry_conditions in &config.entry_condition_counts {
        for &exit_conditions in &config.exit_condition_counts {
            for draw in 0..config.draws_per_cell {
                jobs.push((entry_conditions, exit_conditions, draw as u64));
            }
        }
    }

    let pairs: Vec<PairedResult> = jobs
        .into_par_iter()
        .map(|(entry_conditions, exit_conditions, draw)| {
            let sequence = mix_sequence(entry_conditions, exit_conditions, draw);
            let strategy = build_factor_strategy(
                config.seed,
                sequence,
                entry_conditions,
                exit_conditions,
                FactorRecipe::SimpleMarket,
            );
            PairedResult {
                h1: evaluate_arm(&strategy, h1_is, h1_oos1, broker, &config),
                h4: evaluate_arm(&strategy, h4_is, h4_oos1, broker, &config),
            }
        })
        .collect();

    let h1 = summarize_lane("H1", &pairs, |pair| pair.h1.as_ref());
    let h4 = summarize_lane("H4", &pairs, |pair| pair.h4.as_ref());
    let pair = summarize_pair(&pairs, &h1, &h4);

    Ok(TimeframeBakeoffReport {
        evaluations: pairs.len() * 4,
        rows: vec![h1, h4],
        pair,
    })
}

pub fn run_timeframe_ablation(
    h1_is: &BarDataset,
    h1_oos1: &BarDataset,
    h4_is: &BarDataset,
    h4_oos1: &BarDataset,
    broker: &SymbolSpecification,
    config: TimeframeAblationConfig,
) -> Result<TimeframeAblationReport, DiscoverError> {
    validate_ablation_config(&config)?;

    let mut jobs = Vec::new();
    for &entry_conditions in &config.entry_condition_counts {
        for &exit_conditions in &config.exit_condition_counts {
            for draw in 0..config.draws_per_cell {
                jobs.push((entry_conditions, exit_conditions, draw as u64));
            }
        }
    }

    let pairs: Vec<RawPairedResult> = jobs
        .into_par_iter()
        .map(|(entry_conditions, exit_conditions, draw)| {
            let sequence = mix_sequence(entry_conditions, exit_conditions, draw);
            let strategy = build_factor_strategy(
                config.seed,
                sequence,
                entry_conditions,
                exit_conditions,
                FactorRecipe::SimpleMarket,
            );
            RawPairedResult {
                h1: evaluate_raw_arm(&strategy, h1_is, h1_oos1, broker, &config.scout),
                h4: evaluate_raw_arm(&strategy, h4_is, h4_oos1, broker, &config.scout),
            }
        })
        .collect();

    let modes = [
        TimeframeSelectionMode::NoScreen,
        TimeframeSelectionMode::SharedGates,
        TimeframeSelectionMode::TimeframeSpecificGates,
        TimeframeSelectionMode::TradesOnly,
        TimeframeSelectionMode::ReturnOnly,
        TimeframeSelectionMode::ProfitFactorOnly,
        TimeframeSelectionMode::DrawdownOnly,
        TimeframeSelectionMode::ExpectancyTopK,
        TimeframeSelectionMode::MedianFoldExpectancyTopK,
        TimeframeSelectionMode::ExpectancyTimesTradesTopK,
        TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK,
        TimeframeSelectionMode::DrawdownTopK,
        TimeframeSelectionMode::RandomTopK,
    ];
    let rows = modes
        .iter()
        .flat_map(|&mode| {
            [
                summarize_ablation_lane(mode, "H1", &pairs, |pair| pair.h1.as_ref(), &config),
                summarize_ablation_lane(mode, "H4", &pairs, |pair| pair.h4.as_ref(), &config),
            ]
        })
        .collect::<Vec<_>>();
    let comparisons = modes
        .iter()
        .map(|&mode| summarize_ablation_pair(mode, &pairs, &rows))
        .collect::<Vec<_>>();

    Ok(TimeframeAblationReport {
        evaluations: pairs.len() * 4,
        rows,
        comparisons,
    })
}

pub fn run_timeframe_rolling_ablation(
    h4_is: &BarDataset,
    windows: &[TimeframeRollingWindow<'_>],
    broker: &SymbolSpecification,
    config: TimeframeAblationConfig,
) -> Result<TimeframeRollingReport, DiscoverError> {
    validate_ablation_config(&config)?;
    if windows.is_empty() {
        return Err(DiscoverError::InvalidConfig(
            "timeframe rolling benchmark needs at least one validation window".into(),
        ));
    }

    let mut jobs = Vec::new();
    for &entry_conditions in &config.entry_condition_counts {
        for &exit_conditions in &config.exit_condition_counts {
            for draw in 0..config.draws_per_cell {
                jobs.push((entry_conditions, exit_conditions, draw as u64));
            }
        }
    }

    let pairs: Vec<RawRollingPairedResult> = jobs
        .into_par_iter()
        .map(|(entry_conditions, exit_conditions, draw)| {
            let sequence = mix_sequence(entry_conditions, exit_conditions, draw);
            let strategy = build_factor_strategy(
                config.seed,
                sequence,
                entry_conditions,
                exit_conditions,
                FactorRecipe::SimpleMarket,
            );
            RawRollingPairedResult {
                h4: evaluate_raw_rolling_arm(
                    &strategy,
                    h4_is,
                    windows,
                    broker,
                    &config.scout,
                    "H4",
                ),
            }
        })
        .collect();

    let modes = [
        TimeframeSelectionMode::SharedGates,
        TimeframeSelectionMode::RandomTopK,
        TimeframeSelectionMode::DrawdownTopK,
        TimeframeSelectionMode::RecoveryFactorTopK,
        TimeframeSelectionMode::TradesTopK,
        TimeframeSelectionMode::ReturnTopK,
        TimeframeSelectionMode::ProfitFactorTopK,
        TimeframeSelectionMode::SharpeTopK,
        TimeframeSelectionMode::ExpectancyTopK,
        TimeframeSelectionMode::MedianFoldExpectancyTopK,
        TimeframeSelectionMode::ExpectancyTimesTradesTopK,
        TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK,
    ];
    let mut rows = Vec::new();
    for (window_index, window) in windows.iter().enumerate() {
        for &mode in &modes {
            rows.push(summarize_rolling_lane(
                mode,
                "H4",
                window_index,
                &window.label,
                &pairs,
                |pair| pair.h4.as_ref(),
                &config,
            ));
        }
    }

    Ok(TimeframeRollingReport {
        evaluations: pairs.len() * windows.len(),
        rows,
    })
}

fn validate_ablation_config(config: &TimeframeAblationConfig) -> Result<(), DiscoverError> {
    let shared = &config.shared_gates;
    validate_config(&TimeframeBakeoffConfig {
        seed: config.seed,
        draws_per_cell: config.draws_per_cell,
        entry_condition_counts: config.entry_condition_counts.clone(),
        exit_condition_counts: config.exit_condition_counts.clone(),
        scout: config.scout.clone(),
        minimum_trades: shared.minimum_trades,
        minimum_return_percent: shared.minimum_return_percent,
        minimum_profit_factor: shared.minimum_profit_factor,
        maximum_drawdown_percent: shared.maximum_drawdown_percent,
        oos1_retention: shared.oos1_retention,
    })?;
    for (label, gates) in [
        ("shared", &config.shared_gates),
        ("H1", &config.h1_gates),
        ("H4", &config.h4_gates),
    ] {
        if !gates.minimum_return_percent.is_finite()
            || !gates.minimum_profit_factor.is_finite()
            || !gates.maximum_drawdown_percent.is_finite()
            || !gates.oos1_retention.is_finite()
            || gates.minimum_profit_factor < 0.0
            || gates.maximum_drawdown_percent < 0.0
            || !(0.0..=2.0).contains(&gates.oos1_retention)
        {
            return Err(DiscoverError::InvalidConfig(format!(
                "timeframe ablation {label} gates are invalid"
            )));
        }
    }
    Ok(())
}

fn evaluate_raw_arm(
    strategy: &quantforge_ir::StrategyIr,
    is_dataset: &BarDataset,
    oos1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    scout: &ScoutConfig,
) -> Option<RawArmResult> {
    let is_result = evaluate_strategy(strategy, is_dataset, broker, scout).ok()?;
    let oos1_metrics = evaluate_strategy(strategy, oos1_dataset, broker, scout)
        .ok()?
        .metrics;
    let is_metrics = is_result.metrics;
    let fold_r = calendar_year_fold_r(&is_result.trades);
    let retention = (is_metrics.expectancy_r > 0.0
        && is_metrics.expectancy_r.is_finite()
        && oos1_metrics.expectancy_r.is_finite())
    .then_some(oos1_metrics.expectancy_r / is_metrics.expectancy_r);
    Some(RawArmResult {
        is_metrics,
        oos1_metrics,
        retention,
        fold_r,
    })
}

fn evaluate_raw_rolling_arm(
    strategy: &quantforge_ir::StrategyIr,
    is_dataset: &BarDataset,
    windows: &[TimeframeRollingWindow<'_>],
    broker: &SymbolSpecification,
    scout: &ScoutConfig,
    timeframe: &str,
) -> Option<RawRollingArmResult> {
    let is_result = evaluate_strategy(strategy, is_dataset, broker, scout).ok()?;
    let future_metrics = windows
        .iter()
        .map(|window| {
            let dataset = if timeframe == "H1" {
                window.h1
            } else {
                window.h4
            };
            evaluate_strategy(strategy, dataset, broker, scout)
                .ok()
                .map(|result| result.metrics)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(RawRollingArmResult {
        is_metrics: is_result.metrics,
        future_metrics,
        fold_r: calendar_year_fold_r(&is_result.trades),
    })
}

fn ablation_gates<'a>(
    mode: TimeframeSelectionMode,
    timeframe: &str,
    config: &'a TimeframeAblationConfig,
) -> Option<&'a TimeframeGateConfig> {
    match mode {
        TimeframeSelectionMode::NoScreen => None,
        TimeframeSelectionMode::SharedGates
        | TimeframeSelectionMode::TradesOnly
        | TimeframeSelectionMode::ReturnOnly
        | TimeframeSelectionMode::ProfitFactorOnly
        | TimeframeSelectionMode::DrawdownOnly
        | TimeframeSelectionMode::ExpectancyTopK
        | TimeframeSelectionMode::MedianFoldExpectancyTopK
        | TimeframeSelectionMode::ExpectancyTimesTradesTopK
        | TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK
        | TimeframeSelectionMode::DrawdownTopK
        | TimeframeSelectionMode::RandomTopK
        | TimeframeSelectionMode::TradesTopK
        | TimeframeSelectionMode::ReturnTopK
        | TimeframeSelectionMode::ProfitFactorTopK
        | TimeframeSelectionMode::RecoveryFactorTopK
        | TimeframeSelectionMode::SharpeTopK => Some(&config.shared_gates),
        TimeframeSelectionMode::TimeframeSpecificGates => Some(if timeframe == "H1" {
            &config.h1_gates
        } else {
            &config.h4_gates
        }),
    }
}

fn ablation_passes_is(
    arm: &RawArmResult,
    mode: TimeframeSelectionMode,
    timeframe: &str,
    config: &TimeframeAblationConfig,
) -> bool {
    match mode {
        TimeframeSelectionMode::NoScreen => true,
        TimeframeSelectionMode::SharedGates | TimeframeSelectionMode::TimeframeSpecificGates => {
            let gates = ablation_gates(mode, timeframe, config).expect("gates exist for screen");
            passes_screen(
                &arm.is_metrics,
                gates.minimum_trades,
                gates.minimum_return_percent,
                gates.minimum_profit_factor,
                gates.maximum_drawdown_percent,
            )
        }
        TimeframeSelectionMode::TradesOnly => {
            arm.is_metrics.trade_count >= config.shared_gates.minimum_trades
        }
        TimeframeSelectionMode::ReturnOnly => {
            arm.is_metrics.return_percent > config.shared_gates.minimum_return_percent
        }
        TimeframeSelectionMode::ProfitFactorOnly => {
            let pf = arm
                .is_metrics
                .profit_factor
                .unwrap_or(if arm.is_metrics.net_profit > 0.0 {
                    f64::INFINITY
                } else {
                    0.0
                });
            pf >= config.shared_gates.minimum_profit_factor
        }
        TimeframeSelectionMode::DrawdownOnly => {
            arm.is_metrics.max_drawdown_percent <= config.shared_gates.maximum_drawdown_percent
        }
        TimeframeSelectionMode::ExpectancyTopK
        | TimeframeSelectionMode::MedianFoldExpectancyTopK
        | TimeframeSelectionMode::ExpectancyTimesTradesTopK
        | TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK
        | TimeframeSelectionMode::DrawdownTopK
        | TimeframeSelectionMode::RandomTopK
        | TimeframeSelectionMode::TradesTopK
        | TimeframeSelectionMode::ReturnTopK
        | TimeframeSelectionMode::ProfitFactorTopK
        | TimeframeSelectionMode::RecoveryFactorTopK
        | TimeframeSelectionMode::SharpeTopK => true,
    }
}

fn ablation_passes_oos1(
    arm: &RawArmResult,
    mode: TimeframeSelectionMode,
    timeframe: &str,
    config: &TimeframeAblationConfig,
) -> Option<bool> {
    let gates = ablation_gates(mode, timeframe, config)?;
    Some(
        arm.oos1_metrics.expectancy_r > 0.0
            && arm
                .retention
                .is_some_and(|value| value >= gates.oos1_retention),
    )
}

fn is_rank_mode(mode: TimeframeSelectionMode) -> bool {
    matches!(
        mode,
        TimeframeSelectionMode::ExpectancyTopK
            | TimeframeSelectionMode::MedianFoldExpectancyTopK
            | TimeframeSelectionMode::ExpectancyTimesTradesTopK
            | TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK
            | TimeframeSelectionMode::DrawdownTopK
            | TimeframeSelectionMode::RandomTopK
            | TimeframeSelectionMode::TradesTopK
            | TimeframeSelectionMode::ReturnTopK
            | TimeframeSelectionMode::ProfitFactorTopK
            | TimeframeSelectionMode::RecoveryFactorTopK
            | TimeframeSelectionMode::SharpeTopK
    )
}

fn rank_feature(mode: TimeframeSelectionMode, arm: &RawArmResult) -> f64 {
    rank_feature_values(mode, &arm.is_metrics, &arm.fold_r)
}

fn rank_feature_values(
    mode: TimeframeSelectionMode,
    is_metrics: &BacktestMetrics,
    fold_r: &FoldRStats,
) -> f64 {
    match mode {
        TimeframeSelectionMode::ExpectancyTopK => is_metrics.expectancy_r,
        TimeframeSelectionMode::MedianFoldExpectancyTopK => fold_r.median_fold_r,
        TimeframeSelectionMode::ExpectancyTimesTradesTopK => {
            is_metrics.expectancy_r * is_metrics.trade_count as f64
        }
        TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK => {
            is_metrics.expectancy_r * (is_metrics.trade_count as f64).sqrt()
        }
        TimeframeSelectionMode::DrawdownTopK => -is_metrics.max_drawdown_percent,
        TimeframeSelectionMode::RandomTopK => f64::NEG_INFINITY,
        TimeframeSelectionMode::TradesTopK => is_metrics.trade_count as f64,
        TimeframeSelectionMode::ReturnTopK => is_metrics.return_percent,
        TimeframeSelectionMode::ProfitFactorTopK => {
            is_metrics
                .profit_factor
                .unwrap_or(if is_metrics.net_profit > 0.0 {
                    f64::INFINITY
                } else {
                    0.0
                })
        }
        TimeframeSelectionMode::RecoveryFactorTopK => is_metrics.recovery_factor(),
        TimeframeSelectionMode::SharpeTopK => is_metrics.sharpe_ratio.unwrap_or(f64::NEG_INFINITY),
        _ => f64::NEG_INFINITY,
    }
}

fn select_ranked_top_k<'a>(
    mode: TimeframeSelectionMode,
    timeframe: &str,
    arms: &[&'a RawArmResult],
    config: &TimeframeAblationConfig,
) -> Vec<&'a RawArmResult> {
    let eligible = arms
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, arm)| {
            ablation_passes_is(arm, TimeframeSelectionMode::SharedGates, timeframe, config)
        })
        .collect::<Vec<_>>();
    let budget = rank_budget(eligible.len());
    let mut ranked = eligible;
    if mode == TimeframeSelectionMode::RandomTopK {
        ranked.sort_by_key(|(index, _)| random_rank(config.seed, timeframe, *index));
    } else {
        ranked.sort_by(|(left_index, left), (right_index, right)| {
            rank_feature(mode, right)
                .total_cmp(&rank_feature(mode, left))
                .then_with(|| left_index.cmp(right_index))
        });
    }
    ranked
        .into_iter()
        .take(budget)
        .map(|(_, arm)| arm)
        .collect()
}

fn select_rolling_ranked_top_k<'a>(
    mode: TimeframeSelectionMode,
    timeframe: &str,
    arms: &[&'a RawRollingArmResult],
    config: &TimeframeAblationConfig,
) -> Vec<&'a RawRollingArmResult> {
    let eligible = arms
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, arm)| {
            passes_screen(
                &arm.is_metrics,
                config.shared_gates.minimum_trades,
                config.shared_gates.minimum_return_percent,
                config.shared_gates.minimum_profit_factor,
                config.shared_gates.maximum_drawdown_percent,
            )
        })
        .collect::<Vec<_>>();
    let budget = rank_budget(eligible.len());
    let mut ranked = eligible;
    if mode == TimeframeSelectionMode::RandomTopK {
        ranked.sort_by_key(|(index, _)| random_rank(config.seed, timeframe, *index));
    } else {
        ranked.sort_by(|(left_index, left), (right_index, right)| {
            rank_feature_values(mode, &right.is_metrics, &right.fold_r)
                .total_cmp(&rank_feature_values(mode, &left.is_metrics, &left.fold_r))
                .then_with(|| left_index.cmp(right_index))
        });
    }
    ranked
        .into_iter()
        .take(budget)
        .map(|(_, arm)| arm)
        .collect()
}

fn rank_budget(eligible_arms: usize) -> usize {
    (eligible_arms + 4) / 5
}

fn summarize_rolling_lane(
    mode: TimeframeSelectionMode,
    timeframe: &str,
    window_index: usize,
    window_label: &str,
    pairs: &[RawRollingPairedResult],
    select: impl Fn(&RawRollingPairedResult) -> Option<&RawRollingArmResult>,
    config: &TimeframeAblationConfig,
) -> TimeframeRollingRow {
    let arms: Vec<&RawRollingArmResult> = pairs.iter().filter_map(select).collect();
    let eligible = arms
        .iter()
        .filter(|arm| {
            passes_screen(
                &arm.is_metrics,
                config.shared_gates.minimum_trades,
                config.shared_gates.minimum_return_percent,
                config.shared_gates.minimum_profit_factor,
                config.shared_gates.maximum_drawdown_percent,
            )
        })
        .count();
    let selected: Vec<&RawRollingArmResult> = if is_rank_mode(mode) {
        select_rolling_ranked_top_k(mode, timeframe, &arms, config)
    } else {
        arms.iter()
            .copied()
            .filter(|arm| {
                passes_screen(
                    &arm.is_metrics,
                    config.shared_gates.minimum_trades,
                    config.shared_gates.minimum_return_percent,
                    config.shared_gates.minimum_profit_factor,
                    config.shared_gates.maximum_drawdown_percent,
                )
            })
            .collect()
    };
    let unselected: Vec<&RawRollingArmResult> = arms
        .iter()
        .copied()
        .filter(|arm| {
            !selected
                .iter()
                .any(|selected| std::ptr::eq(*selected, *arm))
        })
        .collect();
    let selected_future = selected
        .iter()
        .map(|arm| arm.future_metrics[window_index].expectancy_r)
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let selected_future_trades = selected
        .iter()
        .map(|arm| arm.future_metrics[window_index].trade_count as f64)
        .collect::<Vec<_>>();
    let unselected_future_trades = unselected
        .iter()
        .map(|arm| arm.future_metrics[window_index].trade_count as f64)
        .collect::<Vec<_>>();
    let unselected_future = unselected
        .iter()
        .map(|arm| arm.future_metrics[window_index].expectancy_r)
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let future_positive = selected_future.iter().filter(|value| **value > 0.0).count();
    let selected_median = median(&selected_future);
    let unselected_median = median(&unselected_future);
    TimeframeRollingRow {
        window: window_label.into(),
        selection_mode: mode,
        timeframe: timeframe.into(),
        draws: arms.len(),
        eligible,
        selected: selected.len(),
        selected_rate: rate(selected.len(), arms.len()),
        future_positive,
        future_positive_rate: rate(future_positive, selected.len()),
        selected_future_trade_count: median(&selected_future_trades),
        unselected_future_trade_count: median(&unselected_future_trades),
        selected_future_expectancy_r: selected_median,
        unselected_future_expectancy_r: unselected_median,
        selected_future_expectancy_lift_r: match (selected_median, unselected_median) {
            (Some(selected), Some(unselected)) => Some(selected - unselected),
            _ => None,
        },
    }
}

fn summarize_ablation_lane(
    mode: TimeframeSelectionMode,
    timeframe: &str,
    pairs: &[RawPairedResult],
    select: impl Fn(&RawPairedResult) -> Option<&RawArmResult>,
    config: &TimeframeAblationConfig,
) -> TimeframeAblationRow {
    let arms: Vec<&RawArmResult> = pairs.iter().filter_map(select).collect();
    let selected: Vec<&RawArmResult> = if is_rank_mode(mode) {
        select_ranked_top_k(mode, timeframe, &arms, config)
    } else {
        arms.iter()
            .copied()
            .filter(|arm| ablation_passes_is(arm, mode, timeframe, config))
            .collect()
    };
    let unselected: Vec<&RawArmResult> = arms
        .iter()
        .copied()
        .filter(|arm| {
            !selected
                .iter()
                .any(|selected| std::ptr::eq(*selected, *arm))
        })
        .collect();
    let selected_oos = selected
        .iter()
        .map(|arm| arm.oos1_metrics.expectancy_r)
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let unselected_oos = unselected
        .iter()
        .map(|arm| arm.oos1_metrics.expectancy_r)
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let oos1_positive = selected_oos.iter().filter(|value| **value > 0.0).count();
    let (oos1_survivors, oos1_survival_rate) = if ablation_gates(mode, timeframe, config).is_some()
    {
        let survivors = selected
            .iter()
            .filter(|arm| ablation_passes_oos1(arm, mode, timeframe, config) == Some(true))
            .count();
        (Some(survivors), Some(rate(survivors, selected.len())))
    } else {
        (None, None)
    };

    TimeframeAblationRow {
        selection_mode: mode,
        timeframe: timeframe.into(),
        draws: arms.len(),
        selected: selected.len(),
        selected_rate: rate(selected.len(), arms.len()),
        oos1_positive,
        oos1_positive_rate: rate(oos1_positive, selected.len()),
        oos1_survivors,
        oos1_survival_rate,
        median_is_expectancy_r: median(
            &arms
                .iter()
                .map(|arm| arm.is_metrics.expectancy_r)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        median_oos1_expectancy_r: median(
            &arms
                .iter()
                .map(|arm| arm.oos1_metrics.expectancy_r)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        selected_oos1_expectancy_r: median(&selected_oos),
        unselected_oos1_expectancy_r: median(&unselected_oos),
        selected_future_expectancy_lift_r: match (median(&selected_oos), median(&unselected_oos)) {
            (Some(selected), Some(unselected)) => Some(selected - unselected),
            _ => None,
        },
    }
}

fn summarize_ablation_pair(
    mode: TimeframeSelectionMode,
    pairs: &[RawPairedResult],
    rows: &[TimeframeAblationRow],
) -> TimeframeAblationComparison {
    let paired: Vec<(&RawArmResult, &RawArmResult)> = pairs
        .iter()
        .filter_map(|pair| Some((pair.h1.as_ref()?, pair.h4.as_ref()?)))
        .collect();
    let h1_oos1_wins = paired
        .iter()
        .filter(|(h1, h4)| h1.oos1_metrics.expectancy_r > h4.oos1_metrics.expectancy_r)
        .count();
    let h4_oos1_wins = paired
        .iter()
        .filter(|(h1, h4)| h4.oos1_metrics.expectancy_r > h1.oos1_metrics.expectancy_r)
        .count();
    let h1_row = rows
        .iter()
        .find(|row| row.selection_mode == mode && row.timeframe == "H1")
        .expect("ablation H1 row exists");
    let h4_row = rows
        .iter()
        .find(|row| row.selection_mode == mode && row.timeframe == "H4")
        .expect("ablation H4 row exists");
    TimeframeAblationComparison {
        selection_mode: mode,
        paired_comparisons: paired.len(),
        h1_oos1_wins,
        h4_oos1_wins,
        h4_selected_oos1_lift_r: match (
            h4_row.selected_oos1_expectancy_r,
            h1_row.selected_oos1_expectancy_r,
        ) {
            (Some(h4), Some(h1)) => Some(h4 - h1),
            _ => None,
        },
        h4_selected_future_expectancy_lift_r: h4_row.selected_future_expectancy_lift_r,
    }
}

fn validate_config(config: &TimeframeBakeoffConfig) -> Result<(), DiscoverError> {
    if config.draws_per_cell == 0 {
        return Err(DiscoverError::InvalidConfig(
            "timeframe bakeoff draws_per_cell must be > 0".into(),
        ));
    }
    if config.entry_condition_counts.is_empty() || config.exit_condition_counts.is_empty() {
        return Err(DiscoverError::InvalidConfig(
            "timeframe bakeoff needs entry and exit condition counts".into(),
        ));
    }
    if config
        .entry_condition_counts
        .iter()
        .any(|count| !(2..=UniversalGrammarConfig::MAX_ENTRY_CONDITIONS).contains(count))
    {
        return Err(DiscoverError::InvalidConfig(
            "timeframe bakeoff entry counts must be within 2..=4".into(),
        ));
    }
    if config
        .exit_condition_counts
        .iter()
        .any(|count| !(1..=3).contains(count))
    {
        return Err(DiscoverError::InvalidConfig(
            "timeframe bakeoff exit counts must be within 1..=3".into(),
        ));
    }
    if !config.oos1_retention.is_finite() || !(0.0..=2.0).contains(&config.oos1_retention) {
        return Err(DiscoverError::InvalidConfig(
            "timeframe bakeoff OOS1 retention must be between 0 and 2".into(),
        ));
    }
    Ok(())
}

fn evaluate_arm(
    strategy: &quantforge_ir::StrategyIr,
    is_dataset: &BarDataset,
    oos1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &TimeframeBakeoffConfig,
) -> Option<ArmResult> {
    let is_metrics = evaluate_strategy(strategy, is_dataset, broker, &config.scout)
        .ok()?
        .metrics;
    let oos1_metrics = evaluate_strategy(strategy, oos1_dataset, broker, &config.scout)
        .ok()?
        .metrics;
    let passed_is = passes_screen(
        &is_metrics,
        config.minimum_trades,
        config.minimum_return_percent,
        config.minimum_profit_factor,
        config.maximum_drawdown_percent,
    );
    let retention = (is_metrics.expectancy_r > 0.0
        && is_metrics.expectancy_r.is_finite()
        && oos1_metrics.expectancy_r.is_finite())
    .then_some(oos1_metrics.expectancy_r / is_metrics.expectancy_r);
    let passed_oos1 = passed_is
        && oos1_metrics.expectancy_r > 0.0
        && retention.is_some_and(|value| value >= config.oos1_retention);
    Some(ArmResult {
        is_metrics,
        oos1_metrics,
        passed_is,
        passed_oos1,
        retention,
    })
}

fn summarize_lane(
    timeframe: &str,
    pairs: &[PairedResult],
    select: impl Fn(&PairedResult) -> Option<&ArmResult>,
) -> TimeframeBakeoffLaneRow {
    let arms: Vec<&ArmResult> = pairs.iter().filter_map(select).collect();
    let screened: Vec<&ArmResult> = arms.iter().copied().filter(|arm| arm.passed_is).collect();
    let survivors = screened.iter().filter(|arm| arm.passed_oos1).count();
    let selected_oos: Vec<f64> = screened
        .iter()
        .map(|arm| arm.oos1_metrics.expectancy_r)
        .filter(|value| value.is_finite())
        .collect();
    let unselected_oos: Vec<f64> = arms
        .iter()
        .filter(|arm| !arm.passed_is)
        .map(|arm| arm.oos1_metrics.expectancy_r)
        .filter(|value| value.is_finite())
        .collect();
    let selected_future_expectancy_lift_r = match (median(&selected_oos), median(&unselected_oos)) {
        (Some(selected), Some(unselected)) => Some(selected - unselected),
        _ => None,
    };

    TimeframeBakeoffLaneRow {
        timeframe: timeframe.into(),
        draws: arms.len(),
        screened: screened.len(),
        oos1_survivors: survivors,
        oos1_survival_rate: rate(survivors, screened.len()),
        median_is_expectancy_r: median(
            &arms
                .iter()
                .map(|arm| arm.is_metrics.expectancy_r)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        median_oos1_expectancy_r: median(
            &arms
                .iter()
                .map(|arm| arm.oos1_metrics.expectancy_r)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        median_retention: median(
            &arms
                .iter()
                .filter_map(|arm| arm.retention)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        median_trade_count: median(
            &arms
                .iter()
                .map(|arm| arm.is_metrics.trade_count as f64)
                .collect::<Vec<_>>(),
        ),
        median_drawdown_percent: median(
            &arms
                .iter()
                .map(|arm| arm.is_metrics.max_drawdown_percent)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        median_recovery_factor: median(
            &arms
                .iter()
                .map(|arm| arm.is_metrics.recovery_factor())
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        median_sharpe: median(
            &arms
                .iter()
                .filter_map(|arm| arm.is_metrics.sharpe_ratio)
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>(),
        ),
        selected_oos1_expectancy_r: median(&selected_oos),
        unselected_oos1_expectancy_r: median(&unselected_oos),
        selected_future_expectancy_lift_r,
    }
}

fn summarize_pair(
    pairs: &[PairedResult],
    h1: &TimeframeBakeoffLaneRow,
    h4: &TimeframeBakeoffLaneRow,
) -> TimeframeBakeoffPair {
    let paired: Vec<(&ArmResult, &ArmResult)> = pairs
        .iter()
        .filter_map(|pair| Some((pair.h1.as_ref()?, pair.h4.as_ref()?)))
        .collect();
    let h1_oos1_wins = paired
        .iter()
        .filter(|(h1, h4)| h1.oos1_metrics.expectancy_r > h4.oos1_metrics.expectancy_r)
        .count();
    let h4_oos1_wins = paired
        .iter()
        .filter(|(h1, h4)| h4.oos1_metrics.expectancy_r > h1.oos1_metrics.expectancy_r)
        .count();
    let recommendation = if h4.oos1_survival_rate > h1.oos1_survival_rate
        && h4.median_retention.unwrap_or(f64::NEG_INFINITY)
            >= h1.median_retention.unwrap_or(f64::NEG_INFINITY)
        && h4.screened >= 5
    {
        "H4 is the stronger research lane on this Development/OOS1 comparison; verify it on a second historical origin before changing defaults.".into()
    } else if h1.oos1_survival_rate > h4.oos1_survival_rate
        && h1.median_retention.unwrap_or(f64::NEG_INFINITY)
            >= h4.median_retention.unwrap_or(f64::NEG_INFINITY)
    {
        "H1 is the stronger research lane on this Development/OOS1 comparison; keep H4 as an exploratory lane.".into()
    } else {
        "The lanes are inconclusive; do not promote either timeframe from one comparison.".into()
    };
    TimeframeBakeoffPair {
        paired_comparisons: paired.len(),
        h1_oos1_wins,
        h4_oos1_wins,
        h4_retention_lift: match (h4.median_retention, h1.median_retention) {
            (Some(h4), Some(h1)) => Some(h4 - h1),
            _ => None,
        },
        h4_pass_rate_lift: h4.oos1_survival_rate - h1.oos1_survival_rate,
        h4_selected_future_expectancy_lift_r: match (
            h4.selected_future_expectancy_lift_r,
            h1.selected_future_expectancy_lift_r,
        ) {
            (Some(h4), Some(h1)) => Some(h4 - h1),
            _ => None,
        },
        recommendation,
    }
}

fn mix_sequence(entry_conditions: usize, exit_conditions: usize, draw: u64) -> u64 {
    (entry_conditions as u64)
        .wrapping_mul(1_000_003)
        .wrapping_add((exit_conditions as u64).wrapping_mul(10_007))
        .wrapping_add(draw)
}

fn random_rank(seed: u64, timeframe: &str, index: usize) -> u64 {
    let timeframe_hash = timeframe
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ byte as u64).wrapping_mul(0x100000001b3)
        });
    let mut value = seed ^ timeframe_hash ^ (index as u64).wrapping_mul(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn rate(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    } else {
        sorted[middle]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_or_invalid_comparison_config() {
        let mut config = TimeframeBakeoffConfig::default();
        config.draws_per_cell = 0;
        assert!(run_timeframe_bakeoff_config_validation(&config).is_err());
        config.draws_per_cell = 1;
        config.entry_condition_counts = vec![5];
        assert!(run_timeframe_bakeoff_config_validation(&config).is_err());
    }

    fn run_timeframe_bakeoff_config_validation(
        config: &TimeframeBakeoffConfig,
    ) -> Result<(), DiscoverError> {
        validate_config(config)
    }
}
