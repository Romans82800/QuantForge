//! Deterministic, correlation-constrained strategy portfolio packing.

use quantforge_core::{ContentHash, HashError, stable_json_hash};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const PORTFOLIO_SCHEMA_VERSION: u16 = 1;
pub const PORTFOLIO_PROTOCOL_VERSION: &str = "portfolio-pack-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortfolioObjective {
    RiskAdjustedReturn,
    Cvar,
    MinimizeDrawdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PortfolioConfig {
    pub objective: PortfolioObjective,
    pub maximum_pairwise_correlation: f64,
    pub maximum_weight_per_strategy: f64,
    pub maximum_symbol_exposure: f64,
    pub maximum_cohort_exposure: f64,
    pub maximum_strategies: usize,
    pub minimum_return_percent: f64,
    pub cvar_tail_fraction: f64,
    pub stress_trials: usize,
    pub stress_block_length: usize,
    pub seed: u64,
}

impl Default for PortfolioConfig {
    fn default() -> Self {
        Self {
            objective: PortfolioObjective::RiskAdjustedReturn,
            maximum_pairwise_correlation: 0.70,
            maximum_weight_per_strategy: 0.25,
            maximum_symbol_exposure: 1.0,
            maximum_cohort_exposure: 0.50,
            maximum_strategies: 10,
            minimum_return_percent: 0.0,
            cvar_tail_fraction: 0.05,
            stress_trials: 1_000,
            stress_block_length: 5,
            seed: 42,
        }
    }
}

