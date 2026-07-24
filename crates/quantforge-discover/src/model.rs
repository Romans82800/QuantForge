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
        // Scout defaults: loose enough for random search to fill the pot.
        Self {
            minimum_trades: 10,
            maximum_drawdown_percent: 40.0,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            minimum_return_drawdown: 0.0,
        }
    }
}

impl GateConfig {
    /// Stricter thresholds applied only when depositing into the databank.
    pub fn deposit_defaults() -> Self {
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
    /// Early H1/IS screen used during random search (cheap reject).
    pub gates: GateConfig,
    /// Final metrics required to enter or replace an elite in the pot.
    #[serde(default = "GateConfig::deposit_defaults")]
    pub deposit_gates: GateConfig,
    pub precision: PrecisionGateConfig,
    /// OOS1 expectancy must be at least this fraction of IS expectancy before a
    /// candidate may enter the databank (promotion-grade IS/OOS1/OOS2 workflow).
    #[serde(default = "default_oos1_expectancy_retention")]
    pub oos1_expectancy_retention: f64,
    /// When false (SQX Selected-TF style), Discover deposits from H1/IS only and
    /// defers M1 path fidelity to a later fidelity demo. Research-grade until verified.
    #[serde(default = "default_require_m1_precision")]
    pub require_m1_precision: bool,
    /// Prefer market entries, fixed/ATR SL-TP, no trailing/BE/partials, and a
    /// hard time stop of at most 16 bars — higher H1↔M1 agreement.
    #[serde(default = "default_simple_exits")]
    pub simple_exits: bool,
    /// Mandatory portfolio protection applied to every generated strategy.
    /// When enabled, exposure is flattened and entries are blocked from 22:00
    /// until the next broker day.
    pub flatten_at_22: bool,
    /// Cap each strategy to one filled entry per broker-local calendar day.
    /// Improves H1↔M1 agreement and keeps trade counts in a swing-friendly band.
    #[serde(default = "default_max_one_entry_per_day")]
    pub max_one_entry_per_day: bool,
    /// Keep random-filling the initial accepted pot until it holds this many
    /// strategies, then unlock crossover/mutation from that pot. Databank size
    /// is independent and only grows with M1-robust survivors.
    #[serde(
        default = "default_mutate_after_elites",
        alias = "mutate_after_generation"
    )]
    pub mutate_after_elites: usize,
    /// After breeding starts, this fraction of each batch remains fresh random seeds.
    #[serde(default = "default_random_fill_fraction")]
    pub random_fill_fraction: f64,
    /// Rayon worker threads for candidate evaluation. `0` = all logical CPUs.
    #[serde(default = "default_worker_threads")]
    pub worker_threads: usize,
    /// After IS scout + OOS1 pass, require M1 walk-forward / Monte Carlo /
    /// ±param neighborhood before a candidate may enter the pot.
    #[serde(default = "default_require_m1_robustness")]
    pub require_m1_robustness: bool,
    #[serde(default = "default_robustness_folds")]
    pub robustness_folds: usize,
    #[serde(default = "default_robustness_monte_carlo_trials")]
    pub robustness_monte_carlo_trials: usize,
    #[serde(default = "default_robustness_neighborhood_samples")]
    pub robustness_neighborhood_samples: usize,
    pub scout: ScoutConfig,
}

fn default_oos1_expectancy_retention() -> f64 {
    0.7
}

fn default_require_m1_precision() -> bool {
    // SQX builds on Selected TF first; M1 is a later retest.
    false
}

fn default_simple_exits() -> bool {
    true
}

fn default_max_one_entry_per_day() -> bool {
    true
}

fn default_mutate_after_elites() -> usize {
    300
}

fn default_random_fill_fraction() -> f64 {
    0.4
}

fn default_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|cores| cores.get().saturating_sub(1).max(1))
        .unwrap_or(1)
}

fn default_require_m1_robustness() -> bool {
    true
}

fn default_robustness_folds() -> usize {
    3
}

fn default_robustness_monte_carlo_trials() -> usize {
    250
}

fn default_robustness_neighborhood_samples() -> usize {
    8
}

