//! Equal-budget Development scout per entry-condition count.
//!
//! This replaces the old per-family bakeoff. Families said what indicators a
//! strategy was allowed to use; the question that actually matters is how many
//! mirrored entry conditions survive out of sample, because every extra
//! condition is another degree of freedom to overfit.

use crate::engine::evolve_new_with_pack;
use crate::model::{DiscoverConfig, DiscoverError, DiscoverRunMode, UniversalGrammarConfig};
use crate::multi_symbol::PackSymbol;
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
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

/// Run an equal-budget Development scout for each entry-condition count. OOS1
/// is deliberately unavailable here; this diagnostic may guide grammar choice
/// and therefore is part of research, not certification.
pub fn run_condition_bakeoff(
    dataset: &BarDataset,
    certification_oos1: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    config: ConditionBakeoffConfig,
) -> Result<ConditionBakeoffReport, DiscoverError> {
    if certification_oos1.is_some() {
        return Err(DiscoverError::InvalidConfig(
            "condition bakeoff cannot consume OOS1; compare grammars on Development only".into(),
        ));
    }
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
            None,
            m1_dataset,
            broker,
            pack,
            primary_symbol,
            discover,
            config.generations,
        )?;
        let is_values_r = bank
            .accepted_pool
            .iter()
            .map(|elite| elite.is_expectancy / crate::FIXED_RISK_PER_TRADE)
            .collect::<Vec<_>>();
        rows.push(ConditionBakeoffRow {
            entry_conditions,
            median_is_expectancy_r: median(&is_values_r),
            median_oos1_expectancy_r: 0.0,
            median_retention: 0.0,
            pass_rate: 0.0,
            elites: bank.elites.len(),
            pot_elites: bank.accepted_pool.len(),
            oos1_tested: 0,
            evaluations: bank.evaluation_count,
        });
    }
    rows.sort_by(|left, right| {
        right
            .median_is_expectancy_r
            .partial_cmp(&left.median_is_expectancy_r)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Break remaining ties toward the simpler arm.
            .then_with(|| left.entry_conditions.cmp(&right.entry_conditions))
    });
    // This is a development diagnostic. Prefer the simplest viable arm within
    // 90% of the best Development expectancy; never call it OOS validated.
    let best = rows
        .iter()
        .map(|row| row.median_is_expectancy_r)
        .fold(0.0_f64, f64::max);
    let recommended = rows
        .iter()
        .filter(|row| row.median_is_expectancy_r > 0.0)
        .filter(|row| row.median_is_expectancy_r + 1e-12 >= best * 0.9)
        .min_by_key(|row| row.entry_conditions)
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
    fn bakeoff_ranks_by_development_expectancy() {
        let mut rows = [row(4, 0.5), row(2, 0.9)];
        rows[0].median_is_expectancy_r = 0.1;
        rows[1].median_is_expectancy_r = 0.3;
        rows.sort_by(|left, right| {
            right
                .median_is_expectancy_r
                .partial_cmp(&left.median_is_expectancy_r)
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
