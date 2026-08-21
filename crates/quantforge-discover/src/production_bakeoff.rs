//! One-shot offline comparison of the current Holding battery with a simple
//! IS-evidence ranking lane.
//!
//! This module deliberately does not participate in Discover promotion. It
//! consumes a frozen Holding cohort and sealed-period replay results, then
//! produces one immutable comparison report.

use crate::holding_corr::pearson;
use crate::{Databank, Elite, entry_family_key, model::NicheKey};
use chrono::{DateTime, Datelike, Utc};
use quantforge_core::ContentHash;
use quantforge_eval::{BacktestMetrics, EquityPoint, Trade};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCTION_BAKEOFF_SCHEMA_VERSION: u16 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffConfig {
    pub seed: u64,
    pub selection_fraction: f64,
    pub minimum_lift_r: f64,
    pub maximum_drawdown_multiplier: f64,
    pub maximum_drawdown_additive_percent: f64,
}

impl Default for ProductionBakeoffConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            selection_fraction: 0.20,
            minimum_lift_r: 0.10,
            maximum_drawdown_multiplier: 1.25,
            maximum_drawdown_additive_percent: 5.0,
        }
    }
}

impl ProductionBakeoffConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0 < self.selection_fraction && self.selection_fraction <= 1.0) {
            return Err("selection_fraction must be greater than 0 and at most 1".into());
        }
        if !self.minimum_lift_r.is_finite() || self.minimum_lift_r < 0.0 {
            return Err("minimum_lift_r must be finite and non-negative".into());
        }
        if !self.maximum_drawdown_multiplier.is_finite() || self.maximum_drawdown_multiplier < 1.0 {
            return Err("maximum_drawdown_multiplier must be finite and at least 1".into());
        }
        if !self.maximum_drawdown_additive_percent.is_finite()
            || self.maximum_drawdown_additive_percent < 0.0
        {
            return Err("maximum_drawdown_additive_percent must be finite and non-negative".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProductionBakeoffStrictInput {
    pub tested_fingerprints: BTreeSet<String>,
    pub passed_fingerprints: BTreeSet<String>,
    pub rejection_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SealedCandidateResult {
    pub metrics: BacktestMetrics,
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffStrictSummary {
    pub tested: usize,
    pub passed: usize,
    pub rejected: usize,
    pub rejection_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffPortfolioSummary {
    pub initial_balance: f64,
    pub ending_balance: f64,
    pub net_profit: f64,
    pub return_percent: f64,
    pub max_drawdown: f64,
    pub max_drawdown_percent: f64,
    pub recovery_factor: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffYearSummary {
    pub year: i32,
    pub trades: usize,
    pub net_profit: f64,
    pub expectancy_r: f64,
    pub positive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffCandidateResult {
    pub strategy_id: String,
    pub fingerprint: String,
    pub evaluated: bool,
    pub expectancy_r: Option<f64>,
    pub median_r: Option<f64>,
    pub trade_count: Option<usize>,
    pub net_profit: Option<f64>,
    pub return_percent: Option<f64>,
    pub max_drawdown: Option<f64>,
    pub max_drawdown_percent: Option<f64>,
    pub recovery_factor: Option<f64>,
    pub positive_expectancy: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffSummary {
    pub selected: usize,
    pub evaluated: usize,
    pub evaluation_errors: usize,
    pub positive_expectancy_count: usize,
    pub positive_expectancy_rate: f64,
    pub median_expectancy_r: Option<f64>,
    pub mean_expectancy_r: Option<f64>,
    pub median_trade_count: Option<f64>,
    pub total_trade_count: usize,
    pub total_net_profit: f64,
    pub portfolio: ProductionBakeoffPortfolioSummary,
    pub candidate_results: Vec<ProductionBakeoffCandidateResult>,
    pub calendar_years: Vec<ProductionBakeoffYearSummary>,
    pub positive_calendar_year_fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffArmReport {
    pub name: String,
    pub selected: usize,
    pub selected_ids: Vec<String>,
    pub summary: ProductionBakeoffSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffDecision {
    pub adopt_simple_lane: bool,
    pub simple_positive_aggregate: bool,
    pub simple_positive_median: bool,
    pub simple_beats_random: bool,
    pub simple_drawdown_ok: bool,
    pub simple_positive_calendar_years: bool,
    pub simple_vs_random_lift_r: Option<f64>,
    pub simple_vs_strict_lift_r: Option<f64>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionBakeoffReport {
    pub schema_version: u16,
    pub config: ProductionBakeoffConfig,
    pub cohort_ids: Vec<String>,
    pub cohort_fingerprints: Vec<String>,
    pub eligible_count: usize,
    pub selection_budget: usize,
    pub unsealed_data_hash: ContentHash,
    pub sealed_data_hash: ContentHash,
    pub sealed_start_timestamp_ms: i64,
    pub sealed_end_timestamp_ms_exclusive: i64,
    pub basic_gates: crate::GateConfig,
    pub correlation_threshold: f64,
    pub max_promoted_per_niche: usize,
    pub max_per_entry_family: usize,
    pub strict: ProductionBakeoffStrictSummary,
    pub arms: Vec<ProductionBakeoffArmReport>,
    pub decision: ProductionBakeoffDecision,
}

#[derive(Debug, Clone)]
struct RankedCandidate {
    index: usize,
    score: f64,
    median_fold_r: f64,
    neighborhood_survival: f64,
    recovery_factor: f64,
    drawdown_percent: f64,
}

pub fn run_production_bakeoff(
    bank: &Databank,
    candidates: &[Elite],
    strict_input: &ProductionBakeoffStrictInput,
    sealed_results: &BTreeMap<String, SealedCandidateResult>,
    unsealed_data_hash: ContentHash,
    sealed_data_hash: ContentHash,
    sealed_start_timestamp_ms: i64,
    sealed_end_timestamp_ms_exclusive: i64,
    config: ProductionBakeoffConfig,
) -> Result<ProductionBakeoffReport, String> {
    config.validate()?;
    if candidates.is_empty() {
        return Err("production bakeoff requires at least one frozen candidate".into());
    }
    let mut fingerprints = BTreeSet::new();
    for candidate in candidates {
        if !fingerprints.insert(candidate.structural_fingerprint.to_string()) {
            return Err(format!(
                "production bakeoff candidate cohort contains duplicate fingerprint {}",
                candidate.structural_fingerprint
            ));
        }
    }
    if !strict_input.tested_fingerprints.is_empty()
        && !strict_input
            .tested_fingerprints
            .iter()
            .all(|fingerprint| fingerprints.contains(fingerprint))
    {
        return Err("strict battery contains a fingerprint absent from the frozen cohort".into());
    }

    let eligible: Vec<usize> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, elite)| {
            passes_basic_gates(&elite.metrics, &bank.config.deposit_gates).then_some(index)
        })
        .collect();
    let selection_budget = if eligible.is_empty() {
        0
    } else {
        ((eligible.len() as f64 * config.selection_fraction).ceil() as usize).max(1)
    };

    let ranked = ranked_candidates(candidates, &eligible);
    let simple_indices = select_with_diversity(
        candidates,
        &ranked.iter().map(|row| row.index).collect::<Vec<_>>(),
        selection_budget,
        bank.config.correlation_threshold,
        bank.config.max_promoted_per_niche,
        bank.config.max_per_entry_family,
    );
    let random_order = random_order(&eligible, config.seed);
    let random_indices = select_with_diversity(
        candidates,
        &random_order,
        simple_indices.len(),
        bank.config.correlation_threshold,
        bank.config.max_promoted_per_niche,
        bank.config.max_per_entry_family,
    );

    let strict_indices = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, elite)| {
            strict_input
                .passed_fingerprints
                .contains(&elite.structural_fingerprint.to_string())
                .then_some(index)
        })
        .collect::<Vec<_>>();

    let strict_summary = ProductionBakeoffStrictSummary {
        tested: strict_input.tested_fingerprints.len(),
        passed: strict_input.passed_fingerprints.len(),
        rejected: strict_input
            .tested_fingerprints
            .len()
            .saturating_sub(strict_input.passed_fingerprints.len()),
        rejection_counts: strict_input.rejection_counts.clone(),
    };

    let arms = vec![
        summarize_arm(
            "strict_battery",
            candidates,
            &strict_indices,
            sealed_results,
        ),
        summarize_arm("simple_rank", candidates, &simple_indices, sealed_results),
        summarize_arm(
            "random_control",
            candidates,
            &random_indices,
            sealed_results,
        ),
    ];
    let decision = make_decision(&arms, &config);

    Ok(ProductionBakeoffReport {
        schema_version: PRODUCTION_BAKEOFF_SCHEMA_VERSION,
        config,
        cohort_ids: candidates
            .iter()
            .map(|candidate| candidate.strategy.id.clone())
            .collect(),
        cohort_fingerprints: candidates
            .iter()
            .map(|candidate| candidate.structural_fingerprint.to_string())
            .collect(),
        eligible_count: eligible.len(),
        selection_budget,
        unsealed_data_hash,
        sealed_data_hash,
        sealed_start_timestamp_ms,
        sealed_end_timestamp_ms_exclusive,
        basic_gates: bank.config.deposit_gates.clone(),
        correlation_threshold: bank.config.correlation_threshold,
        max_promoted_per_niche: bank.config.max_promoted_per_niche,
        max_per_entry_family: bank.config.max_per_entry_family,
        strict: strict_summary,
        arms,
        decision,
    })
}

fn passes_basic_gates(metrics: &BacktestMetrics, gates: &crate::GateConfig) -> bool {
    let profit_factor = metrics.profit_factor.unwrap_or_else(|| {
        if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::INFINITY
        } else {
            0.0
        }
    });
    metrics.trade_count >= gates.minimum_trades
        && metrics.max_drawdown_percent <= gates.maximum_drawdown_percent
        && metrics.return_percent > gates.minimum_return_percent
        && profit_factor >= gates.minimum_profit_factor
        && metrics.recovery_factor() >= gates.minimum_recovery_factor
}

fn ranked_candidates(candidates: &[Elite], eligible: &[usize]) -> Vec<RankedCandidate> {
    let mut ranked = eligible
        .iter()
        .map(|&index| {
            let elite = &candidates[index];
            let neighborhood_survival = elite
                .robustness
                .as_ref()
                .map(|evidence| evidence.parameter_neighborhood.survival_fraction)
                .filter(|value| value.is_finite())
                .unwrap_or(f64::NEG_INFINITY);
            RankedCandidate {
                index,
                score: elite.metrics.expectancy_r * (elite.metrics.trade_count as f64).sqrt(),
                median_fold_r: elite.fold_r.median_fold_r,
                neighborhood_survival,
                recovery_factor: elite.metrics.recovery_factor(),
                drawdown_percent: elite.metrics.max_drawdown_percent,
            }
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| right.median_fold_r.total_cmp(&left.median_fold_r))
            .then_with(|| {
                right
                    .neighborhood_survival
                    .total_cmp(&left.neighborhood_survival)
            })
            .then_with(|| right.recovery_factor.total_cmp(&left.recovery_factor))
            .then_with(|| left.drawdown_percent.total_cmp(&right.drawdown_percent))
            .then_with(|| left.index.cmp(&right.index))
    });
    ranked
}

fn random_order(eligible: &[usize], seed: u64) -> Vec<usize> {
    use rand::SeedableRng;
    use rand::seq::SliceRandom;
    use rand_chacha::ChaCha8Rng;

    let mut order = eligible.to_vec();
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    order.shuffle(&mut rng);
    order
}

fn select_with_diversity(
    candidates: &[Elite],
    order: &[usize],
    target: usize,
    correlation_threshold: f64,
    max_promoted_per_niche: usize,
    max_per_entry_family: usize,
) -> Vec<usize> {
    let mut selected: Vec<usize> = Vec::new();
    let mut niches: BTreeMap<NicheKey, usize> = BTreeMap::new();
    let mut families: BTreeMap<String, usize> = BTreeMap::new();
    for &index in order {
        if selected.len() >= target {
            break;
        }
        let candidate = &candidates[index];
        if max_promoted_per_niche > 0
            && niches.get(&candidate.niche).copied().unwrap_or(0) >= max_promoted_per_niche
        {
            continue;
        }
        let family = entry_family_key(&candidate.strategy);
        if max_per_entry_family > 0
            && families.get(&family).copied().unwrap_or(0) >= max_per_entry_family
        {
            continue;
        }
        let too_correlated = selected.iter().any(|&peer| {
            pearson(
                &candidate.equity_signature,
                &candidates[peer].equity_signature,
            ) > correlation_threshold
        });
        if too_correlated {
            continue;
        }
        selected.push(index);
        *niches.entry(candidate.niche.clone()).or_default() += 1;
        *families.entry(family).or_default() += 1;
    }
    selected
}

fn summarize_arm(
    name: &str,
    candidates: &[Elite],
    indices: &[usize],
    sealed_results: &BTreeMap<String, SealedCandidateResult>,
) -> ProductionBakeoffArmReport {
    let selected_ids = indices
        .iter()
        .map(|&index| candidates[index].strategy.id.clone())
        .collect::<Vec<_>>();
    let results = indices
        .iter()
        .filter_map(|&index| {
            sealed_results.get(&candidates[index].structural_fingerprint.to_string())
        })
        .collect::<Vec<_>>();
    let candidate_results = indices
        .iter()
        .map(|&index| {
            let candidate = &candidates[index];
            let fingerprint = candidate.structural_fingerprint.to_string();
            let result = sealed_results.get(&fingerprint);
            ProductionBakeoffCandidateResult {
                strategy_id: candidate.strategy.id.clone(),
                fingerprint,
                evaluated: result.is_some(),
                expectancy_r: result.map(|value| value.metrics.expectancy_r),
                median_r: result.map(|value| value.metrics.median_r),
                trade_count: result.map(|value| value.metrics.trade_count),
                net_profit: result.map(|value| value.metrics.net_profit),
                return_percent: result.map(|value| value.metrics.return_percent),
                max_drawdown: result.map(|value| value.metrics.max_drawdown),
                max_drawdown_percent: result.map(|value| value.metrics.max_drawdown_percent),
                recovery_factor: result.map(|value| value.metrics.recovery_factor()),
                positive_expectancy: result.map(|value| value.metrics.expectancy_r > 0.0),
            }
        })
        .collect();
    let summary = summarize_results(indices.len(), &results, candidate_results);
    ProductionBakeoffArmReport {
        name: name.into(),
        selected: indices.len(),
        selected_ids,
        summary,
    }
}

fn summarize_results(
    selected: usize,
    results: &[&SealedCandidateResult],
    candidate_results: Vec<ProductionBakeoffCandidateResult>,
) -> ProductionBakeoffSummary {
    let expectancy = results
        .iter()
        .map(|result| result.metrics.expectancy_r)
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    let trade_counts = results
        .iter()
        .map(|result| result.metrics.trade_count as f64)
        .collect::<Vec<_>>();
    let positive_expectancy_count = expectancy.iter().filter(|value| **value > 0.0).count();
    let total_trade_count = results
        .iter()
        .map(|result| result.metrics.trade_count)
        .sum::<usize>();
    let total_net_profit = results
        .iter()
        .map(|result| result.metrics.net_profit)
        .sum::<f64>();
    let calendar_years = calendar_years(results);
    let positive_calendar_year_fraction = if calendar_years.is_empty() {
        0.0
    } else {
        calendar_years.iter().filter(|year| year.positive).count() as f64
            / calendar_years.len() as f64
    };
    ProductionBakeoffSummary {
        selected,
        evaluated: results.len(),
        evaluation_errors: selected.saturating_sub(results.len()),
        positive_expectancy_count,
        positive_expectancy_rate: rate(positive_expectancy_count, expectancy.len()),
        median_expectancy_r: median(expectancy.clone()),
        mean_expectancy_r: (!expectancy.is_empty())
            .then(|| expectancy.iter().sum::<f64>() / expectancy.len() as f64),
        median_trade_count: median(trade_counts),
        total_trade_count,
        total_net_profit,
        portfolio: aggregate_portfolio(results),
        candidate_results,
        calendar_years,
        positive_calendar_year_fraction,
    }
}

fn calendar_years(results: &[&SealedCandidateResult]) -> Vec<ProductionBakeoffYearSummary> {
    #[derive(Default)]
    struct Accumulator {
        trades: usize,
        net_profit: f64,
        total_r: f64,
    }
    let mut years: BTreeMap<i32, Accumulator> = BTreeMap::new();
    for result in results {
        for trade in &result.trades {
            let Some(date_time) = DateTime::<Utc>::from_timestamp_millis(trade.exit_timestamp_ms)
            else {
                continue;
            };
            let row = years.entry(date_time.year()).or_default();
            row.trades += 1;
            row.net_profit += trade.net_profit;
            row.total_r += trade.r_multiple;
        }
    }
    years
        .into_iter()
        .map(|(year, row)| ProductionBakeoffYearSummary {
            year,
            trades: row.trades,
            net_profit: row.net_profit,
            expectancy_r: if row.trades == 0 {
                0.0
            } else {
                row.total_r / row.trades as f64
            },
            positive: row.net_profit > 0.0,
        })
        .collect()
}

fn aggregate_portfolio(results: &[&SealedCandidateResult]) -> ProductionBakeoffPortfolioSummary {
    let initial_balance = results
        .iter()
        .map(|result| result.metrics.initial_balance)
        .sum::<f64>();
    let net_profit = results
        .iter()
        .map(|result| result.metrics.net_profit)
        .sum::<f64>();
    let ending_balance = initial_balance + net_profit;
    let mut timestamps = BTreeSet::new();
    for result in results {
        timestamps.extend(result.equity.iter().map(|point| point.timestamp_ms));
    }
    let mut cursors = vec![0usize; results.len()];
    let mut latest = vec![0.0f64; results.len()];
    let mut peak = initial_balance;
    let mut max_drawdown: f64 = 0.0;
    let mut max_drawdown_percent: f64 = 0.0;
    for timestamp in timestamps {
        let mut portfolio_equity = 0.0;
        for (index, result) in results.iter().enumerate() {
            while cursors[index] < result.equity.len()
                && result.equity[cursors[index]].timestamp_ms <= timestamp
            {
                latest[index] = result.equity[cursors[index]].equity;
                cursors[index] += 1;
            }
            portfolio_equity += if latest[index] == 0.0 {
                result.metrics.initial_balance
            } else {
                latest[index]
            };
        }
        peak = peak.max(portfolio_equity);
        let drawdown = peak - portfolio_equity;
        max_drawdown = max_drawdown.max(drawdown);
        if peak > 0.0 {
            max_drawdown_percent = max_drawdown_percent.max(drawdown / peak * 100.0);
        }
    }
    ProductionBakeoffPortfolioSummary {
        initial_balance,
        ending_balance,
        net_profit,
        return_percent: if initial_balance > 0.0 {
            net_profit / initial_balance * 100.0
        } else {
            0.0
        },
        max_drawdown,
        max_drawdown_percent,
        recovery_factor: if max_drawdown > 1.0e-12 {
            net_profit / max_drawdown
        } else if net_profit > 0.0 {
            f64::INFINITY
        } else {
            net_profit
        },
    }
}

fn make_decision(
    arms: &[ProductionBakeoffArmReport],
    config: &ProductionBakeoffConfig,
) -> ProductionBakeoffDecision {
    let strict = arms.iter().find(|arm| arm.name == "strict_battery");
    let simple = arms.iter().find(|arm| arm.name == "simple_rank");
    let random = arms.iter().find(|arm| arm.name == "random_control");
    let simple_median = simple.and_then(|arm| arm.summary.median_expectancy_r);
    let random_median = random.and_then(|arm| arm.summary.median_expectancy_r);
    let strict_median = strict.and_then(|arm| arm.summary.median_expectancy_r);
    let simple_vs_random_lift_r = simple_median.zip(random_median).map(|(a, b)| a - b);
    let simple_vs_strict_lift_r = simple_median.zip(strict_median).map(|(a, b)| a - b);
    let simple_positive_aggregate =
        simple.is_some_and(|arm| arm.summary.portfolio.net_profit > 0.0);
    let simple_positive_median = simple.is_some_and(|arm| {
        arm.summary
            .median_expectancy_r
            .is_some_and(|value| value > 0.0)
    });
    let simple_beats_random = simple_vs_random_lift_r
        .is_some_and(|lift| lift >= config.minimum_lift_r)
        && random.is_some_and(|arm| arm.summary.evaluated > 0);
    let simple_drawdown_ok = match (simple, random) {
        (Some(simple), Some(random)) if random.summary.selected > 0 => {
            simple.summary.portfolio.max_drawdown_percent
                <= (random.summary.portfolio.max_drawdown_percent
                    * config.maximum_drawdown_multiplier)
                    .max(
                        random.summary.portfolio.max_drawdown_percent
                            + config.maximum_drawdown_additive_percent,
                    )
        }
        _ => false,
    };
    let simple_positive_calendar_years = simple.is_some_and(|arm| {
        !arm.summary.calendar_years.is_empty() && arm.summary.positive_calendar_year_fraction >= 0.5
    });
    let adopt_simple_lane = simple_positive_aggregate
        && simple_positive_median
        && simple_beats_random
        && simple_drawdown_ok
        && simple_positive_calendar_years;
    let reason = if adopt_simple_lane {
        "simple_rank passed the precommitted sealed-period adoption rules".into()
    } else {
        "simple_rank did not pass every precommitted sealed-period adoption rule; keep current production behavior".into()
    };
    ProductionBakeoffDecision {
        adopt_simple_lane,
        simple_positive_aggregate,
        simple_positive_median,
        simple_beats_random,
        simple_drawdown_ok,
        simple_positive_calendar_years,
        simple_vs_random_lift_r,
        simple_vs_strict_lift_r,
        reason,
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2.0
    })
}

fn rate(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::methodology::{FactorRecipe, build_factor_strategy};
    use crate::model::{
        BehaviorDescriptor, DiscoverConfig, Elite, EvidenceComponents, LongShortSkewBucket,
        NicheKey, ThreeLevelBucket,
    };
    use quantforge_core::FloatPolicy;

    fn test_bank() -> Databank {
        let hash = ContentHash::sha256(Vec::<u8>::new());
        let mut config = DiscoverConfig::default();
        config.correlation_threshold = 1.1;
        config.max_promoted_per_niche = 0;
        config.max_per_entry_family = 0;
        config.deposit_gates.minimum_trades = 1;
        config.deposit_gates.maximum_drawdown_percent = 100.0;
        config.deposit_gates.minimum_return_percent = 0.0;
        config.deposit_gates.minimum_profit_factor = 1.0;
        config.deposit_gates.minimum_recovery_factor = 0.0;
        Databank {
            schema_version: crate::DATABANK_SCHEMA_VERSION,
            grammar_version: crate::GRAMMAR_VERSION.into(),
            data_hash: hash.clone(),
            execution_data_hash: hash.clone(),
            broker_spec_hash: hash,
            config,
            completed_generations: 0,
            evaluation_count: 0,
            elites: Vec::new(),
            coverage_map: BTreeMap::new(),
            accepted_pool: Vec::new(),
            accepted_coverage_map: BTreeMap::new(),
            specialist_pool: Vec::new(),
            specialist_coverage_map: BTreeMap::new(),
            holding: Vec::new(),
            holding_coverage_map: BTreeMap::new(),
            telemetry: Default::default(),
        }
    }

    fn elite(sequence: u64, expectancy_r: f64) -> Elite {
        let strategy =
            build_factor_strategy(sequence + 1, sequence, 2, 1, FactorRecipe::SimpleMarket);
        let fingerprint = strategy
            .structural_fingerprint(FloatPolicy::default())
            .unwrap();
        let trade_count = 100;
        let net_profit = expectancy_r * trade_count as f64 * 100.0;
        let metrics = BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 100_000.0 + net_profit,
            net_profit,
            return_percent: 10.0,
            trade_count,
            winning_trades: 60,
            losing_trades: 40,
            win_rate: 60.0,
            profit_factor: Some(1.5),
            max_drawdown: 1_000.0,
            max_drawdown_percent: 5.0,
            sharpe_ratio: Some(1.0),
            expectancy: net_profit / trade_count as f64,
            expectancy_r,
            median_r: expectancy_r,
        };
        Elite {
            strategy,
            structural_fingerprint: fingerprint,
            descriptor: BehaviorDescriptor {
                entry_conditions: 2,
                exit_conditions: 1,
                trades_per_1000_bars: 10.0,
                average_bars_held: 4.0,
                drawdown_percent: 5.0,
                win_rate_percent: 60.0,
                long_short_skew: 0.0,
            },
            niche: NicheKey {
                entry_conditions: 2,
                trade_frequency: ThreeLevelBucket::Medium,
                hold_time: ThreeLevelBucket::Medium,
                drawdown: ThreeLevelBucket::Low,
                win_rate: ThreeLevelBucket::Medium,
                long_short_skew: LongShortSkewBucket::Balanced,
            },
            evidence: EvidenceComponents {
                return_component: 1.0,
                profit_factor_component: 1.0,
                trade_count_bonus: 1.0,
                drawdown_penalty: 0.0,
                complexity_penalty: 0.0,
                total: 3.0,
            },
            novelty: 0.0,
            complexity: 1,
            metrics,
            is_expectancy: expectancy_r,
            oos1_expectancy: None,
            oos1_expectancy_ratio: None,
            fold_r: crate::FoldRStats {
                fold_count: 3,
                median_fold_r: expectancy_r,
                fold_spread: 0.01,
                pooled_r: expectancy_r,
                max_year_share: 0.5,
                has_negative_fold: false,
                usable: true,
            },
            observed_trade_sharpe: None,
            expected_max_lucky_sharpe: None,
            deflated_trade_sharpe: None,
            multi_symbol_results: Vec::new(),
            gate_results: Vec::new(),
            robustness: None,
            equity_signature: vec![sequence as f64, 0.0, 1.0, 0.5],
            discovered_generation: 0,
            island_id: 0,
        }
    }

    fn sealed_result(expectancy_r: f64) -> SealedCandidateResult {
        let metrics = BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 100_000.0,
            net_profit: 0.0,
            return_percent: 0.0,
            trade_count: 0,
            winning_trades: 0,
            losing_trades: 0,
            win_rate: 0.0,
            profit_factor: None,
            max_drawdown: 0.0,
            max_drawdown_percent: 0.0,
            sharpe_ratio: None,
            expectancy: expectancy_r,
            expectancy_r,
            median_r: expectancy_r,
        };
        SealedCandidateResult {
            metrics,
            trades: Vec::new(),
            equity: Vec::new(),
        }
    }

    #[test]
    fn config_rejects_invalid_selection_fraction() {
        let mut config = ProductionBakeoffConfig::default();
        config.selection_fraction = 0.0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn median_is_deterministic() {
        assert_eq!(median(vec![3.0, 1.0, 2.0]), Some(2.0));
        assert_eq!(median(vec![4.0, 1.0, 3.0, 2.0]), Some(2.5));
    }

    #[test]
    fn strict_summary_counts_only_the_frozen_battery_rows() {
        let input = ProductionBakeoffStrictInput {
            tested_fingerprints: ["a", "b", "c"].into_iter().map(String::from).collect(),
            passed_fingerprints: ["a"].into_iter().map(String::from).collect(),
            rejection_counts: [("folds".into(), 2)].into_iter().collect(),
        };
        assert_eq!(input.tested_fingerprints.len(), 3);
        assert_eq!(input.passed_fingerprints.len(), 1);
    }

    #[test]
    fn sealed_results_cannot_change_eligibility_or_ranking() {
        let bank = test_bank();
        let candidates = vec![elite(1, 0.40), elite(2, 0.20), elite(3, 0.10)];
        let strict = ProductionBakeoffStrictInput::default();
        let config = ProductionBakeoffConfig {
            selection_fraction: 0.20,
            ..Default::default()
        };
        let without_sealed = run_production_bakeoff(
            &bank,
            &candidates,
            &strict,
            &BTreeMap::new(),
            ContentHash::sha256(vec![1]),
            ContentHash::sha256(vec![2]),
            10,
            20,
            config.clone(),
        )
        .unwrap();
        let with_sealed = run_production_bakeoff(
            &bank,
            &candidates,
            &strict,
            &candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    (
                        candidate.structural_fingerprint.to_string(),
                        sealed_result(if index == 0 { 0.50 } else { -0.20 }),
                    )
                })
                .collect(),
            ContentHash::sha256(vec![1]),
            ContentHash::sha256(vec![2]),
            10,
            20,
            config,
        )
        .unwrap();
        assert_eq!(without_sealed.eligible_count, with_sealed.eligible_count);
        assert_eq!(
            without_sealed.selection_budget,
            with_sealed.selection_budget
        );
        assert_eq!(
            without_sealed.arms[1].selected_ids,
            with_sealed.arms[1].selected_ids
        );
        assert_eq!(
            with_sealed.arms[1].selected_ids,
            vec![candidates[0].strategy.id.clone()]
        );
    }

    #[test]
    fn selection_is_deterministic_and_random_control_has_same_budget() {
        let bank = test_bank();
        let candidates = (0..10)
            .map(|index| elite(index, 0.10 + index as f64 * 0.01))
            .collect::<Vec<_>>();
        let config = ProductionBakeoffConfig {
            selection_fraction: 0.20,
            ..Default::default()
        };
        let first = run_production_bakeoff(
            &bank,
            &candidates,
            &ProductionBakeoffStrictInput::default(),
            &BTreeMap::new(),
            ContentHash::sha256(vec![1]),
            ContentHash::sha256(vec![2]),
            10,
            20,
            config.clone(),
        )
        .unwrap();
        let second = run_production_bakeoff(
            &bank,
            &candidates,
            &ProductionBakeoffStrictInput::default(),
            &BTreeMap::new(),
            ContentHash::sha256(vec![1]),
            ContentHash::sha256(vec![2]),
            10,
            20,
            config,
        )
        .unwrap();
        assert_eq!(first.arms, second.arms);
        assert_eq!(first.selection_budget, 2);
        assert_eq!(first.arms[1].selected, first.arms[2].selected);
    }

    #[test]
    fn zero_survivor_strict_battery_is_a_valid_arm() {
        let bank = test_bank();
        let candidates = vec![elite(1, 0.20), elite(2, 0.10)];
        let fingerprint = candidates[0].structural_fingerprint.to_string();
        let strict = ProductionBakeoffStrictInput {
            tested_fingerprints: [fingerprint].into_iter().collect(),
            passed_fingerprints: BTreeSet::new(),
            rejection_counts: [("fold stability".into(), 1)].into_iter().collect(),
        };
        let report = run_production_bakeoff(
            &bank,
            &candidates,
            &strict,
            &BTreeMap::new(),
            ContentHash::sha256(vec![1]),
            ContentHash::sha256(vec![2]),
            10,
            20,
            ProductionBakeoffConfig::default(),
        )
        .unwrap();
        assert_eq!(report.strict.passed, 0);
        assert_eq!(report.arms[0].selected, 0);
        assert_eq!(report.arms[0].summary.evaluated, 0);
        assert_eq!(report.arms[0].summary.evaluation_errors, 0);
    }
}
