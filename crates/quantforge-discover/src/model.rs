use crate::archive::{niche_key, niche_label};
use crate::{DATABANK_SCHEMA_VERSION, GRAMMAR_VERSION};
use quantforge_core::{ContentHash, FloatPolicy};
use quantforge_eval::{BacktestMetrics, EvalError, ScoutConfig};
use quantforge_ir::{IrError, StrategyIr};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GateConfig {
    pub minimum_trades: usize,
    pub maximum_drawdown_percent: f64,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub minimum_return_drawdown: f64,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            minimum_trades: 20,
            maximum_drawdown_percent: 30.0,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            minimum_return_drawdown: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrecisionGateConfig {
    /// A positive M1 return must retain at least this fraction of the H1
    /// screening return. Values above one are allowed and mean M1 improved.
    pub minimum_return_retention: f64,
}

impl Default for PrecisionGateConfig {
    fn default() -> Self {
        Self {
            minimum_return_retention: 0.95,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoverConfig {
    pub initial_candidates: usize,
    pub batch_size: usize,
    pub correlation_threshold: f64,
    pub novelty_weight: f64,
    pub tournament_size: usize,
    pub structural_mutation_probability: f64,
    pub seed: u64,
    pub gates: GateConfig,
    pub precision: PrecisionGateConfig,
    /// Mandatory portfolio protection applied to every generated strategy.
    /// When enabled, exposure is flattened and entries are blocked from 22:00
    /// until the next broker day.
    pub flatten_at_22: bool,
    pub scout: ScoutConfig,
}

impl Default for DiscoverConfig {
    fn default() -> Self {
        Self {
            initial_candidates: 500,
            batch_size: 200,
            correlation_threshold: 0.88,
            novelty_weight: 10.0,
            tournament_size: 4,
            structural_mutation_probability: 0.18,
            seed: 42,
            gates: GateConfig::default(),
            precision: PrecisionGateConfig::default(),
            flatten_at_22: false,
            scout: ScoutConfig::default(),
        }
    }
}

impl DiscoverConfig {
    pub(crate) fn validate(&self) -> Result<(), DiscoverError> {
        if self.initial_candidates == 0 {
            return Err(DiscoverError::InvalidConfig(
                "initial_candidates must be greater than zero".into(),
            ));
        }
        if self.batch_size == 0 {
            return Err(DiscoverError::InvalidConfig(
                "batch_size must be greater than zero".into(),
            ));
        }
        if self.tournament_size == 0 {
            return Err(DiscoverError::InvalidConfig(
                "tournament_size must be greater than zero".into(),
            ));
        }
        for (name, value, inclusive_max) in [
            ("correlation_threshold", self.correlation_threshold, 1.0),
            (
                "structural_mutation_probability",
                self.structural_mutation_probability,
                1.0,
            ),
        ] {
            if !value.is_finite() || !(0.0..=inclusive_max).contains(&value) {
                return Err(DiscoverError::InvalidConfig(format!(
                    "{name} must be finite and between 0 and {inclusive_max}"
                )));
            }
        }
        if !self.novelty_weight.is_finite() || self.novelty_weight < 0.0 {
            return Err(DiscoverError::InvalidConfig(
                "novelty_weight must be finite and non-negative".into(),
            ));
        }
        if !self.precision.minimum_return_retention.is_finite()
            || !(0.0..=1.0).contains(&self.precision.minimum_return_retention)
        {
            return Err(DiscoverError::InvalidConfig(
                "minimum_return_retention must be finite and between 0 and 1".into(),
            ));
        }
        if !self.gates.maximum_drawdown_percent.is_finite()
            || self.gates.maximum_drawdown_percent < 0.0
            || !self.gates.minimum_return_percent.is_finite()
            || !self.gates.minimum_profit_factor.is_finite()
            || self.gates.minimum_profit_factor < 0.0
            || !self.gates.minimum_return_drawdown.is_finite()
            || self.gates.minimum_return_drawdown < 0.0
        {
            return Err(DiscoverError::InvalidConfig(
                "gate thresholds must be finite and non-negative where applicable".into(),
            ));
        }
        self.scout.validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyStyle {
    Trend,
    Momentum,
    Breakout,
    MeanReversion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreeLevelBucket {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LongShortSkewBucket {
    ShortHeavy,
    Balanced,
    LongHeavy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehaviorDescriptor {
    pub family: FamilyStyle,
    pub trades_per_1000_bars: f64,
    pub average_bars_held: f64,
    pub drawdown_percent: f64,
    pub win_rate_percent: f64,
    /// -1 is entirely short, 0 balanced and +1 entirely long.
    pub long_short_skew: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NicheKey {
    pub family: FamilyStyle,
    pub trade_frequency: ThreeLevelBucket,
    pub hold_time: ThreeLevelBucket,
    pub drawdown: ThreeLevelBucket,
    pub win_rate: ThreeLevelBucket,
    pub long_short_skew: LongShortSkewBucket,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceComponents {
    pub return_component: f64,
    pub profit_factor_component: f64,
    pub trade_count_bonus: f64,
    pub drawdown_penalty: f64,
    pub complexity_penalty: f64,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Elite {
    pub strategy: StrategyIr,
    pub structural_fingerprint: ContentHash,
    pub descriptor: BehaviorDescriptor,
    pub niche: NicheKey,
    pub evidence: EvidenceComponents,
    pub novelty: f64,
    pub complexity: usize,
    pub metrics: BacktestMetrics,
    /// Downsampled equity deltas, normalized only when correlation is computed.
    pub equity_signature: Vec<f64>,
    pub discovered_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositDecision {
    AcceptedEmpty,
    ReplacedElite,
    RejectedGate,
    RejectedClone,
    RejectedCorrelated,
    RejectedNicheNotImproved,
    RejectedPrecision,
    RejectedEvaluation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverTelemetry {
    pub accepted_empty: u64,
    pub replaced_elite: u64,
    pub rejected_gate: u64,
    pub rejected_clone: u64,
    pub rejected_correlated: u64,
    pub rejected_niche_not_improved: u64,
    pub rejected_precision: u64,
    pub rejected_evaluation: u64,
    pub evaluation_errors: BTreeMap<String, u64>,
}

impl DiscoverTelemetry {
    pub(crate) fn record(&mut self, decision: DepositDecision) {
        match decision {
            DepositDecision::AcceptedEmpty => self.accepted_empty += 1,
            DepositDecision::ReplacedElite => self.replaced_elite += 1,
            DepositDecision::RejectedGate => self.rejected_gate += 1,
            DepositDecision::RejectedClone => self.rejected_clone += 1,
            DepositDecision::RejectedCorrelated => self.rejected_correlated += 1,
            DepositDecision::RejectedNicheNotImproved => {
                self.rejected_niche_not_improved += 1;
            }
            DepositDecision::RejectedPrecision => self.rejected_precision += 1,
            DepositDecision::RejectedEvaluation => self.rejected_evaluation += 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Databank {
    pub schema_version: u16,
    pub grammar_version: String,
    pub data_hash: ContentHash,
    /// M1 chronology used to decide which candidates were allowed into the
    /// archive. A databank without this binding is not promotion grade.
    pub execution_data_hash: ContentHash,
    pub broker_spec_hash: ContentHash,
    pub config: DiscoverConfig,
    pub completed_generations: u64,
    pub evaluation_count: u64,
    pub elites: Vec<Elite>,
    /// Stable niche string to elite fingerprint, convenient for UI coverage maps.
    pub coverage_map: BTreeMap<String, ContentHash>,
    pub telemetry: DiscoverTelemetry,
}

impl Databank {
    pub fn coverage(&self) -> usize {
        self.elites.len()
    }

    pub fn qd_score(&self) -> f64 {
        self.elites
            .iter()
            .map(|elite| elite.evidence.total.max(0.0))
            .sum()
    }

    /// Validates the persisted archive independently of any UI or CLI adapter.
    pub fn validate_integrity(&self) -> Result<(), DiscoverError> {
        self.config.validate()?;
        if self.schema_version != DATABANK_SCHEMA_VERSION {
            return Err(DiscoverError::IncompatibleDatabank(format!(
                "schema version {} is not supported",
                self.schema_version
            )));
        }
        if self.grammar_version != GRAMMAR_VERSION {
            return Err(DiscoverError::IncompatibleDatabank(format!(
                "grammar {} does not match {}",
                self.grammar_version, GRAMMAR_VERSION
            )));
        }
        if self.evaluation_count == 0 || self.elites.is_empty() {
            return Err(DiscoverError::IncompatibleDatabank(
                "a promotion-grade databank requires evaluations and elites".into(),
            ));
        }
        let fingerprints: BTreeSet<_> = self
            .elites
            .iter()
            .map(|elite| elite.structural_fingerprint.clone())
            .collect();
        let covered: BTreeSet<_> = self.coverage_map.values().cloned().collect();
        if fingerprints.len() != self.elites.len()
            || self.coverage_map.len() != self.elites.len()
            || covered != fingerprints
        {
            return Err(DiscoverError::IncompatibleDatabank(
                "coverage, niche or fingerprint identities are inconsistent".into(),
            ));
        }
        for elite in &self.elites {
            let fingerprint = elite
                .strategy
                .structural_fingerprint(FloatPolicy::default())?;
            let effective_profit_factor = elite.metrics.profit_factor.unwrap_or(
                if elite.metrics.net_profit > 0.0 && elite.metrics.winning_trades > 0 {
                    f64::MAX
                } else {
                    0.0
                },
            );
            let fixed_risk = matches!(
                elite.strategy.risk,
                quantforge_ir::RiskPolicy::FixedCurrency { amount }
                    if (amount - crate::FIXED_RISK_PER_TRADE).abs() <= 1.0e-9
            );
            if elite.strategy.manage.flatten_end_of_day != self.config.flatten_at_22
                || !fixed_risk
                || fingerprint != elite.structural_fingerprint
                || niche_key(&elite.descriptor) != elite.niche
                || self.coverage_map.get(&niche_label(&elite.niche))
                    != Some(&elite.structural_fingerprint)
                || elite.metrics.trade_count < self.config.gates.minimum_trades
                || elite.metrics.return_percent <= self.config.gates.minimum_return_percent
                || effective_profit_factor < self.config.gates.minimum_profit_factor
                || return_drawdown_ratio(&elite.metrics) < self.config.gates.minimum_return_drawdown
                || elite.metrics.max_drawdown_percent > self.config.gates.maximum_drawdown_percent
                || elite.discovered_generation > self.completed_generations
                || !elite.evidence.total.is_finite()
                || !elite.novelty.is_finite()
            {
                return Err(DiscoverError::IncompatibleDatabank(
                    "an elite is structurally invalid or no longer passes its stored gates".into(),
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn return_drawdown_ratio(metrics: &BacktestMetrics) -> f64 {
    if metrics.max_drawdown_percent > 1.0e-12 {
        metrics.return_percent / metrics.max_drawdown_percent
    } else if metrics.return_percent > 0.0 {
        f64::INFINITY
    } else {
        metrics.return_percent
    }
}

fn is_zero(value: &f64) -> bool {
    value.abs() <= f64::EPSILON
}

#[derive(Debug, Error)]
pub enum DiscoverError {
    #[error("invalid discovery configuration: {0}")]
    InvalidConfig(String),
    #[error("cannot continue databank: {0}")]
    IncompatibleDatabank(String),
    #[error("the initial population produced no eligible elites; loosen gates or use more data")]
    EmptyArchive,
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error(transparent)]
    Ir(#[from] IrError),
    #[error(transparent)]
    Broker(#[from] quantforge_broker::BrokerSpecError),
}
