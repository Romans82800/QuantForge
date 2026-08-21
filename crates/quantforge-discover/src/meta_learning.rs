//! Leakage-safe meta-selection over strategy evidence.
//!
//! This module never evaluates a strategy and never reads market data. It only
//! consumes IS-side evidence plus outcomes that were produced later by an
//! independent validation run. The type and partition checks are deliberately
//! strict: fitting is allowed only on the training scope, and sealed windows
//! are excluded from training by construction.

use crate::model::{Elite, FamilyStyle};
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::{BacktestMetrics, ScoutConfig, evaluate_strategy};
use quantforge_ir::StrategyIr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaWindowRole {
    Development,
    Validation,
    Sealed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaDatasetScope {
    Training,
    Validation,
    Sealed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaLearningConfig {
    /// The primary forward label horizon recommended for model training.
    pub primary_horizon_months: u32,
    /// A longer confirmation horizon for the final research decision.
    pub confirmation_horizon_months: u32,
    /// Minimum future trades required for a positive survival label.
    pub minimum_future_trades: usize,
    /// Minimum future mean R required for a positive survival label.
    pub minimum_future_expectancy_r: f64,
    /// Minimum future/IS expectancy retention required for survival.
    pub minimum_retention: f64,
    /// Fraction of candidates selected by probability for precision@K and
    /// selected-vs-unselected expectancy comparisons.
    pub top_k_fraction: f64,
    /// Asset identity is deliberately excluded from the initial model.
    pub include_asset_identity: bool,
    /// Logistic-regression training iterations.
    pub iterations: usize,
    pub learning_rate: f64,
    pub l2_penalty: f64,
    pub calibration_bins: usize,
}

impl Default for MetaLearningConfig {
    fn default() -> Self {
        Self {
            primary_horizon_months: 6,
            confirmation_horizon_months: 12,
            minimum_future_trades: 10,
            minimum_future_expectancy_r: 0.0,
            minimum_retention: 0.70,
            top_k_fraction: 0.20,
            include_asset_identity: false,
            iterations: 800,
            learning_rate: 0.15,
            l2_penalty: 0.01,
            calibration_bins: 10,
        }
    }
}

impl MetaLearningConfig {
    pub fn validate(&self) -> Result<(), MetaLearningError> {
        if self.primary_horizon_months == 0 || self.confirmation_horizon_months == 0 {
            return Err(MetaLearningError::InvalidConfig(
                "meta horizons must be positive".into(),
            ));
        }
        if self.minimum_future_trades == 0 {
            return Err(MetaLearningError::InvalidConfig(
                "minimum_future_trades must be positive".into(),
            ));
        }
        if !self.minimum_future_expectancy_r.is_finite()
            || !self.minimum_retention.is_finite()
            || self.minimum_retention < 0.0
        {
            return Err(MetaLearningError::InvalidConfig(
                "future label thresholds must be finite and non-negative where applicable".into(),
            ));
        }
        if !self.top_k_fraction.is_finite()
            || !(0.0..=1.0).contains(&self.top_k_fraction)
            || self.top_k_fraction == 0.0
        {
            return Err(MetaLearningError::InvalidConfig(
                "top_k_fraction must be in (0, 1]".into(),
            ));
        }
        if self.iterations == 0
            || !self.learning_rate.is_finite()
            || self.learning_rate <= 0.0
            || !self.l2_penalty.is_finite()
            || self.l2_penalty < 0.0
            || !(2..=100).contains(&self.calibration_bins)
        {
            return Err(MetaLearningError::InvalidConfig(
                "logistic and calibration settings are invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWindow {
    pub id: String,
    pub role: MetaWindowRole,
    /// The last timestamp visible when the IS-only features were produced.
    pub feature_cutoff_timestamp_ms: i64,
    pub label_start_timestamp_ms: i64,
    pub label_end_timestamp_ms: i64,
    pub horizon_months: u32,
}

impl MetaWindow {
    fn validate(&self) -> Result<(), MetaLearningError> {
        if self.id.trim().is_empty() {
            return Err(MetaLearningError::InvalidWindow(
                "meta window id cannot be empty".into(),
            ));
        }
        if self.feature_cutoff_timestamp_ms >= self.label_start_timestamp_ms
            || self.label_start_timestamp_ms >= self.label_end_timestamp_ms
            || self.horizon_months == 0
        {
            return Err(MetaLearningError::InvalidWindow(format!(
                "window {} is not strictly chronological",
                self.id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaFeatureRecord {
    pub strategy_id: String,
    pub asset: Option<String>,
    pub feature_cutoff_timestamp_ms: i64,
    pub family: String,
    pub complexity: usize,
    pub entry_conditions: usize,
    pub exit_conditions: usize,
    pub is_expectancy_r: f64,
    pub is_return_percent: f64,
    pub is_trade_count: usize,
    pub is_profit_factor: Option<f64>,
    pub is_sharpe: Option<f64>,
    pub is_drawdown_percent: f64,
    pub is_recovery_factor: Option<f64>,
    pub is_return_drawdown_ratio: f64,
    pub is_median_r: f64,
    pub fold_median_expectancy_r: f64,
    pub fold_spread_r: f64,
    pub fold_passing_fraction: Option<f64>,
    pub fold_has_negative: bool,
    pub parameter_median_ratio: Option<f64>,
    pub neighborhood_survival_fraction: Option<f64>,
    pub neighborhood_samples: usize,
    pub m1_return_retention: Option<f64>,
    pub m1_trade_retention: Option<f64>,
    pub m1_drawdown_expansion: Option<f64>,
}

impl MetaFeatureRecord {
    /// Extract only evidence available at the IS cutoff from a databank elite.
    /// OOS1 and sealed metrics are intentionally not read here.
    pub fn from_elite(
        strategy_id: impl Into<String>,
        asset: Option<String>,
        feature_cutoff_timestamp_ms: i64,
        elite: &Elite,
    ) -> Self {
        let metrics = &elite.metrics;
        let robustness = elite.robustness.as_ref();
        let parameter = robustness.map(|value| &value.parameter_neighborhood);
        let m1 = robustness.map(|value| &value.m1_retention);
        let fold_passing_fraction = robustness
            .and_then(|value| value.sequential_walk_forward.as_ref())
            .or_else(|| robustness.map(|value| &value.walk_forward))
            .map(|value| value.passing_fraction);
        // `Elite::is_expectancy` is the account-currency expectancy used by
        // the discovery gates. Retention labels are expressed in per-trade R,
        // so the meta-layer must use the explicit R metric here.
        let is_expectancy_r = metrics.expectancy_r;
        let return_drawdown = if metrics.max_drawdown_percent > 1.0e-12 {
            metrics.return_percent / metrics.max_drawdown_percent
        } else if metrics.return_percent > 0.0 {
            1_000.0
        } else {
            0.0
        };
        Self {
            strategy_id: strategy_id.into(),
            asset,
            feature_cutoff_timestamp_ms,
            family: family_label(&elite.strategy.meta.thesis_hint),
            complexity: elite.complexity,
            entry_conditions: elite.descriptor.entry_conditions,
            exit_conditions: elite.descriptor.exit_conditions,
            is_expectancy_r,
            is_return_percent: metrics.return_percent,
            is_trade_count: metrics.trade_count,
            is_profit_factor: metrics.profit_factor,
            is_sharpe: elite.observed_trade_sharpe.or(metrics.sharpe_ratio),
            is_drawdown_percent: metrics.max_drawdown_percent,
            is_recovery_factor: finite_option(metrics.recovery_factor()),
            is_return_drawdown_ratio: return_drawdown,
            is_median_r: metrics.median_r,
            fold_median_expectancy_r: elite.fold_r.median_fold_r,
            fold_spread_r: elite.fold_r.fold_spread,
            fold_passing_fraction,
            fold_has_negative: elite.fold_r.has_negative_fold,
            parameter_median_ratio: parameter.and_then(|value| value.original_recovery_to_median),
            neighborhood_survival_fraction: parameter.map(|value| value.survival_fraction),
            neighborhood_samples: parameter.map_or(0, |value| value.samples_evaluated),
            m1_return_retention: m1.and_then(|value| value.return_retention),
            m1_trade_retention: m1.and_then(|value| value.trade_retention),
            m1_drawdown_expansion: m1.and_then(|value| value.drawdown_expansion),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaFutureOutcome {
    pub window_id: String,
    pub future_expectancy_r: f64,
    pub future_trade_count: usize,
    #[serde(default)]
    pub future_return_percent: Option<f64>,
    #[serde(default)]
    pub future_profit_factor: Option<f64>,
    #[serde(default)]
    pub future_drawdown_percent: Option<f64>,
}

impl MetaFutureOutcome {
    pub fn from_metrics(window_id: impl Into<String>, metrics: &BacktestMetrics) -> Self {
        Self {
            window_id: window_id.into(),
            future_expectancy_r: metrics.expectancy_r,
            future_trade_count: metrics.trade_count,
            future_return_percent: Some(metrics.return_percent),
            future_profit_factor: metrics.profit_factor,
            future_drawdown_percent: Some(metrics.max_drawdown_percent),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCandidate {
    pub features: MetaFeatureRecord,
    pub outcomes: Vec<MetaFutureOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaLearningInput {
    #[serde(default)]
    pub config: MetaLearningConfig,
    pub windows: Vec<MetaWindow>,
    pub candidates: Vec<MetaCandidate>,
}

/// A later replay window backed by a dataset that ends at or before the label
/// boundary. The dataset may include pre-window warm-up bars.
#[derive(Debug)]
pub struct MetaReplayWindow<'a> {
    pub window: MetaWindow,
    pub dataset: &'a BarDataset,
}

/// One real strategy candidate to replay. `features` must contain only the
/// evidence available at its origin cutoff.
#[derive(Debug, Clone)]
pub struct MetaReplayCandidate {
    pub features: MetaFeatureRecord,
    pub strategy: StrategyIr,
    pub broker: SymbolSpecification,
    pub scout: ScoutConfig,
}

pub struct MetaReplayOrigin<'a> {
    pub windows: Vec<MetaReplayWindow<'a>>,
    pub candidates: Vec<MetaReplayCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaLabel {
    pub survived: bool,
    pub retention_r: Option<f64>,
    pub future_expectancy_r: f64,
    pub future_trade_count: usize,
    pub future_return_percent: Option<f64>,
    pub future_profit_factor: Option<f64>,
    pub future_drawdown_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaDatasetRow {
    pub candidate_id: String,
    pub window_id: String,
    pub role: MetaWindowRole,
    pub feature_cutoff_timestamp_ms: i64,
    pub label_start_timestamp_ms: i64,
    pub label_end_timestamp_ms: i64,
    pub features: MetaFeatureRecord,
    pub label: MetaLabel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaDataset {
    pub scope: MetaDatasetScope,
    pub rows: Vec<MetaDatasetRow>,
}

#[derive(Debug, Error)]
pub enum MetaLearningError {
    #[error("invalid meta-learning configuration: {0}")]
    InvalidConfig(String),
    #[error("invalid meta-learning window: {0}")]
    InvalidWindow(String),
    #[error("invalid meta-learning dataset: {0}")]
    InvalidDataset(String),
    #[error("meta-learning model cannot be fit: {0}")]
    InsufficientTrainingData(String),
}

pub fn build_meta_dataset(
    input: &MetaLearningInput,
    scope: MetaDatasetScope,
    max_training_label_end_timestamp_ms: Option<i64>,
) -> Result<MetaDataset, MetaLearningError> {
    input.config.validate()?;
    validate_windows(&input.windows)?;
    validate_candidates(&input.candidates)?;
    let selected_windows = input
        .windows
        .iter()
        .filter(|window| match scope {
            MetaDatasetScope::Training => {
                window.role != MetaWindowRole::Sealed
                    && max_training_label_end_timestamp_ms
                        .is_none_or(|cutoff| window.label_end_timestamp_ms <= cutoff)
            }
            MetaDatasetScope::Validation => window.role == MetaWindowRole::Validation,
            MetaDatasetScope::Sealed => window.role == MetaWindowRole::Sealed,
        })
        .collect::<Vec<_>>();
    build_dataset_from_windows(&input.candidates, selected_windows, scope, &input.config)
}

/// Replay real strategies on later windows and produce the serializable input
/// consumed by `run_meta_walk_forward`. This function is research-only: it
/// does not alter Discover, promote strategies, or make trading decisions.
pub fn build_meta_learning_input_from_replay(
    origins: &[MetaReplayOrigin<'_>],
    config: MetaLearningConfig,
) -> Result<MetaLearningInput, MetaLearningError> {
    config.validate()?;
    let mut windows = Vec::new();
    let mut candidates = Vec::new();
    let mut window_ids = BTreeSet::new();
    for origin in origins {
        if origin.windows.is_empty() || origin.candidates.is_empty() {
            return Err(MetaLearningError::InvalidDataset(
                "each replay origin needs candidates and later windows".into(),
            ));
        }
        for replay_window in &origin.windows {
            replay_window.window.validate()?;
            if !window_ids.insert(replay_window.window.id.clone()) {
                return Err(MetaLearningError::InvalidWindow(format!(
                    "duplicate replay window id {}",
                    replay_window.window.id
                )));
            }
            let last_timestamp = replay_window
                .dataset
                .bars
                .last()
                .map(|bar| bar.timestamp_ms)
                .ok_or_else(|| {
                    MetaLearningError::InvalidDataset(format!(
                        "replay window {} has no bars",
                        replay_window.window.id
                    ))
                })?;
            if last_timestamp >= replay_window.window.label_end_timestamp_ms {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "replay window {} contains bars at or after its label end",
                    replay_window.window.id
                )));
            }
            if !replay_window
                .dataset
                .bars
                .iter()
                .any(|bar| bar.timestamp_ms >= replay_window.window.label_start_timestamp_ms)
            {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "replay window {} has no bars inside its label range",
                    replay_window.window.id
                )));
            }
            windows.push(replay_window.window.clone());
        }
        let mut origin_candidate_ids = BTreeSet::new();
        for candidate in &origin.candidates {
            if candidate.features.feature_cutoff_timestamp_ms
                >= origin
                    .windows
                    .iter()
                    .map(|window| window.window.label_start_timestamp_ms)
                    .min()
                    .unwrap_or(i64::MAX)
            {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "candidate {} is not strictly before its first label window",
                    candidate.features.strategy_id
                )));
            }
            if !origin_candidate_ids.insert(candidate.features.strategy_id.clone()) {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "duplicate candidate {} inside replay origin",
                    candidate.features.strategy_id
                )));
            }
            let mut outcomes = Vec::new();
            for replay_window in &origin.windows {
                let metrics = evaluate_strategy(
                    &candidate.strategy,
                    replay_window.dataset,
                    &candidate.broker,
                    &candidate.scout,
                )
                .map_err(|error| {
                    MetaLearningError::InvalidDataset(format!(
                        "candidate {} failed replay on {}: {error}",
                        candidate.features.strategy_id, replay_window.window.id
                    ))
                })?;
                outcomes.push(outcome_from_replay(
                    &replay_window.window,
                    &metrics.trades,
                    metrics.metrics.initial_balance,
                ));
            }
            candidates.push(MetaCandidate {
                features: candidate.features.clone(),
                outcomes,
            });
        }
    }
    let input = MetaLearningInput {
        config,
        windows,
        candidates,
    };
    validate_windows(&input.windows)?;
    validate_candidates(&input.candidates)?;
    Ok(input)
}

fn outcome_from_replay(
    window: &MetaWindow,
    trades: &[quantforge_eval::Trade],
    initial_balance: f64,
) -> MetaFutureOutcome {
    let trades = trades
        .iter()
        .filter(|trade| {
            trade.entry_timestamp_ms >= window.label_start_timestamp_ms
                && trade.entry_timestamp_ms < window.label_end_timestamp_ms
                && trade.exit_timestamp_ms < window.label_end_timestamp_ms
        })
        .collect::<Vec<_>>();
    let net_profit = trades.iter().map(|trade| trade.net_profit).sum::<f64>();
    let expectancy_r = if trades.is_empty() {
        0.0
    } else {
        trades.iter().map(|trade| trade.r_multiple).sum::<f64>() / trades.len() as f64
    };
    let gross_profit = trades
        .iter()
        .filter(|trade| trade.net_profit > 0.0)
        .map(|trade| trade.net_profit)
        .sum::<f64>();
    let gross_loss = trades
        .iter()
        .filter(|trade| trade.net_profit < 0.0)
        .map(|trade| trade.net_profit.abs())
        .sum::<f64>();
    MetaFutureOutcome {
        window_id: window.id.clone(),
        future_expectancy_r: expectancy_r,
        future_trade_count: trades.len(),
        future_return_percent: (initial_balance > 0.0 && initial_balance.is_finite())
            .then_some(net_profit / initial_balance * 100.0),
        future_profit_factor: (gross_loss > 0.0).then_some(gross_profit / gross_loss),
        future_drawdown_percent: None,
    }
}

fn build_dataset_from_windows(
    candidates: &[MetaCandidate],
    windows: Vec<&MetaWindow>,
    scope: MetaDatasetScope,
    config: &MetaLearningConfig,
) -> Result<MetaDataset, MetaLearningError> {
    let mut rows = Vec::new();
    for window in windows {
        for candidate in candidates.iter().filter(|candidate| {
            candidate.features.feature_cutoff_timestamp_ms == window.feature_cutoff_timestamp_ms
        }) {
            let outcome = candidate
                .outcomes
                .iter()
                .find(|outcome| outcome.window_id == window.id)
                .ok_or_else(|| {
                    MetaLearningError::InvalidDataset(format!(
                        "candidate {} has no outcome for window {}",
                        candidate.features.strategy_id, window.id
                    ))
                })?;
            if candidate.features.feature_cutoff_timestamp_ms >= window.label_start_timestamp_ms {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "candidate {} has features at or after label start for window {}",
                    candidate.features.strategy_id, window.id
                )));
            }
            let retention_r = if candidate.features.is_expectancy_r > 1.0e-12 {
                finite_option(outcome.future_expectancy_r / candidate.features.is_expectancy_r)
            } else {
                None
            };
            let survived = outcome.future_trade_count >= config.minimum_future_trades
                && outcome.future_expectancy_r >= config.minimum_future_expectancy_r
                && retention_r.is_some_and(|value| value >= config.minimum_retention);
            rows.push(MetaDatasetRow {
                candidate_id: candidate.features.strategy_id.clone(),
                window_id: window.id.clone(),
                role: window.role,
                feature_cutoff_timestamp_ms: window.feature_cutoff_timestamp_ms,
                label_start_timestamp_ms: window.label_start_timestamp_ms,
                label_end_timestamp_ms: window.label_end_timestamp_ms,
                features: candidate.features.clone(),
                label: MetaLabel {
                    survived,
                    retention_r,
                    future_expectancy_r: outcome.future_expectancy_r,
                    future_trade_count: outcome.future_trade_count,
                    future_return_percent: outcome.future_return_percent,
                    future_profit_factor: outcome.future_profit_factor,
                    future_drawdown_percent: outcome.future_drawdown_percent,
                },
            });
        }
    }
    Ok(MetaDataset { scope, rows })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaLogisticModel {
    pub feature_names: Vec<String>,
    pub family_values: Vec<String>,
    pub asset_values: Vec<String>,
    pub include_asset_identity: bool,
    pub means: Vec<f64>,
    pub scales: Vec<f64>,
    pub coefficients: Vec<f64>,
    pub intercept: f64,
    pub training_rows: usize,
    pub training_positive_rate: f64,
}

impl MetaLogisticModel {
    pub fn fit(
        dataset: &MetaDataset,
        config: &MetaLearningConfig,
    ) -> Result<Self, MetaLearningError> {
        config.validate()?;
        if dataset.scope != MetaDatasetScope::Training {
            return Err(MetaLearningError::InsufficientTrainingData(
                "only a Training dataset can fit the meta-model".into(),
            ));
        }
        if dataset.rows.len() < 4 {
            return Err(MetaLearningError::InsufficientTrainingData(
                "at least four training rows are required".into(),
            ));
        }
        let positives = dataset.rows.iter().filter(|row| row.label.survived).count();
        if positives == 0 || positives == dataset.rows.len() {
            return Err(MetaLearningError::InsufficientTrainingData(
                "training labels must contain both survivors and failures".into(),
            ));
        }
        let encoder = FeatureEncoder::fit(dataset, config.include_asset_identity);
        let raw = dataset
            .rows
            .iter()
            .map(|row| encoder.encode(&row.features))
            .collect::<Vec<_>>();
        let (means, scales) = standardization(&raw);
        let design = raw
            .iter()
            .map(|row| standardize(row, &means, &scales))
            .collect::<Vec<_>>();
        let mut coefficients = vec![0.0; encoder.names.len()];
        let prior = positives as f64 / dataset.rows.len() as f64;
        let mut intercept = (prior / (1.0 - prior)).ln();
        for _ in 0..config.iterations {
            let mut gradient = vec![0.0; coefficients.len()];
            let mut intercept_gradient = 0.0;
            for (x, row) in design.iter().zip(&dataset.rows) {
                let probability = sigmoid(intercept + dot(&coefficients, x));
                let error = probability - if row.label.survived { 1.0 } else { 0.0 };
                intercept_gradient += error;
                for (index, value) in x.iter().enumerate() {
                    gradient[index] += error * value;
                }
            }
            let scale = 1.0 / dataset.rows.len() as f64;
            intercept -= config.learning_rate * intercept_gradient * scale;
            for (index, coefficient) in coefficients.iter_mut().enumerate() {
                let penalty = config.l2_penalty * *coefficient;
                *coefficient -= config.learning_rate * (gradient[index] * scale + penalty);
            }
        }
        Ok(Self {
            feature_names: encoder.names,
            family_values: encoder.family_values,
            asset_values: encoder.asset_values,
            include_asset_identity: config.include_asset_identity,
            means,
            scales,
            coefficients,
            intercept,
            training_rows: dataset.rows.len(),
            training_positive_rate: prior,
        })
    }

    pub fn predict_probability(&self, features: &MetaFeatureRecord) -> f64 {
        let encoder = FeatureEncoder {
            names: self.feature_names.clone(),
            family_values: self.family_values.clone(),
            asset_values: self.asset_values.clone(),
            include_asset_identity: self.include_asset_identity,
        };
        let raw = encoder.encode(features);
        let standardized = standardize(&raw, &self.means, &self.scales);
        sigmoid(self.intercept + dot(&self.coefficients, &standardized))
    }

    pub fn evaluate(
        &self,
        dataset: &MetaDataset,
        config: &MetaLearningConfig,
    ) -> Result<MetaEvaluationReport, MetaLearningError> {
        config.validate()?;
        let mut predictions = dataset
            .rows
            .iter()
            .map(|row| MetaPrediction {
                candidate_id: row.candidate_id.clone(),
                window_id: row.window_id.clone(),
                probability: self.predict_probability(&row.features),
                survived: row.label.survived,
                future_expectancy_r: row.label.future_expectancy_r,
                future_trade_count: row.label.future_trade_count,
                retention_r: row.label.retention_r,
            })
            .collect::<Vec<_>>();
        predictions.sort_by(|left, right| {
            right
                .probability
                .total_cmp(&left.probability)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        let selected_count = if predictions.is_empty() {
            0
        } else {
            ((predictions.len() as f64 * config.top_k_fraction).ceil() as usize)
                .max(1)
                .min(predictions.len())
        };
        let selected = &predictions[..selected_count];
        let unselected = &predictions[selected_count..];
        let selected_expectancy = mean(selected.iter().map(|row| row.future_expectancy_r));
        let unselected_expectancy = mean(unselected.iter().map(|row| row.future_expectancy_r));
        Ok(MetaEvaluationReport {
            scope: dataset.scope,
            window_ids: dataset
                .rows
                .iter()
                .map(|row| row.window_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            rows: predictions.len(),
            positives: predictions.iter().filter(|row| row.survived).count(),
            selected: selected.len(),
            precision_at_k: if selected.is_empty() {
                None
            } else {
                Some(
                    selected.iter().filter(|row| row.survived).count() as f64
                        / selected.len() as f64,
                )
            },
            auc: auc(&predictions),
            brier_score: brier_score(&predictions),
            expected_calibration_error: expected_calibration_error(
                &predictions,
                config.calibration_bins,
            ),
            calibration: calibration_bins(&predictions, config.calibration_bins),
            selected_future_expectancy_r: selected_expectancy,
            unselected_future_expectancy_r: unselected_expectancy,
            selected_future_expectancy_lift_r: selected_expectancy
                .zip(unselected_expectancy)
                .map(|(selected, unselected)| selected - unselected),
            selected_future_trade_count: mean(
                selected.iter().map(|row| row.future_trade_count as f64),
            ),
            unselected_future_trade_count: mean(
                unselected.iter().map(|row| row.future_trade_count as f64),
            ),
            selected_retention_r: mean(selected.iter().filter_map(|row| row.retention_r)),
            unselected_retention_r: mean(unselected.iter().filter_map(|row| row.retention_r)),
        })
    }
}

/// Interpretable ridge regression over IS-only evidence with future
/// expectancy as the continuous target. Unlike the survival classifier, this
/// model is trained to rank candidates by the quantity we actually want to
/// improve: later per-trade R.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExpectancyModel {
    pub feature_names: Vec<String>,
    pub family_values: Vec<String>,
    pub asset_values: Vec<String>,
    pub include_asset_identity: bool,
    pub means: Vec<f64>,
    pub scales: Vec<f64>,
    pub coefficients: Vec<f64>,
    pub intercept: f64,
    pub target_mean: f64,
    pub target_scale: f64,
    pub training_rows: usize,
    pub training_mean_future_expectancy_r: f64,
}

impl MetaExpectancyModel {
    pub fn fit(
        dataset: &MetaDataset,
        config: &MetaLearningConfig,
    ) -> Result<Self, MetaLearningError> {
        config.validate()?;
        if dataset.scope != MetaDatasetScope::Training {
            return Err(MetaLearningError::InsufficientTrainingData(
                "only a Training dataset can fit the meta-model".into(),
            ));
        }
        if dataset.rows.len() < 4 {
            return Err(MetaLearningError::InsufficientTrainingData(
                "at least four training rows are required".into(),
            ));
        }
        let encoder = FeatureEncoder::fit(dataset, config.include_asset_identity);
        let raw = dataset
            .rows
            .iter()
            .map(|row| encoder.encode(&row.features))
            .collect::<Vec<_>>();
        let (means, scales) = standardization(&raw);
        let design = raw
            .iter()
            .map(|row| standardize(row, &means, &scales))
            .collect::<Vec<_>>();
        let targets = dataset
            .rows
            .iter()
            .map(|row| row.label.future_expectancy_r)
            .collect::<Vec<_>>();
        let target_mean = targets.iter().sum::<f64>() / targets.len() as f64;
        let target_scale = (targets
            .iter()
            .map(|target| (target - target_mean).powi(2))
            .sum::<f64>()
            / targets.len() as f64)
            .sqrt()
            .max(1.0e-9);
        let scaled_targets = targets
            .iter()
            .map(|target| (target - target_mean) / target_scale)
            .collect::<Vec<_>>();
        let mut coefficients = vec![0.0; encoder.names.len()];
        let mut intercept = 0.0;
        // The regression objective has a larger stable step-size range than
        // logistic loss when many standardized evidence fields are present.
        // Scale the configured research learning rate by the feature width so
        // ordinary inputs cannot diverge into infinite predictions.
        let learning_rate = config.learning_rate / encoder.names.len().max(1) as f64;
        for _ in 0..config.iterations {
            let mut gradient = vec![0.0; coefficients.len()];
            let mut intercept_gradient = 0.0;
            for (x, target) in design.iter().zip(&scaled_targets) {
                let error = intercept + dot(&coefficients, x) - target;
                intercept_gradient += error;
                for (index, value) in x.iter().enumerate() {
                    gradient[index] += error * value;
                }
            }
            let scale = 1.0 / dataset.rows.len() as f64;
            intercept -= learning_rate * intercept_gradient * scale;
            for (index, coefficient) in coefficients.iter_mut().enumerate() {
                let penalty = config.l2_penalty * *coefficient;
                *coefficient -= learning_rate * (gradient[index] * scale + penalty);
            }
        }
        Ok(Self {
            feature_names: encoder.names,
            family_values: encoder.family_values,
            asset_values: encoder.asset_values,
            include_asset_identity: config.include_asset_identity,
            means,
            scales,
            coefficients,
            intercept,
            target_mean,
            target_scale,
            training_rows: dataset.rows.len(),
            training_mean_future_expectancy_r: target_mean,
        })
    }

    pub fn predict_expectancy(&self, features: &MetaFeatureRecord) -> f64 {
        let encoder = FeatureEncoder {
            names: self.feature_names.clone(),
            family_values: self.family_values.clone(),
            asset_values: self.asset_values.clone(),
            include_asset_identity: self.include_asset_identity,
        };
        let raw = encoder.encode(features);
        let standardized = standardize(&raw, &self.means, &self.scales);
        self.target_mean
            + self.target_scale * (self.intercept + dot(&self.coefficients, &standardized))
    }

    pub fn evaluate(
        &self,
        dataset: &MetaDataset,
        config: &MetaLearningConfig,
    ) -> Result<MetaExpectancyEvaluationReport, MetaLearningError> {
        config.validate()?;
        let mut predictions = dataset
            .rows
            .iter()
            .map(|row| MetaExpectancyPrediction {
                candidate_id: row.candidate_id.clone(),
                window_id: row.window_id.clone(),
                predicted_future_expectancy_r: self.predict_expectancy(&row.features),
                actual_future_expectancy_r: row.label.future_expectancy_r,
                future_trade_count: row.label.future_trade_count,
                survived: row.label.survived,
                retention_r: row.label.retention_r,
            })
            .collect::<Vec<_>>();
        predictions.sort_by(|left, right| {
            right
                .predicted_future_expectancy_r
                .total_cmp(&left.predicted_future_expectancy_r)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        let selected_count = if predictions.is_empty() {
            0
        } else {
            ((predictions.len() as f64 * config.top_k_fraction).ceil() as usize)
                .max(1)
                .min(predictions.len())
        };
        let selected = &predictions[..selected_count];
        let unselected = &predictions[selected_count..];
        let selected_expectancy = mean(selected.iter().map(|row| row.actual_future_expectancy_r));
        let unselected_expectancy =
            mean(unselected.iter().map(|row| row.actual_future_expectancy_r));
        Ok(MetaExpectancyEvaluationReport {
            scope: dataset.scope,
            window_ids: dataset
                .rows
                .iter()
                .map(|row| row.window_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            rows: predictions.len(),
            selected: selected.len(),
            positive_expectancy_rate: if predictions.is_empty() {
                None
            } else {
                Some(
                    predictions
                        .iter()
                        .filter(|row| row.actual_future_expectancy_r > 0.0)
                        .count() as f64
                        / predictions.len() as f64,
                )
            },
            selected_positive_expectancy_rate: if selected.is_empty() {
                None
            } else {
                Some(
                    selected
                        .iter()
                        .filter(|row| row.actual_future_expectancy_r > 0.0)
                        .count() as f64
                        / selected.len() as f64,
                )
            },
            rank_correlation: rank_correlation(
                predictions
                    .iter()
                    .map(|row| row.predicted_future_expectancy_r)
                    .collect::<Vec<_>>()
                    .as_slice(),
                predictions
                    .iter()
                    .map(|row| row.actual_future_expectancy_r)
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            rmse: rmse(
                predictions
                    .iter()
                    .map(|row| row.predicted_future_expectancy_r)
                    .collect::<Vec<_>>()
                    .as_slice(),
                predictions
                    .iter()
                    .map(|row| row.actual_future_expectancy_r)
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            selected_future_expectancy_r: selected_expectancy,
            unselected_future_expectancy_r: unselected_expectancy,
            selected_future_expectancy_lift_r: selected_expectancy
                .zip(unselected_expectancy)
                .map(|(selected, unselected)| selected - unselected),
            selected_future_trade_count: mean(
                selected.iter().map(|row| row.future_trade_count as f64),
            ),
            unselected_future_trade_count: mean(
                unselected.iter().map(|row| row.future_trade_count as f64),
            ),
            selected_retention_r: mean(selected.iter().filter_map(|row| row.retention_r)),
            unselected_retention_r: mean(unselected.iter().filter_map(|row| row.retention_r)),
            selected_survival_rate: if selected.is_empty() {
                None
            } else {
                Some(
                    selected.iter().filter(|row| row.survived).count() as f64
                        / selected.len() as f64,
                )
            },
            unselected_survival_rate: if unselected.is_empty() {
                None
            } else {
                Some(
                    unselected.iter().filter(|row| row.survived).count() as f64
                        / unselected.len() as f64,
                )
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExpectancyPrediction {
    pub candidate_id: String,
    pub window_id: String,
    pub predicted_future_expectancy_r: f64,
    pub actual_future_expectancy_r: f64,
    pub future_trade_count: usize,
    pub survived: bool,
    pub retention_r: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExpectancyEvaluationReport {
    pub scope: MetaDatasetScope,
    pub window_ids: Vec<String>,
    pub rows: usize,
    pub selected: usize,
    pub positive_expectancy_rate: Option<f64>,
    pub selected_positive_expectancy_rate: Option<f64>,
    pub rank_correlation: Option<f64>,
    pub rmse: Option<f64>,
    pub selected_future_expectancy_r: Option<f64>,
    pub unselected_future_expectancy_r: Option<f64>,
    pub selected_future_expectancy_lift_r: Option<f64>,
    pub selected_future_trade_count: Option<f64>,
    pub unselected_future_trade_count: Option<f64>,
    pub selected_retention_r: Option<f64>,
    pub unselected_retention_r: Option<f64>,
    pub selected_survival_rate: Option<f64>,
    pub unselected_survival_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExpectancyWalkForwardEpisode {
    pub evaluation_window_id: String,
    pub evaluation_role: MetaWindowRole,
    pub training_rows: usize,
    pub training_window_ids: Vec<String>,
    pub model: MetaExpectancyModel,
    pub report: MetaExpectancyEvaluationReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExpectancyWalkForwardReport {
    pub config: MetaLearningConfig,
    pub episodes: Vec<MetaExpectancyWalkForwardEpisode>,
    pub final_sealed_evaluation: Option<MetaExpectancyEvaluationReport>,
}

pub fn run_meta_expectancy_walk_forward(
    input: &MetaLearningInput,
) -> Result<MetaExpectancyWalkForwardReport, MetaLearningError> {
    input.config.validate()?;
    validate_windows(&input.windows)?;
    validate_candidates(&input.candidates)?;
    let mut evaluation_windows = input
        .windows
        .iter()
        .filter(|window| window.role != MetaWindowRole::Development)
        .collect::<Vec<_>>();
    evaluation_windows.sort_by(|left, right| {
        left.feature_cutoff_timestamp_ms
            .cmp(&right.feature_cutoff_timestamp_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    let sealed_count = evaluation_windows
        .iter()
        .filter(|window| window.role == MetaWindowRole::Sealed)
        .count();
    if sealed_count > 1 {
        return Err(MetaLearningError::InvalidWindow(
            "only one final sealed evaluation window is allowed per run".into(),
        ));
    }
    let mut episodes = Vec::new();
    for evaluation_window in evaluation_windows {
        let training = build_meta_dataset(
            input,
            MetaDatasetScope::Training,
            Some(evaluation_window.feature_cutoff_timestamp_ms),
        )?;
        let model = MetaExpectancyModel::fit(&training, &input.config)?;
        let scope = if evaluation_window.role == MetaWindowRole::Sealed {
            MetaDatasetScope::Sealed
        } else {
            MetaDatasetScope::Validation
        };
        let evaluation = build_dataset_from_windows(
            &input.candidates,
            vec![evaluation_window],
            scope,
            &input.config,
        )?;
        let report = model.evaluate(&evaluation, &input.config)?;
        let training_window_ids = training
            .rows
            .iter()
            .map(|row| row.window_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        episodes.push(MetaExpectancyWalkForwardEpisode {
            evaluation_window_id: evaluation_window.id.clone(),
            evaluation_role: evaluation_window.role,
            training_rows: training.rows.len(),
            training_window_ids,
            model,
            report,
        });
    }
    let final_sealed_evaluation = episodes
        .iter()
        .find(|episode| episode.evaluation_role == MetaWindowRole::Sealed)
        .map(|episode| episode.report.clone());
    Ok(MetaExpectancyWalkForwardReport {
        config: input.config.clone(),
        episodes,
        final_sealed_evaluation,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPrediction {
    pub candidate_id: String,
    pub window_id: String,
    pub probability: f64,
    pub survived: bool,
    pub future_expectancy_r: f64,
    pub future_trade_count: usize,
    pub retention_r: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCalibrationBin {
    pub lower_probability: f64,
    pub upper_probability: f64,
    pub rows: usize,
    pub mean_probability: Option<f64>,
    pub observed_survival_rate: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaEvaluationReport {
    pub scope: MetaDatasetScope,
    pub window_ids: Vec<String>,
    pub rows: usize,
    pub positives: usize,
    pub selected: usize,
    pub auc: Option<f64>,
    pub precision_at_k: Option<f64>,
    pub brier_score: Option<f64>,
    pub expected_calibration_error: Option<f64>,
    pub calibration: Vec<MetaCalibrationBin>,
    pub selected_future_expectancy_r: Option<f64>,
    pub unselected_future_expectancy_r: Option<f64>,
    pub selected_future_expectancy_lift_r: Option<f64>,
    pub selected_future_trade_count: Option<f64>,
    pub unselected_future_trade_count: Option<f64>,
    pub selected_retention_r: Option<f64>,
    pub unselected_retention_r: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWalkForwardEpisode {
    pub evaluation_window_id: String,
    pub evaluation_role: MetaWindowRole,
    pub training_rows: usize,
    pub training_window_ids: Vec<String>,
    pub model: MetaLogisticModel,
    pub report: MetaEvaluationReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaWalkForwardReport {
    pub config: MetaLearningConfig,
    pub episodes: Vec<MetaWalkForwardEpisode>,
    pub final_sealed_evaluation: Option<MetaEvaluationReport>,
}

pub fn run_meta_walk_forward(
    input: &MetaLearningInput,
) -> Result<MetaWalkForwardReport, MetaLearningError> {
    input.config.validate()?;
    validate_windows(&input.windows)?;
    validate_candidates(&input.candidates)?;
    let mut evaluation_windows = input
        .windows
        .iter()
        .filter(|window| window.role != MetaWindowRole::Development)
        .collect::<Vec<_>>();
    evaluation_windows.sort_by(|left, right| {
        left.feature_cutoff_timestamp_ms
            .cmp(&right.feature_cutoff_timestamp_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    let sealed_count = evaluation_windows
        .iter()
        .filter(|window| window.role == MetaWindowRole::Sealed)
        .count();
    if sealed_count > 1 {
        return Err(MetaLearningError::InvalidWindow(
            "only one final sealed evaluation window is allowed per run".into(),
        ));
    }
    let mut episodes = Vec::new();
    for evaluation_window in evaluation_windows {
        let training = build_meta_dataset(
            input,
            MetaDatasetScope::Training,
            Some(evaluation_window.feature_cutoff_timestamp_ms),
        )?;
        let model = MetaLogisticModel::fit(&training, &input.config)?;
        let scope = if evaluation_window.role == MetaWindowRole::Sealed {
            MetaDatasetScope::Sealed
        } else {
            MetaDatasetScope::Validation
        };
        let evaluation = build_dataset_from_windows(
            &input.candidates,
            vec![evaluation_window],
            scope,
            &input.config,
        )?;
        let report = model.evaluate(&evaluation, &input.config)?;
        let training_window_ids = training
            .rows
            .iter()
            .map(|row| row.window_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        episodes.push(MetaWalkForwardEpisode {
            evaluation_window_id: evaluation_window.id.clone(),
            evaluation_role: evaluation_window.role,
            training_rows: training.rows.len(),
            training_window_ids,
            model,
            report,
        });
    }
    let final_sealed_evaluation = episodes
        .iter()
        .find(|episode| episode.evaluation_role == MetaWindowRole::Sealed)
        .map(|episode| episode.report.clone());
    Ok(MetaWalkForwardReport {
        config: input.config.clone(),
        episodes,
        final_sealed_evaluation,
    })
}

fn validate_windows(windows: &[MetaWindow]) -> Result<(), MetaLearningError> {
    let mut ids = BTreeSet::new();
    for window in windows {
        window.validate()?;
        if !ids.insert(window.id.clone()) {
            return Err(MetaLearningError::InvalidWindow(format!(
                "duplicate meta window id {}",
                window.id
            )));
        }
    }
    if windows
        .iter()
        .filter(|window| window.role == MetaWindowRole::Sealed)
        .count()
        > 1
    {
        return Err(MetaLearningError::InvalidWindow(
            "only one final sealed evaluation window is allowed".into(),
        ));
    }
    Ok(())
}

fn validate_candidates(candidates: &[MetaCandidate]) -> Result<(), MetaLearningError> {
    let mut keys = BTreeSet::new();
    for candidate in candidates {
        if candidate.features.strategy_id.trim().is_empty() {
            return Err(MetaLearningError::InvalidDataset(
                "candidate strategy_id cannot be empty".into(),
            ));
        }
        if !feature_record_is_finite(&candidate.features)
            || candidate.features.feature_cutoff_timestamp_ms <= 0
        {
            return Err(MetaLearningError::InvalidDataset(format!(
                "candidate {} contains invalid IS-only features",
                candidate.features.strategy_id
            )));
        }
        if !keys.insert((
            candidate.features.strategy_id.clone(),
            candidate.features.feature_cutoff_timestamp_ms,
        )) {
            return Err(MetaLearningError::InvalidDataset(format!(
                "duplicate candidate {} at the same feature cutoff",
                candidate.features.strategy_id
            )));
        }
        let mut outcome_ids = BTreeSet::new();
        for outcome in &candidate.outcomes {
            if outcome.window_id.trim().is_empty()
                || !outcome.future_expectancy_r.is_finite()
                || outcome
                    .future_return_percent
                    .is_some_and(|value| !value.is_finite())
                || outcome
                    .future_profit_factor
                    .is_some_and(|value| !value.is_finite())
                || outcome
                    .future_drawdown_percent
                    .is_some_and(|value| !value.is_finite())
            {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "candidate {} contains an invalid future outcome",
                    candidate.features.strategy_id
                )));
            }
            if !outcome_ids.insert(outcome.window_id.clone()) {
                return Err(MetaLearningError::InvalidDataset(format!(
                    "candidate {} repeats outcome window {}",
                    candidate.features.strategy_id, outcome.window_id
                )));
            }
        }
    }
    Ok(())
}

fn feature_record_is_finite(features: &MetaFeatureRecord) -> bool {
    [
        features.is_expectancy_r,
        features.is_return_percent,
        features.is_profit_factor.unwrap_or(0.0),
        features.is_sharpe.unwrap_or(0.0),
        features.is_drawdown_percent,
        features.is_recovery_factor.unwrap_or(0.0),
        features.is_return_drawdown_ratio,
        features.is_median_r,
        features.fold_median_expectancy_r,
        features.fold_spread_r,
        features.fold_passing_fraction.unwrap_or(0.0),
        features.parameter_median_ratio.unwrap_or(0.0),
        features.neighborhood_survival_fraction.unwrap_or(0.0),
        features.m1_return_retention.unwrap_or(0.0),
        features.m1_trade_retention.unwrap_or(0.0),
        features.m1_drawdown_expansion.unwrap_or(0.0),
    ]
    .into_iter()
    .all(f64::is_finite)
}

struct FeatureEncoder {
    names: Vec<String>,
    family_values: Vec<String>,
    asset_values: Vec<String>,
    include_asset_identity: bool,
}

impl FeatureEncoder {
    fn fit(dataset: &MetaDataset, include_asset_identity: bool) -> Self {
        let family_values = dataset
            .rows
            .iter()
            .map(|row| normalized_family(&row.features.family))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let asset_values = if include_asset_identity {
            dataset
                .rows
                .iter()
                .map(|row| normalized_asset(row.features.asset.as_deref()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let mut names = base_feature_names()
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>();
        names.extend(family_values.iter().map(|value| format!("family={value}")));
        if include_asset_identity {
            names.extend(asset_values.iter().map(|value| format!("asset={value}")));
        }
        Self {
            names,
            family_values,
            asset_values,
            include_asset_identity,
        }
    }

    fn encode(&self, features: &MetaFeatureRecord) -> Vec<f64> {
        let mut values = base_feature_values(features);
        let family = normalized_family(&features.family);
        values.extend(
            self.family_values
                .iter()
                .map(|value| f64::from(*value == family)),
        );
        if self.include_asset_identity {
            let asset = normalized_asset(features.asset.as_deref());
            values.extend(
                self.asset_values
                    .iter()
                    .map(|value| f64::from(*value == asset)),
            );
        }
        values
    }
}

fn base_feature_names() -> [&'static str; 23] {
    [
        "is_expectancy_r",
        "is_return_percent",
        "is_trade_count_log1p",
        "is_profit_factor",
        "is_sharpe",
        "is_drawdown_percent",
        "is_recovery_factor",
        "is_return_drawdown_ratio",
        "is_median_r",
        "fold_median_expectancy_r",
        "fold_spread_r",
        "fold_passing_fraction",
        "fold_has_negative",
        "parameter_median_ratio",
        "neighborhood_survival_fraction",
        "neighborhood_samples_log1p",
        "m1_return_retention",
        "m1_trade_retention",
        "m1_drawdown_expansion",
        "complexity_log1p",
        "entry_conditions",
        "exit_conditions",
        "has_robustness_fields",
    ]
}

fn base_feature_values(features: &MetaFeatureRecord) -> Vec<f64> {
    vec![
        finite_or_zero(features.is_expectancy_r),
        finite_or_zero(features.is_return_percent),
        (features.is_trade_count as f64 + 1.0).ln(),
        finite_or_zero(features.is_profit_factor.unwrap_or(0.0)).clamp(-100.0, 100.0),
        finite_or_zero(features.is_sharpe.unwrap_or(0.0)).clamp(-100.0, 100.0),
        finite_or_zero(features.is_drawdown_percent),
        finite_or_zero(features.is_recovery_factor.unwrap_or(0.0)).clamp(-100.0, 100.0),
        finite_or_zero(features.is_return_drawdown_ratio).clamp(-100.0, 100.0),
        finite_or_zero(features.is_median_r),
        finite_or_zero(features.fold_median_expectancy_r),
        finite_or_zero(features.fold_spread_r),
        finite_or_zero(features.fold_passing_fraction.unwrap_or(0.0)),
        f64::from(features.fold_has_negative),
        finite_or_zero(features.parameter_median_ratio.unwrap_or(0.0)),
        finite_or_zero(features.neighborhood_survival_fraction.unwrap_or(0.0)),
        (features.neighborhood_samples as f64 + 1.0).ln(),
        finite_or_zero(features.m1_return_retention.unwrap_or(0.0)),
        finite_or_zero(features.m1_trade_retention.unwrap_or(0.0)),
        finite_or_zero(features.m1_drawdown_expansion.unwrap_or(0.0)),
        (features.complexity as f64 + 1.0).ln(),
        features.entry_conditions as f64,
        features.exit_conditions as f64,
        f64::from(
            features.fold_passing_fraction.is_some()
                || features.parameter_median_ratio.is_some()
                || features.neighborhood_survival_fraction.is_some()
                || features.m1_return_retention.is_some(),
        ),
    ]
}

fn standardization(rows: &[Vec<f64>]) -> (Vec<f64>, Vec<f64>) {
    let width = rows.first().map_or(0, Vec::len);
    let mut means = vec![0.0; width];
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            means[index] += value;
        }
    }
    if !rows.is_empty() {
        for mean in &mut means {
            *mean /= rows.len() as f64;
        }
    }
    let mut scales = vec![0.0; width];
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            scales[index] += (value - means[index]).powi(2);
        }
    }
    for scale in &mut scales {
        *scale = (*scale / rows.len().max(1) as f64).sqrt();
        if *scale < 1.0e-9 {
            *scale = 1.0;
        }
    }
    (means, scales)
}

fn standardize(row: &[f64], means: &[f64], scales: &[f64]) -> Vec<f64> {
    row.iter()
        .enumerate()
        .map(|(index, value)| (value - means[index]) / scales[index])
        .collect()
}

fn sigmoid(value: f64) -> f64 {
    if value >= 0.0 {
        let exponent = (-value).exp();
        1.0 / (1.0 + exponent)
    } else {
        let exponent = value.exp();
        exponent / (1.0 + exponent)
    }
}

fn dot(left: &[f64], right: &[f64]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn rmse(predicted: &[f64], actual: &[f64]) -> Option<f64> {
    if predicted.is_empty() || predicted.len() != actual.len() {
        return None;
    }
    Some(
        (predicted
            .iter()
            .zip(actual)
            .map(|(predicted, actual)| (predicted - actual).powi(2))
            .sum::<f64>()
            / predicted.len() as f64)
            .sqrt(),
    )
}

fn rank_correlation(left: &[f64], right: &[f64]) -> Option<f64> {
    if left.is_empty() || left.len() != right.len() {
        return None;
    }
    let left_ranks = ranks(left);
    let right_ranks = ranks(right);
    pearson_correlation(&left_ranks, &right_ranks)
}

fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order = (0..values.len()).collect::<Vec<_>>();
    order.sort_by(|left, right| {
        values[*left]
            .total_cmp(&values[*right])
            .then_with(|| left.cmp(right))
    });
    let mut result = vec![0.0; values.len()];
    let mut start = 0;
    while start < order.len() {
        let mut end = start + 1;
        while end < order.len() && (values[order[start]] - values[order[end]]).abs() < 1.0e-12 {
            end += 1;
        }
        let rank = (start + end - 1) as f64 / 2.0;
        for index in start..end {
            result[order[index]] = rank;
        }
        start = end;
    }
    result
}

fn pearson_correlation(left: &[f64], right: &[f64]) -> Option<f64> {
    if left.is_empty() || left.len() != right.len() {
        return None;
    }
    let left_mean = left.iter().sum::<f64>() / left.len() as f64;
    let right_mean = right.iter().sum::<f64>() / right.len() as f64;
    let mut numerator = 0.0;
    let mut left_sum = 0.0;
    let mut right_sum = 0.0;
    for (left, right) in left.iter().zip(right) {
        let left_delta = left - left_mean;
        let right_delta = right - right_mean;
        numerator += left_delta * right_delta;
        left_sum += left_delta.powi(2);
        right_sum += right_delta.powi(2);
    }
    let denominator = (left_sum * right_sum).sqrt();
    (denominator > 1.0e-12).then_some(numerator / denominator)
}

fn auc(predictions: &[MetaPrediction]) -> Option<f64> {
    let positives = predictions.iter().filter(|row| row.survived).count();
    let negatives = predictions.len().saturating_sub(positives);
    if positives == 0 || negatives == 0 {
        return None;
    }
    let mut concordant = 0.0;
    for positive in predictions.iter().filter(|row| row.survived) {
        for negative in predictions.iter().filter(|row| !row.survived) {
            if positive.probability > negative.probability {
                concordant += 1.0;
            } else if (positive.probability - negative.probability).abs() < 1.0e-12 {
                concordant += 0.5;
            }
        }
    }
    Some(concordant / (positives * negatives) as f64)
}

fn brier_score(predictions: &[MetaPrediction]) -> Option<f64> {
    (!predictions.is_empty()).then(|| {
        predictions
            .iter()
            .map(|row| {
                let target = f64::from(row.survived);
                (row.probability - target).powi(2)
            })
            .sum::<f64>()
            / predictions.len() as f64
    })
}

fn calibration_bins(predictions: &[MetaPrediction], bin_count: usize) -> Vec<MetaCalibrationBin> {
    (0..bin_count)
        .map(|index| {
            let lower = index as f64 / bin_count as f64;
            let upper = if index + 1 == bin_count {
                1.0
            } else {
                (index + 1) as f64 / bin_count as f64
            };
            let values = predictions
                .iter()
                .filter(|row| {
                    row.probability >= lower && (row.probability < upper || index + 1 == bin_count)
                })
                .collect::<Vec<_>>();
            MetaCalibrationBin {
                lower_probability: lower,
                upper_probability: upper,
                rows: values.len(),
                mean_probability: mean(values.iter().map(|row| row.probability)),
                observed_survival_rate: if values.is_empty() {
                    None
                } else {
                    Some(
                        values.iter().filter(|row| row.survived).count() as f64
                            / values.len() as f64,
                    )
                },
            }
        })
        .collect()
}

fn expected_calibration_error(predictions: &[MetaPrediction], bin_count: usize) -> Option<f64> {
    if predictions.is_empty() {
        return None;
    }
    Some(
        calibration_bins(predictions, bin_count)
            .into_iter()
            .filter_map(|bin| {
                bin.mean_probability.zip(bin.observed_survival_rate).map(
                    |(mean_probability, observed)| {
                        bin.rows as f64 / predictions.len() as f64
                            * (mean_probability - observed).abs()
                    },
                )
            })
            .sum(),
    )
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values = values.filter(|value| value.is_finite()).collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn finite_option(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn normalized_family(value: &str) -> String {
    if value.trim().is_empty() {
        "unknown".into()
    } else {
        value.trim().to_ascii_lowercase()
    }
}

fn normalized_asset(value: Option<&str>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_uppercase())
        .unwrap_or_else(|| "unknown".into())
}

fn family_label(thesis_hint: &str) -> String {
    let normalized = thesis_hint.trim().to_ascii_lowercase();
    FamilyStyle::ALL
        .iter()
        .find(|family| {
            let label = format!("{:?}", family).to_ascii_lowercase();
            normalized.contains(&label)
        })
        .map(|family| format!("{:?}", family))
        .unwrap_or_else(|| {
            if normalized.is_empty() {
                "unknown".into()
            } else {
                normalized
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate_seed;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_core::ContentHash;
    use quantforge_data::{Bar, BarDataset};

    fn replay_broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Meta fixture".into(),
            symbol: "TEST".into(),
            digits: 5,
            point: 0.00001,
            tick_size: 0.00001,
            tick_value: 1.0,
            contract_size: 100_000.0,
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
            swap_multipliers: vec![],
            sessions: vec![],
            timezone: "Etc/UTC".into(),
            account_currency: "USD".into(),
            base_currency: "USD".into(),
            profit_currency: "USD".into(),
            margin_currency: "USD".into(),
            synthetic_spreads: vec![],
        }
    }

    fn replay_dataset() -> BarDataset {
        let bars = (0..480)
            .map(|index| {
                let timestamp_ms = index * 3_600_000;
                let wave = ((index as f64) / 9.0).sin() * 0.004;
                let open = 1.10 + wave + index as f64 * 0.00001;
                let close = open + (((index % 7) as f64) - 3.0) * 0.0001;
                Bar {
                    timestamp_ms,
                    open,
                    high: open.max(close) + 0.0004,
                    low: open.min(close) - 0.0004,
                    close,
                    tick_volume: 100,
                    real_volume: 0,
                    spread_points: Some(2),
                }
            })
            .collect::<Vec<_>>();
        BarDataset {
            data_hash: ContentHash::sha256(serde_json::to_vec(&bars).unwrap()),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars,
        }
    }

    fn feature(id: &str, cutoff: i64, quality: f64, asset: Option<&str>) -> MetaFeatureRecord {
        MetaFeatureRecord {
            strategy_id: id.into(),
            asset: asset.map(str::to_string),
            feature_cutoff_timestamp_ms: cutoff,
            family: if quality > 0.0 {
                "trend_pullback".into()
            } else {
                "mean_reversion_band".into()
            },
            complexity: if quality > 0.0 { 4 } else { 18 },
            entry_conditions: 2,
            exit_conditions: 1,
            is_expectancy_r: quality,
            is_return_percent: quality * 10.0,
            is_trade_count: if quality > 0.0 { 40 } else { 8 },
            is_profit_factor: Some(if quality > 0.0 { 1.5 } else { 0.9 }),
            is_sharpe: Some(quality),
            is_drawdown_percent: if quality > 0.0 { 10.0 } else { 40.0 },
            is_recovery_factor: Some(quality),
            is_return_drawdown_ratio: quality,
            is_median_r: quality,
            fold_median_expectancy_r: quality,
            fold_spread_r: 0.1,
            fold_passing_fraction: Some(if quality > 0.0 { 0.8 } else { 0.2 }),
            fold_has_negative: quality <= 0.0,
            parameter_median_ratio: Some(if quality > 0.0 { 1.0 } else { 0.3 }),
            neighborhood_survival_fraction: Some(if quality > 0.0 { 0.8 } else { 0.1 }),
            neighborhood_samples: 20,
            m1_return_retention: Some(if quality > 0.0 { 0.9 } else { 0.2 }),
            m1_trade_retention: Some(if quality > 0.0 { 0.9 } else { 0.2 }),
            m1_drawdown_expansion: Some(if quality > 0.0 { 1.0 } else { 2.0 }),
        }
    }

    fn input() -> MetaLearningInput {
        let mut candidates = Vec::new();
        for cutoff in [100_i64, 200_i64, 300_i64] {
            for index in 0..10 {
                let quality = if index < 5 { 1.0 } else { -1.0 };
                let id = format!("{cutoff}-{index}");
                let feature = feature(
                    &id,
                    cutoff,
                    quality,
                    Some(if index % 2 == 0 { "EURUSD" } else { "USDJPY" }),
                );
                candidates.push(MetaCandidate {
                    features: feature,
                    outcomes: vec![
                        MetaFutureOutcome {
                            window_id: format!("dev-{cutoff}"),
                            future_expectancy_r: if quality > 0.0 { 0.80 } else { -0.10 },
                            future_trade_count: if quality > 0.0 { 20 } else { 2 },
                            future_return_percent: None,
                            future_profit_factor: None,
                            future_drawdown_percent: None,
                        },
                        MetaFutureOutcome {
                            window_id: format!("val-{cutoff}"),
                            future_expectancy_r: if quality > 0.0 { 0.70 } else { -0.08 },
                            future_trade_count: if quality > 0.0 { 20 } else { 2 },
                            future_return_percent: None,
                            future_profit_factor: None,
                            future_drawdown_percent: None,
                        },
                        MetaFutureOutcome {
                            window_id: format!("sealed-{cutoff}"),
                            future_expectancy_r: if quality > 0.0 { 0.60 } else { -0.06 },
                            future_trade_count: if quality > 0.0 { 20 } else { 2 },
                            future_return_percent: None,
                            future_profit_factor: None,
                            future_drawdown_percent: None,
                        },
                    ],
                });
            }
        }
        MetaLearningInput {
            config: MetaLearningConfig {
                minimum_future_trades: 5,
                minimum_retention: 0.5,
                ..MetaLearningConfig::default()
            },
            windows: vec![
                MetaWindow {
                    id: "dev-100".into(),
                    role: MetaWindowRole::Development,
                    feature_cutoff_timestamp_ms: 100,
                    label_start_timestamp_ms: 101,
                    label_end_timestamp_ms: 200,
                    horizon_months: 6,
                },
                MetaWindow {
                    id: "val-200".into(),
                    role: MetaWindowRole::Validation,
                    feature_cutoff_timestamp_ms: 200,
                    label_start_timestamp_ms: 201,
                    label_end_timestamp_ms: 300,
                    horizon_months: 6,
                },
                MetaWindow {
                    id: "sealed-300".into(),
                    role: MetaWindowRole::Sealed,
                    feature_cutoff_timestamp_ms: 300,
                    label_start_timestamp_ms: 301,
                    label_end_timestamp_ms: 400,
                    horizon_months: 12,
                },
            ],
            candidates,
        }
    }

    #[test]
    fn training_dataset_excludes_validation_and_sealed_windows() {
        let input = input();
        let training = build_meta_dataset(&input, MetaDatasetScope::Training, Some(200)).unwrap();
        assert_eq!(training.rows.len(), 10);
        assert!(
            training
                .rows
                .iter()
                .all(|row| row.role == MetaWindowRole::Development)
        );
    }

    #[test]
    fn temporal_separation_rejects_a_label_that_starts_before_features_end() {
        let mut input = input();
        input.windows[0].label_start_timestamp_ms = 100;
        let error = build_meta_dataset(&input, MetaDatasetScope::Training, Some(200))
            .expect_err("overlapping label must be rejected");
        assert!(error.to_string().contains("strictly chronological"));
    }

    #[test]
    fn logistic_model_is_interpretable_and_excludes_asset_by_default() {
        let input = input();
        let training = build_meta_dataset(&input, MetaDatasetScope::Training, Some(200)).unwrap();
        let model = MetaLogisticModel::fit(&training, &input.config).unwrap();
        assert!(
            model
                .feature_names
                .iter()
                .all(|name| !name.starts_with("asset="))
        );
        assert_eq!(model.training_rows, 10);
        let validation = build_meta_dataset(&input, MetaDatasetScope::Validation, None).unwrap();
        let report = model.evaluate(&validation, &input.config).unwrap();
        assert!(report.auc.unwrap() > 0.80);
        assert!(report.precision_at_k.unwrap() > 0.80);
        assert!(report.selected_future_expectancy_lift_r.unwrap() > 0.0);
    }

    #[test]
    fn expectancy_model_directly_ranks_future_expectancy() {
        let input = input();
        let training = build_meta_dataset(&input, MetaDatasetScope::Training, Some(200)).unwrap();
        let model = MetaExpectancyModel::fit(&training, &input.config).unwrap();
        assert!(
            model
                .feature_names
                .iter()
                .all(|name| !name.starts_with("asset="))
        );
        assert_eq!(model.training_rows, 10);
        let validation = build_meta_dataset(&input, MetaDatasetScope::Validation, None).unwrap();
        let report = model.evaluate(&validation, &input.config).unwrap();
        assert!(report.rank_correlation.unwrap() > 0.80);
        assert!(report.selected_future_expectancy_lift_r.unwrap() > 0.0);
    }

    #[test]
    fn asset_identity_is_opt_in() {
        let mut input = input();
        input.config.include_asset_identity = true;
        let training = build_meta_dataset(&input, MetaDatasetScope::Training, Some(200)).unwrap();
        let model = MetaLogisticModel::fit(&training, &input.config).unwrap();
        assert!(
            model
                .feature_names
                .iter()
                .any(|name| name == "asset=EURUSD")
        );
    }

    #[test]
    fn walk_forward_sealed_episode_uses_only_known_prior_labels() {
        let input = input();
        let report = run_meta_walk_forward(&input).unwrap();
        assert_eq!(report.episodes.len(), 2);
        let sealed = report
            .episodes
            .iter()
            .find(|episode| episode.evaluation_role == MetaWindowRole::Sealed)
            .unwrap();
        assert_eq!(sealed.training_rows, 20);
        assert!(
            !sealed
                .training_window_ids
                .iter()
                .any(|id| id.starts_with("sealed"))
        );
        assert!(report.final_sealed_evaluation.is_some());
    }

    #[test]
    fn expectancy_walk_forward_sealed_episode_uses_only_known_prior_labels() {
        let input = input();
        let report = run_meta_expectancy_walk_forward(&input).unwrap();
        assert_eq!(report.episodes.len(), 2);
        let sealed = report
            .episodes
            .iter()
            .find(|episode| episode.evaluation_role == MetaWindowRole::Sealed)
            .unwrap();
        assert_eq!(sealed.training_rows, 20);
        assert!(
            !sealed
                .training_window_ids
                .iter()
                .any(|id| id.starts_with("sealed"))
        );
        assert!(report.final_sealed_evaluation.is_some());
    }

    #[test]
    fn replay_builder_replays_and_filters_trades_to_each_label_window() {
        let dataset = replay_dataset();
        let origin = MetaReplayOrigin {
            windows: vec![MetaReplayWindow {
                window: MetaWindow {
                    id: "validation-1".into(),
                    role: MetaWindowRole::Validation,
                    feature_cutoff_timestamp_ms: 3_600_000,
                    label_start_timestamp_ms: 24 * 3_600_000,
                    label_end_timestamp_ms: 480 * 3_600_000,
                    horizon_months: 6,
                },
                dataset: &dataset,
            }],
            candidates: vec![MetaReplayCandidate {
                features: feature("fixture-strategy", 3_600_000, 1.0, Some("TEST")),
                strategy: generate_seed(7, 0),
                broker: replay_broker(),
                scout: ScoutConfig::default(),
            }],
        };

        let input = build_meta_learning_input_from_replay(&[origin], MetaLearningConfig::default())
            .expect("fixture strategy should replay");
        assert_eq!(input.windows.len(), 1);
        assert_eq!(input.candidates.len(), 1);
        assert_eq!(input.candidates[0].outcomes.len(), 1);
        assert_eq!(input.candidates[0].outcomes[0].window_id, "validation-1");
        assert!(input.candidates[0].outcomes[0].future_trade_count <= 480);
    }

    #[test]
    fn replay_builder_rejects_data_at_or_after_label_end() {
        let mut dataset = replay_dataset();
        dataset.bars.push(Bar {
            timestamp_ms: 480 * 3_600_000,
            open: 1.1,
            high: 1.101,
            low: 1.099,
            close: 1.1,
            tick_volume: 1,
            real_volume: 0,
            spread_points: Some(2),
        });
        let origin = MetaReplayOrigin {
            windows: vec![MetaReplayWindow {
                window: MetaWindow {
                    id: "sealed-1".into(),
                    role: MetaWindowRole::Sealed,
                    feature_cutoff_timestamp_ms: 3_600_000,
                    label_start_timestamp_ms: 24 * 3_600_000,
                    label_end_timestamp_ms: 480 * 3_600_000,
                    horizon_months: 12,
                },
                dataset: &dataset,
            }],
            candidates: vec![MetaReplayCandidate {
                features: feature("fixture-strategy", 3_600_000, 1.0, Some("TEST")),
                strategy: generate_seed(7, 0),
                broker: replay_broker(),
                scout: ScoutConfig::default(),
            }],
        };
        let error = build_meta_learning_input_from_replay(&[origin], MetaLearningConfig::default())
            .expect_err("future bars must be rejected before replay");
        assert!(error.to_string().contains("at or after its label end"));
    }
}
