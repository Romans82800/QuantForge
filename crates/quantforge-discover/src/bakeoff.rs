//! Equal-budget Fast Scout per entry-condition count, ranked by OOS1 retention.
//!
//! This replaces the old per-family bakeoff. Families said what indicators a
//! strategy was allowed to use; the question that actually matters is how many
//! mirrored entry conditions survive out of sample, because every extra
//! condition is another degree of freedom to overfit.

use crate::engine::{evolve_new_with_pack, passes_oos1_pick};
use crate::model::{DiscoverConfig, DiscoverError, DiscoverRunMode, UniversalGrammarConfig};
use crate::multi_symbol::PackSymbol;
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::evaluate_strategy;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionBakeoffConfig {
    pub discover: DiscoverConfig,
    /// Generations of Fast Scout per condition count (after the initial batch).
    pub generations: u64,
    /// Entry-condition counts to compare, each pinned exactly.
    pub entry_condition_counts: Vec<usize>,
}

impl Default for ConditionBakeoffConfig {
    fn default() -> Self {
        let discover = DiscoverConfig {
            run_mode: DiscoverRunMode::FastScout,
            require_m1_robustness: false,
            require_m1_precision: false,
            initial_candidates: 60,
            batch_size: 30,
            mutate_after_elites: 12,
            // A comparison needs the same planned sample for every arm.
            // Fast Scout otherwise stops once a pot reaches eight members.
            early_stop_pot_elites: Some(usize::MAX),
            worker_threads: 1,
            ..DiscoverConfig::default()
        };
        Self {
            discover,
            generations: 3,
            entry_condition_counts: vec![2, 3, 4],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionBakeoffRow {
    pub entry_conditions: usize,
    /// Expectancy normalized by the fixed $1,000 risk policy.
    pub median_is_expectancy_r: f64,
    /// Expectancy normalized by the fixed $1,000 risk policy.
    pub median_oos1_expectancy_r: f64,
    pub median_retention: f64,
    /// OOS1 passes / every current unique candidate in the eligible pot.
    pub pass_rate: f64,
    pub elites: usize,
    pub pot_elites: usize,
    pub oos1_tested: usize,
    pub evaluations: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionBakeoffReport {
    pub rows: Vec<ConditionBakeoffRow>,
    pub recommended: Option<usize>,
}

/// Run an equal-budget Fast Scout for each entry-condition count, then
/// independently recheck every retained pot member on OOS1 before ranking it.
pub fn run_condition_bakeoff(
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    config: ConditionBakeoffConfig,
) -> Result<ConditionBakeoffReport, DiscoverError> {
    let mut rows = Vec::with_capacity(config.entry_condition_counts.len());
    for entry_conditions in &config.entry_condition_counts {
        let entry_conditions =
            (*entry_conditions).clamp(2, UniversalGrammarConfig::MAX_ENTRY_CONDITIONS);
        let mut discover = config.discover.clone();
        discover.universal_grammar.minimum_entry_conditions = entry_conditions;
        discover.universal_grammar.maximum_entry_conditions = entry_conditions;
        discover.run_mode = DiscoverRunMode::FastScout;
        // Keep each arm on the same planned evaluation budget even when the
        // caller supplied a generic DiscoverConfig rather than this type's
        // default preset.
        discover.early_stop_pot_elites = Some(usize::MAX);
        discover.apply_run_mode();
        let bank = evolve_new_with_pack(
            dataset,
            oos1_dataset,
            m1_dataset,
            broker,
            pack,
            primary_symbol,
            discover,
            config.generations,
        )?;
        // Production Discover preserves OOS1 metrics only for candidates that
        // pass OOS1 into the databank. Aggregating those persisted fields here
        // made the tester display a mechanical 100% pass rate. Re-evaluate
        // every current pot member on the held-out OOS1 partition instead.
        let mut is_values_r = Vec::with_capacity(bank.accepted_pool.len());
        let mut oos1_values_r = Vec::with_capacity(bank.accepted_pool.len());
        let mut retentions = Vec::with_capacity(bank.accepted_pool.len());
        let mut passes = 0usize;
        if let Some(oos1) = oos1_dataset {
            for elite in &bank.accepted_pool {
                let oos1_result =
                    evaluate_strategy(&elite.strategy, oos1, broker, &bank.config.scout)?;
                let is_expectancy = elite.is_expectancy;
                let oos1_expectancy = oos1_result.metrics.expectancy;
                is_values_r.push(is_expectancy / crate::FIXED_RISK_PER_TRADE);
                oos1_values_r.push(oos1_expectancy / crate::FIXED_RISK_PER_TRADE);
                if is_expectancy > 0.0 && is_expectancy.is_finite() && oos1_expectancy.is_finite() {
                    retentions.push(oos1_expectancy / is_expectancy);
                }
                if passes_oos1_pick(
                    is_expectancy,
                    oos1_expectancy,
                    bank.config.oos1_expectancy_retention,
                ) {
                    passes += 1;
                }
            }
        }
        let oos1_tested = oos1_values_r.len();
        let pass_rate = if oos1_tested == 0 {
            0.0
        } else {
            passes as f64 / oos1_tested as f64
        };
        rows.push(ConditionBakeoffRow {
            entry_conditions,
            median_is_expectancy_r: median(&is_values_r),
            median_oos1_expectancy_r: median(&oos1_values_r),
            median_retention: median(&retentions),
            pass_rate,
            elites: bank.elites.len(),
            pot_elites: bank.accepted_pool.len(),
            oos1_tested,
            evaluations: bank.evaluation_count,
        });
    }
    rows.sort_by(|left, right| {
        right
            .median_retention
            .partial_cmp(&left.median_retention)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                right
                    .median_oos1_expectancy_r
                    .partial_cmp(&left.median_oos1_expectancy_r)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            // Break remaining ties toward the simpler arm.
            .then_with(|| left.entry_conditions.cmp(&right.entry_conditions))
    });
    let recommended = rows
        .iter()
        .find(|row| {
            row.oos1_tested >= 10
                && row.pass_rate >= 0.50
                && row.median_retention >= 0.70
                && row.median_oos1_expectancy_r > 0.0
        })
        .map(|row| row.entry_conditions);
    Ok(ConditionBakeoffReport { rows, recommended })
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(entry_conditions: usize, retention: f64) -> ConditionBakeoffRow {
        ConditionBakeoffRow {
            entry_conditions,
            median_is_expectancy_r: 0.2,
            median_oos1_expectancy_r: 0.2,
            median_retention: retention,
            pass_rate: 0.5,
            elites: 1,
            pot_elites: 1,
            oos1_tested: 1,
            evaluations: 10,
        }
    }

    #[test]
    fn bakeoff_ranks_by_retention() {
        let mut rows = [row(4, 0.5), row(2, 0.9)];
        rows.sort_by(|left, right| {
            right
                .median_retention
                .partial_cmp(&left.median_retention)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        assert_eq!(rows[0].entry_conditions, 2);
    }

    #[test]
    fn default_config_compares_two_three_and_four_conditions() {
        assert_eq!(
            ConditionBakeoffConfig::default().entry_condition_counts,
            vec![2, 3, 4]
        );
    }
}