impl Default for DiscoverConfig {
    fn default() -> Self {
        Self {
            initial_candidates: 500,
            batch_size: 200,
            correlation_threshold: 0.85,
            novelty_weight: 10.0,
            tournament_size: 4,
            structural_mutation_probability: 0.18,
            seed: 42,
            gates: GateConfig::default(),
            deposit_gates: GateConfig::deposit_defaults(),
            precision: PrecisionGateConfig::default(),
            oos1_expectancy_retention: default_oos1_expectancy_retention(),
            require_m1_precision: default_require_m1_precision(),
            simple_exits: default_simple_exits(),
            flatten_at_22: false,
            max_one_entry_per_day: default_max_one_entry_per_day(),
            mutate_after_elites: default_mutate_after_elites(),
            random_fill_fraction: default_random_fill_fraction(),
            worker_threads: default_worker_threads(),
            require_m1_robustness: default_require_m1_robustness(),
            robustness_folds: default_robustness_folds(),
            robustness_monte_carlo_trials: default_robustness_monte_carlo_trials(),
            robustness_neighborhood_samples: default_robustness_neighborhood_samples(),
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
        if !self.oos1_expectancy_retention.is_finite()
            || !(0.0..=2.0).contains(&self.oos1_expectancy_retention)
        {
            return Err(DiscoverError::InvalidConfig(
                "oos1_expectancy_retention must be finite and between 0 and 2".into(),
            ));
        }
        if !self.gates.maximum_drawdown_percent.is_finite()
            || self.gates.maximum_drawdown_percent < 0.0
            || !self.gates.minimum_return_percent.is_finite()
            || !self.gates.minimum_profit_factor.is_finite()
            || self.gates.minimum_profit_factor < 0.0
            || !self.gates.minimum_return_drawdown.is_finite()
            || self.gates.minimum_return_drawdown < 0.0
            || !self.deposit_gates.maximum_drawdown_percent.is_finite()
            || self.deposit_gates.maximum_drawdown_percent < 0.0
            || !self.deposit_gates.minimum_return_percent.is_finite()
            || !self.deposit_gates.minimum_profit_factor.is_finite()
            || self.deposit_gates.minimum_profit_factor < 0.0
            || !self.deposit_gates.minimum_return_drawdown.is_finite()
            || self.deposit_gates.minimum_return_drawdown < 0.0
        {
            return Err(DiscoverError::InvalidConfig(
                "gate thresholds must be finite and non-negative where applicable".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.random_fill_fraction)
            || !self.random_fill_fraction.is_finite()
        {
            return Err(DiscoverError::InvalidConfig(
                "random_fill_fraction must be finite and between 0 and 1".into(),
            ));
        }
        if self.require_m1_robustness {
            if self.robustness_folds < 2 {
                return Err(DiscoverError::InvalidConfig(
                    "robustness_folds must be at least 2".into(),
                ));
            }
            if self.robustness_monte_carlo_trials == 0
                || self.robustness_neighborhood_samples == 0
            {
                return Err(DiscoverError::InvalidConfig(
                    "robustness Monte Carlo trials and neighborhood samples must be positive"
                        .into(),
                ));
            }
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
    /// IS (development) expectancy used for ranking and the OOS1 pick gate.
    #[serde(default)]
    pub is_expectancy: f64,
    /// OOS1 (first holdout) expectancy when the promotion split was active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oos1_expectancy: Option<f64>,
    /// `oos1_expectancy / is_expectancy` when IS expectancy is positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oos1_expectancy_ratio: Option<f64>,
    /// Downsampled equity deltas, normalized only when correlation is computed.
    pub equity_signature: Vec<f64>,
    pub discovered_generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositDecision {
    AcceptedToPot,
    ReplacedInPot,
    AcceptedToDatabank,
    ReplacedInDatabank,
    RejectedGate,
    RejectedDepositGate,
    RejectedClone,
    RejectedCorrelated,
    RejectedNicheNotImproved,
    RejectedPrecision,
    RejectedOos1,
    RejectedM1Fidelity,
    RejectedWalkForward,
    RejectedMonteCarlo,
    RejectedParamNeighborhood,
    RejectedEvaluation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverTelemetry {
    #[serde(default)]
    pub pot_accepted: u64,
    #[serde(default)]
    pub pot_replaced: u64,
    #[serde(default)]
    pub databank_accepted: u64,
    #[serde(default)]
    pub databank_replaced: u64,
    /// Legacy alias counters kept for older UI/readers.
    #[serde(default)]
    pub accepted_empty: u64,
    #[serde(default)]
    pub replaced_elite: u64,
    pub rejected_gate: u64,
    #[serde(default)]
    pub rejected_deposit_gate: u64,
    pub rejected_clone: u64,
    pub rejected_correlated: u64,
    pub rejected_niche_not_improved: u64,
    pub rejected_precision: u64,
    #[serde(default)]
    pub rejected_oos1: u64,
    #[serde(default)]
    pub rejected_m1_fidelity: u64,
    #[serde(default)]
    pub rejected_walk_forward: u64,
    #[serde(default)]
    pub rejected_monte_carlo: u64,
    #[serde(default)]
    pub rejected_param_neighborhood: u64,
    pub rejected_evaluation: u64,
    pub evaluation_errors: BTreeMap<String, u64>,
}

impl DiscoverTelemetry {
    pub(crate) fn record(&mut self, decision: DepositDecision) {
        match decision {
            DepositDecision::AcceptedToPot => {
                self.pot_accepted += 1;
                self.accepted_empty += 1;
            }
            DepositDecision::ReplacedInPot => {
                self.pot_replaced += 1;
                self.replaced_elite += 1;
            }
            DepositDecision::AcceptedToDatabank => {
                self.databank_accepted += 1;
            }
            DepositDecision::ReplacedInDatabank => {
                self.databank_replaced += 1;
            }
            DepositDecision::RejectedGate => self.rejected_gate += 1,
            DepositDecision::RejectedDepositGate => self.rejected_deposit_gate += 1,
            DepositDecision::RejectedClone => self.rejected_clone += 1,
            DepositDecision::RejectedCorrelated => self.rejected_correlated += 1,
            DepositDecision::RejectedNicheNotImproved => {
                self.rejected_niche_not_improved += 1;
            }
            DepositDecision::RejectedPrecision => self.rejected_precision += 1,
            DepositDecision::RejectedOos1 => self.rejected_oos1 += 1,
            DepositDecision::RejectedM1Fidelity => self.rejected_m1_fidelity += 1,
            DepositDecision::RejectedWalkForward => self.rejected_walk_forward += 1,
            DepositDecision::RejectedMonteCarlo => self.rejected_monte_carlo += 1,
            DepositDecision::RejectedParamNeighborhood => self.rejected_param_neighborhood += 1,
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
    /// Initial accepted pot used for breeding (IS+OOS1+deposit gates only).
    #[serde(default)]
    pub accepted_pool: Vec<Elite>,
    #[serde(default)]
    pub accepted_coverage_map: BTreeMap<String, ContentHash>,
    /// Promotion databank: elites that also passed M1 WFO/MC/param robustness.
    pub elites: Vec<Elite>,
    /// Stable niche string to elite fingerprint, convenient for UI coverage maps.
    pub coverage_map: BTreeMap<String, ContentHash>,
    pub telemetry: DiscoverTelemetry,
}

impl Databank {
    pub fn coverage(&self) -> usize {
        self.elites.len()
    }

    pub fn pot_size(&self) -> usize {
        self.accepted_pool.len()
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
        if self.evaluation_count == 0
            || (self.elites.is_empty() && self.accepted_pool.is_empty())
        {
            return Err(DiscoverError::IncompatibleDatabank(
                "a databank requires evaluations and either an accepted pot or databank elites"
                    .into(),
            ));
        }
        validate_archive_entries(
            &self.elites,
            &self.coverage_map,
            &self.config,
            self.completed_generations,
        )?;
        validate_archive_entries(
            &self.accepted_pool,
            &self.accepted_coverage_map,
            &self.config,
            self.completed_generations,
        )?;
        Ok(())
    }
}

fn validate_archive_entries(
    entries: &[Elite],
    coverage_map: &BTreeMap<String, ContentHash>,
    config: &DiscoverConfig,
    completed_generations: u64,
) -> Result<(), DiscoverError> {
    let fingerprints: BTreeSet<_> = entries
        .iter()
        .map(|elite| elite.structural_fingerprint.clone())
        .collect();
    let covered: BTreeSet<_> = coverage_map.values().cloned().collect();
    if fingerprints.len() != entries.len()
        || coverage_map.len() != entries.len()
        || covered != fingerprints
    {
        return Err(DiscoverError::IncompatibleDatabank(
            "coverage, niche or fingerprint identities are inconsistent".into(),
        ));
    }
    for elite in entries {
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
        if elite.strategy.manage.flatten_end_of_day != config.flatten_at_22
            || elite.strategy.manage.max_one_entry_per_day != config.max_one_entry_per_day
            || !fixed_risk
            || fingerprint != elite.structural_fingerprint
            || niche_key(&elite.descriptor) != elite.niche
            || coverage_map.get(&niche_label(&elite.niche)) != Some(&elite.structural_fingerprint)
            || elite.metrics.trade_count < config.deposit_gates.minimum_trades
            || elite.metrics.return_percent <= config.deposit_gates.minimum_return_percent
            || effective_profit_factor < config.deposit_gates.minimum_profit_factor
            || return_drawdown_ratio(&elite.metrics) < config.deposit_gates.minimum_return_drawdown
            || elite.metrics.max_drawdown_percent > config.deposit_gates.maximum_drawdown_percent
            || elite.discovered_generation > completed_generations
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