impl PortfolioConfig {
    pub fn validate(&self) -> Result<(), PortfolioError> {
        if !self.maximum_pairwise_correlation.is_finite()
            || !(0.0..=1.0).contains(&self.maximum_pairwise_correlation)
        {
            return Err(PortfolioError::InvalidConfig(
                "maximum_pairwise_correlation must be finite and between zero and one".into(),
            ));
        }
        for (name, value) in [
            (
                "maximum_weight_per_strategy",
                self.maximum_weight_per_strategy,
            ),
            ("maximum_symbol_exposure", self.maximum_symbol_exposure),
            ("maximum_cohort_exposure", self.maximum_cohort_exposure),
            ("cvar_tail_fraction", self.cvar_tail_fraction),
        ] {
            if !value.is_finite() || value <= 0.0 || value > 1.0 {
                return Err(PortfolioError::InvalidConfig(format!(
                    "{name} must be finite, greater than zero and at most one"
                )));
            }
        }
        if !self.minimum_return_percent.is_finite() {
            return Err(PortfolioError::InvalidConfig(
                "minimum_return_percent must be finite".into(),
            ));
        }
        if self.maximum_strategies == 0 || self.stress_trials == 0 || self.stress_block_length == 0
        {
            return Err(PortfolioError::InvalidConfig(
                "strategy cap, stress trials and block length must be positive".into(),
            ));
        }
        let minimum_count = minimum_strategy_count(self.maximum_weight_per_strategy);
        if minimum_count > self.maximum_strategies {
            return Err(PortfolioError::InvalidConfig(format!(
                "maximum_weight_per_strategy requires at least {minimum_count} strategies, above the configured strategy cap"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioCandidate {
    pub strategy_fingerprint: ContentHash,
    pub symbol: String,
    /// Diversification group. Callers pass a behaviour cohort (trade frequency
    /// and hold time) so the exposure cap limits correlated selections.
    pub cohort: String,
    pub initial_balance: f64,
    pub return_percent: f64,
    pub maximum_drawdown_percent: f64,
    /// Downsampled chronological equity deltas. The packer rescales these to
    /// the candidate's recorded total return before correlation and stress.
    pub equity_signature: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioAllocation {
    pub strategy_fingerprint: ContentHash,
    pub symbol: String,
    pub cohort: String,
    pub weight: f64,
    pub source_return_percent: f64,
    pub source_maximum_drawdown_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseCorrelation {
    pub left: ContentHash,
    pub right: ContentHash,
    pub correlation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioStressTrial {
    pub trial: usize,
    pub return_percent: f64,
    pub maximum_drawdown_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioStressReport {
    pub method: String,
    pub seed: u64,
    pub trials: usize,
    pub block_length: usize,
    pub tail_fraction: f64,
    pub p05_return_percent: f64,
    pub cvar_return_percent: f64,
    pub p95_maximum_drawdown_percent: f64,
    pub trial_results: Vec<PortfolioStressTrial>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioReport {
    pub schema_version: u16,
    pub protocol_version: String,
    pub portfolio_id: ContentHash,
    pub data_hash: ContentHash,
    pub broker_spec_hash: ContentHash,
    pub config: PortfolioConfig,
    pub source_candidate_count: usize,
    pub selected: Vec<PortfolioAllocation>,
    pub pairwise_correlations: Vec<PairwiseCorrelation>,
    pub maximum_observed_pairwise_correlation: f64,
    pub symbol_exposures: BTreeMap<String, f64>,
    pub cohort_exposures: BTreeMap<String, f64>,
    pub expected_return_percent: f64,
    pub path_maximum_drawdown_percent: f64,
    pub objective_score: f64,
    pub portfolio_return_path: Vec<f64>,
    pub stress: PortfolioStressReport,
}

#[derive(Serialize)]
struct PortfolioIdentity<'a> {
    protocol_version: &'a str,
    data_hash: &'a ContentHash,
    broker_spec_hash: &'a ContentHash,
    config: &'a PortfolioConfig,
    selected: &'a [PortfolioAllocation],
}

pub fn pack_portfolio(
    candidates: &[PortfolioCandidate],
    data_hash: ContentHash,
    broker_spec_hash: ContentHash,
    config: PortfolioConfig,
) -> Result<PortfolioReport, PortfolioError> {
    config.validate()?;
    let prepared = prepare_candidates(candidates, config.cvar_tail_fraction)?;
    let minimum_count = minimum_strategy_count(config.maximum_weight_per_strategy);
    let maximum_count = config.maximum_strategies.min(prepared.len());
    if maximum_count < minimum_count {
        return Err(PortfolioError::NoFeasiblePortfolio(format!(
            "only {} valid candidates are available, but the weight cap requires {minimum_count}",
            prepared.len()
        )));
    }

    let mut best: Option<PortfolioReport> = None;
    for target_count in (minimum_count..=maximum_count).rev() {
        let selected = greedy_selection(&prepared, target_count, &config);
        if selected.len() != target_count {
            continue;
        }
        let report = build_report(
            &selected,
            candidates.len(),
            data_hash.clone(),
            broker_spec_hash.clone(),
            &config,
        )?;
        if report.expected_return_percent < config.minimum_return_percent {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|current| report_is_better(&report, current))
        {
            best = Some(report);
        }
    }
    best.ok_or_else(|| {
        PortfolioError::NoFeasiblePortfolio(
            "no subset satisfies the correlation, exposure, weight and return caps".into(),
        )
    })
}

#[derive(Clone)]
struct PreparedCandidate<'a> {
    candidate: &'a PortfolioCandidate,
    returns: Vec<f64>,
    rank_score: f64,
    cvar_rank_score: f64,
}

fn prepare_candidates(
    candidates: &[PortfolioCandidate],
    tail_fraction: f64,
) -> Result<Vec<PreparedCandidate<'_>>, PortfolioError> {
    if candidates.is_empty() {
        return Err(PortfolioError::NoCandidates);
    }
    let signature_length = candidates[0].equity_signature.len();
    if signature_length < 2 {
        return Err(PortfolioError::InvalidCandidate(
            "equity signatures must contain at least two points".into(),
        ));
    }
    let mut fingerprints = BTreeSet::new();
    let mut prepared = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !fingerprints.insert(candidate.strategy_fingerprint.clone()) {
            return Err(PortfolioError::InvalidCandidate(format!(
                "duplicate strategy fingerprint {}",
                candidate.strategy_fingerprint
            )));
        }
        if candidate.symbol.trim().is_empty()
            || candidate.cohort.trim().is_empty()
            || !candidate.initial_balance.is_finite()
            || candidate.initial_balance <= 0.0
            || !candidate.return_percent.is_finite()
            || !candidate.maximum_drawdown_percent.is_finite()
            || candidate.maximum_drawdown_percent < 0.0
            || candidate.equity_signature.len() != signature_length
            || candidate
                .equity_signature
                .iter()
                .any(|value| !value.is_finite())
        {
            return Err(PortfolioError::InvalidCandidate(format!(
                "strategy {} has invalid metadata, metrics or signature",
                candidate.strategy_fingerprint
            )));
        }
        let returns = normalized_returns(candidate)?;
        let tail = historical_cvar(&returns, tail_fraction).abs().max(1.0e-12);
        let risk_adjusted = candidate.return_percent / candidate.maximum_drawdown_percent.max(0.01);
        let cvar_adjusted = candidate.return_percent / (tail * 100.0);
        // The objective-specific ordering is applied later. Store both risk
        // measures in one stable base score for deterministic tie breaking.
        let rank_score = risk_adjusted + cvar_adjusted * 1.0e-6;
        prepared.push(PreparedCandidate {
            candidate,
            returns,
            rank_score,
            cvar_rank_score: cvar_adjusted,
        });
    }
    Ok(prepared)
}

fn normalized_returns(candidate: &PortfolioCandidate) -> Result<Vec<f64>, PortfolioError> {
    let target_total = candidate.return_percent / 100.0;
    let signature_total: f64 = candidate.equity_signature.iter().sum();
    if signature_total.abs() <= f64::EPSILON {
        if target_total.abs() <= f64::EPSILON {
            return Ok(vec![0.0; candidate.equity_signature.len()]);
        }
        return Err(PortfolioError::InvalidCandidate(format!(
            "strategy {} has a zero-sum equity signature but non-zero return",
            candidate.strategy_fingerprint
        )));
    }
    if target_total != 0.0 && target_total.signum() != signature_total.signum() {
        return Err(PortfolioError::InvalidCandidate(format!(
            "strategy {} has an equity signature inconsistent with its recorded return",
            candidate.strategy_fingerprint
        )));
    }
    let scale = target_total / signature_total;
    Ok(candidate
        .equity_signature
        .iter()
        .map(|delta| delta * scale)
        .collect())
}

fn greedy_selection<'a>(
    candidates: &'a [PreparedCandidate<'a>],
    target_count: usize,
    config: &PortfolioConfig,
) -> Vec<&'a PreparedCandidate<'a>> {
    let mut ordered: Vec<_> = candidates.iter().collect();
    ordered.sort_by(|left, right| {
        candidate_rank(right, config.objective)
            .total_cmp(&candidate_rank(left, config.objective))
            .then_with(|| {
                left.candidate
                    .strategy_fingerprint
                    .cmp(&right.candidate.strategy_fingerprint)
            })
    });
    let maximum_per_symbol = exposure_count(config.maximum_symbol_exposure, target_count);
    let maximum_per_cohort = exposure_count(config.maximum_cohort_exposure, target_count);
    if maximum_per_symbol == 0 || maximum_per_cohort == 0 {
        return Vec::new();
    }
    let mut symbols = BTreeMap::<&str, usize>::new();
    let mut cohorts = BTreeMap::<&str, usize>::new();
    let mut selected = Vec::with_capacity(target_count);
    for candidate in ordered {
        if symbols
            .get(candidate.candidate.symbol.as_str())
            .copied()
            .unwrap_or(0)
            >= maximum_per_symbol
            || cohorts
                .get(candidate.candidate.cohort.as_str())
                .copied()
                .unwrap_or(0)
                >= maximum_per_cohort
            || selected.iter().any(|existing: &&PreparedCandidate<'_>| {
                correlation(&candidate.returns, &existing.returns)
                    > config.maximum_pairwise_correlation
            })
        {
            continue;
        }
        *symbols.entry(&candidate.candidate.symbol).or_default() += 1;
        *cohorts.entry(&candidate.candidate.cohort).or_default() += 1;
        selected.push(candidate);
        if selected.len() == target_count {
            break;
        }
    }
    selected
}

fn candidate_rank(candidate: &PreparedCandidate<'_>, objective: PortfolioObjective) -> f64 {
    match objective {
        PortfolioObjective::RiskAdjustedReturn => candidate.rank_score,
        PortfolioObjective::Cvar => candidate.cvar_rank_score,
        PortfolioObjective::MinimizeDrawdown => {
            -candidate.candidate.maximum_drawdown_percent
                + candidate.candidate.return_percent * 1.0e-6
        }
    }
}

fn build_report(
    selected: &[&PreparedCandidate<'_>],
    source_candidate_count: usize,
    data_hash: ContentHash,
    broker_spec_hash: ContentHash,
    config: &PortfolioConfig,
) -> Result<PortfolioReport, PortfolioError> {
    let weight = 1.0 / selected.len() as f64;
    let allocations: Vec<_> = selected
        .iter()
        .map(|value| PortfolioAllocation {
            strategy_fingerprint: value.candidate.strategy_fingerprint.clone(),
            symbol: value.candidate.symbol.clone(),
            cohort: value.candidate.cohort.clone(),
            weight,
            source_return_percent: value.candidate.return_percent,
            source_maximum_drawdown_percent: value.candidate.maximum_drawdown_percent,
        })
        .collect();
    let mut portfolio_returns = vec![0.0; selected[0].returns.len()];
    for candidate in selected {
        for (portfolio, value) in portfolio_returns.iter_mut().zip(&candidate.returns) {
            *portfolio += value * weight;
        }
    }
    let expected_return_percent: f64 = allocations
        .iter()
        .map(|allocation| allocation.source_return_percent * allocation.weight)
        .sum();
    let path_maximum_drawdown_percent = maximum_drawdown_percent(&portfolio_returns);
    let stress = stress_portfolio(&portfolio_returns, config);
    let objective_score = match config.objective {
        PortfolioObjective::RiskAdjustedReturn => {
            expected_return_percent / path_maximum_drawdown_percent.max(0.01)
        }
        PortfolioObjective::Cvar => stress.cvar_return_percent,
        PortfolioObjective::MinimizeDrawdown => -path_maximum_drawdown_percent,
    };
    let pairwise_correlations = pairwise(selected);
    let maximum_observed_pairwise_correlation = pairwise_correlations
        .iter()
        .map(|value| value.correlation)
        .fold(0.0_f64, f64::max);
    let symbol_exposures = exposures(&allocations, |value| value.symbol.as_str());
    let cohort_exposures = exposures(&allocations, |value| value.cohort.as_str());
    if weight > config.maximum_weight_per_strategy + 1.0e-12
        || maximum_observed_pairwise_correlation > config.maximum_pairwise_correlation + 1.0e-12
        || symbol_exposures
            .values()
            .any(|value| *value > config.maximum_symbol_exposure + 1.0e-12)
        || cohort_exposures
            .values()
            .any(|value| *value > config.maximum_cohort_exposure + 1.0e-12)
    {
        return Err(PortfolioError::InternalConstraintViolation);
    }
    let portfolio_return_path = cumulative_path(&portfolio_returns);
    let portfolio_id = stable_json_hash(&PortfolioIdentity {
        protocol_version: PORTFOLIO_PROTOCOL_VERSION,
        data_hash: &data_hash,
        broker_spec_hash: &broker_spec_hash,
        config,
        selected: &allocations,
    })?;
    Ok(PortfolioReport {
        schema_version: PORTFOLIO_SCHEMA_VERSION,
        protocol_version: PORTFOLIO_PROTOCOL_VERSION.into(),
        portfolio_id,
        data_hash,
        broker_spec_hash,
        config: config.clone(),
        source_candidate_count,
        selected: allocations,
        pairwise_correlations,
        maximum_observed_pairwise_correlation,
        symbol_exposures,
        cohort_exposures,
        expected_return_percent,
        path_maximum_drawdown_percent,
        objective_score,
        portfolio_return_path,
        stress,
    })
}

fn pairwise(selected: &[&PreparedCandidate<'_>]) -> Vec<PairwiseCorrelation> {
    let mut values = Vec::new();
    for left in 0..selected.len() {
        for right in (left + 1)..selected.len() {
            values.push(PairwiseCorrelation {
                left: selected[left].candidate.strategy_fingerprint.clone(),
                right: selected[right].candidate.strategy_fingerprint.clone(),
                correlation: correlation(&selected[left].returns, &selected[right].returns),
            });
        }
    }
    values.sort_by(|left, right| {
        left.left
            .cmp(&right.left)
            .then_with(|| left.right.cmp(&right.right))
    });
    values
}

fn exposures(
    allocations: &[PortfolioAllocation],
    key: impl Fn(&PortfolioAllocation) -> &str,
) -> BTreeMap<String, f64> {
    let mut values = BTreeMap::new();
    for allocation in allocations {
        *values.entry(key(allocation).to_owned()).or_default() += allocation.weight;
    }
    values
}

fn stress_portfolio(returns: &[f64], config: &PortfolioConfig) -> PortfolioStressReport {
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut trial_results = Vec::with_capacity(config.stress_trials);
    for trial in 0..config.stress_trials {
        let mut path = Vec::with_capacity(returns.len());
        while path.len() < returns.len() {
            let start = rng.gen_range(0..returns.len());
            for offset in 0..config.stress_block_length {
                if path.len() == returns.len() {
                    break;
                }
                path.push(returns[(start + offset) % returns.len()]);
            }
        }
        trial_results.push(PortfolioStressTrial {
            trial,
            return_percent: path.iter().sum::<f64>() * 100.0,
            maximum_drawdown_percent: maximum_drawdown_percent(&path),
        });
    }
    let mut return_distribution: Vec<_> = trial_results
        .iter()
        .map(|trial| trial.return_percent)
        .collect();
    let mut drawdown_distribution: Vec<_> = trial_results
        .iter()
        .map(|trial| trial.maximum_drawdown_percent)
        .collect();
    return_distribution.sort_by(f64::total_cmp);
    drawdown_distribution.sort_by(f64::total_cmp);
    let tail_count = ((config.stress_trials as f64 * config.cvar_tail_fraction).ceil() as usize)
        .clamp(1, config.stress_trials);
    PortfolioStressReport {
        method: "circular_moving_block_bootstrap_v1".into(),
        seed: config.seed,
        trials: config.stress_trials,
        block_length: config.stress_block_length,
        tail_fraction: config.cvar_tail_fraction,
        p05_return_percent: percentile(&return_distribution, 0.05),
        cvar_return_percent: return_distribution[..tail_count].iter().sum::<f64>()
            / tail_count as f64,
        p95_maximum_drawdown_percent: percentile(&drawdown_distribution, 0.95),
        trial_results,
    }
}

fn report_is_better(candidate: &PortfolioReport, current: &PortfolioReport) -> bool {
    candidate
        .objective_score
        .total_cmp(&current.objective_score)
        .then_with(|| candidate.selected.len().cmp(&current.selected.len()))
        .then_with(|| current.portfolio_id.cmp(&candidate.portfolio_id))
        .is_gt()
}

fn minimum_strategy_count(maximum_weight: f64) -> usize {
    ((1.0 / maximum_weight) - 1.0e-12).ceil() as usize
}

fn exposure_count(maximum_exposure: f64, target_count: usize) -> usize {
    (maximum_exposure * target_count as f64 + 1.0e-12).floor() as usize
}

fn historical_cvar(values: &[f64], tail_fraction: f64) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by(f64::total_cmp);
    let count = ((ordered.len() as f64 * tail_fraction).ceil() as usize).clamp(1, ordered.len());
    ordered[..count].iter().sum::<f64>() / count as f64
}

fn percentile(sorted: &[f64], probability: f64) -> f64 {
    let index = (probability * (sorted.len() - 1) as f64).round() as usize;
    sorted[index]
}

fn cumulative_path(returns: &[f64]) -> Vec<f64> {
    let mut equity = 1.0;
    returns
        .iter()
        .map(|value| {
            equity += value;
            equity
        })
        .collect()
}

fn maximum_drawdown_percent(returns: &[f64]) -> f64 {
    let mut equity = 1.0;
    let mut peak = equity;
    let mut maximum = 0.0_f64;
    for value in returns {
        equity += value;
        peak = peak.max(equity);
        if peak > 0.0 {
            maximum = maximum.max((peak - equity) / peak * 100.0);
        }
    }
    maximum
}

fn correlation(left: &[f64], right: &[f64]) -> f64 {
    let left_mean = left.iter().sum::<f64>() / left.len() as f64;
    let right_mean = right.iter().sum::<f64>() / right.len() as f64;
    let mut covariance = 0.0;
    let mut left_variance = 0.0;
    let mut right_variance = 0.0;
    for (left, right) in left.iter().zip(right) {
        let left_delta = left - left_mean;
        let right_delta = right - right_mean;
        covariance += left_delta * right_delta;
        left_variance += left_delta * left_delta;
        right_variance += right_delta * right_delta;
    }
    let denominator = (left_variance * right_variance).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        (covariance / denominator).clamp(-1.0, 1.0)
    }
}

#[derive(Debug, Error)]
pub enum PortfolioError {
    #[error("invalid portfolio configuration: {0}")]
    InvalidConfig(String),
    #[error("no portfolio candidates were supplied")]
    NoCandidates,
    #[error("invalid portfolio candidate: {0}")]
    InvalidCandidate(String),
    #[error("no feasible portfolio: {0}")]
    NoFeasiblePortfolio(String),
    #[error("portfolio construction violated a hard constraint")]
    InternalConstraintViolation,
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        label: &str,
        symbol: &str,
        cohort: &str,
        signature: Vec<f64>,
    ) -> PortfolioCandidate {
        PortfolioCandidate {
            strategy_fingerprint: ContentHash::sha256(label),
            symbol: symbol.into(),
            cohort: cohort.into(),
            initial_balance: 100_000.0,
            return_percent: 10.0,
            maximum_drawdown_percent: 5.0,
            equity_signature: signature,
        }
    }

    fn diverse_candidates() -> Vec<PortfolioCandidate> {
        vec![
            candidate("a", "EURUSD", "trend", vec![2.0, 0.0, 2.0, 0.0]),
            candidate("b", "EURUSD", "trend", vec![2.0, 2.0, 0.0, 0.0]),
            candidate("c", "EURUSD", "momentum", vec![2.0, 0.0, 0.0, 2.0]),
            candidate("d", "EURUSD", "momentum", vec![0.0, 0.0, 2.0, 2.0]),
        ]
    }

    #[test]
    fn hard_caps_produce_a_diverse_equal_weight_pack() {
        let config = PortfolioConfig {
            maximum_pairwise_correlation: 0.1,
            maximum_weight_per_strategy: 0.25,
            maximum_cohort_exposure: 0.50,
            maximum_strategies: 4,
            stress_trials: 100,
            stress_block_length: 2,
            ..PortfolioConfig::default()
        };
        let report = pack_portfolio(
            &diverse_candidates(),
            ContentHash::sha256("data"),
            ContentHash::sha256("broker"),
            config,
        )
        .unwrap();
        assert_eq!(report.selected.len(), 4);
        assert!(report.selected.iter().all(|value| value.weight == 0.25));
        assert!(
            report
                .cohort_exposures
                .values()
                .all(|exposure| *exposure <= 0.5)
        );
        assert!(report.maximum_observed_pairwise_correlation <= 0.1);
        assert_eq!(report.stress.trial_results.len(), 100);
    }

    #[test]
    fn correlated_clones_cannot_satisfy_the_weight_cap() {
        let mut candidates = diverse_candidates();
        candidates.truncate(1);
        let mut clone = candidates[0].clone();
        clone.strategy_fingerprint = ContentHash::sha256("clone");
        candidates.push(clone);
        let error = pack_portfolio(
            &candidates,
            ContentHash::sha256("data"),
            ContentHash::sha256("broker"),
            PortfolioConfig {
                maximum_pairwise_correlation: 0.5,
                maximum_weight_per_strategy: 0.5,
                maximum_cohort_exposure: 1.0,
                maximum_strategies: 2,
                ..PortfolioConfig::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, PortfolioError::NoFeasiblePortfolio(_)));
    }

    #[test]
    fn fixed_seed_makes_full_stress_record_reproducible() {
        let config = PortfolioConfig {
            maximum_pairwise_correlation: 0.1,
            maximum_weight_per_strategy: 0.25,
            maximum_cohort_exposure: 0.5,
            maximum_strategies: 4,
            stress_trials: 50,
            stress_block_length: 2,
            seed: 99,
            ..PortfolioConfig::default()
        };
        let first = pack_portfolio(
            &diverse_candidates(),
            ContentHash::sha256("data"),
            ContentHash::sha256("broker"),
            config.clone(),
        )
        .unwrap();
        let second = pack_portfolio(
            &diverse_candidates(),
            ContentHash::sha256("data"),
            ContentHash::sha256("broker"),
            config,
        )
        .unwrap();
        assert_eq!(first, second);
    }
}
