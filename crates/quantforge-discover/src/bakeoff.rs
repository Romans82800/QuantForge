//! Short Fast Scout per Search Family, ranked by OOS1 retention.

use crate::engine::evolve_new_with_pack;
use crate::model::{DiscoverConfig, DiscoverError, DiscoverRunMode, SearchFamily};
use crate::multi_symbol::PackSymbol;
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FamilyBakeoffConfig {
    pub discover: DiscoverConfig,
    /// Generations of Fast Scout per family (after the initial batch).
    pub generations: u64,
    pub families: Vec<SearchFamily>,
}

impl Default for FamilyBakeoffConfig {
    fn default() -> Self {
        let discover = DiscoverConfig {
            run_mode: DiscoverRunMode::FastScout,
            require_m1_robustness: false,
            require_m1_precision: false,
            initial_candidates: 60,
            batch_size: 30,
            mutate_after_elites: 12,
            early_stop_pot_elites: Some(6),
            worker_threads: 1,
            ..DiscoverConfig::default()
        };
        Self {
            discover,
            generations: 3,
            families: SearchFamily::ALL.to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyBakeoffRow {
    pub family: SearchFamily,
    pub median_oos1_expectancy: f64,
    pub median_retention: f64,
    pub pass_rate: f64,
    pub elites: usize,
    pub pot_elites: usize,
    pub evaluations: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyBakeoffReport {
    pub rows: Vec<FamilyBakeoffRow>,
    pub recommended: Option<SearchFamily>,
}

/// Run a short Fast Scout for each family and rank by median OOS1 retention.
pub fn run_family_bakeoff(
    dataset: &BarDataset,
    oos1_dataset: Option<&BarDataset>,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    pack: &[PackSymbol],
    primary_symbol: &str,
    config: FamilyBakeoffConfig,
) -> Result<FamilyBakeoffReport, DiscoverError> {
    let mut rows = Vec::with_capacity(config.families.len());
    for family in &config.families {
        let mut discover = config.discover.clone();
        discover.search_family = *family;
        discover.run_mode = DiscoverRunMode::FastScout;
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
        let retentions: Vec<f64> = bank
            .accepted_pool
            .iter()
            .chain(bank.elites.iter())
            .filter_map(|elite| elite.oos1_expectancy_ratio)
            .collect();
        let oos1_values: Vec<f64> = bank
            .accepted_pool
            .iter()
            .chain(bank.elites.iter())
            .filter_map(|elite| elite.oos1_expectancy)
            .collect();
        let pass_rate = if retentions.is_empty() {
            0.0
        } else {
            retentions
                .iter()
                .filter(|&&value| value >= bank.config.oos1_expectancy_retention)
                .count() as f64
                / retentions.len() as f64
        };
        rows.push(FamilyBakeoffRow {
            family: *family,
            median_oos1_expectancy: median(&oos1_values),
            median_retention: median(&retentions),
            pass_rate,
            elites: bank.elites.len(),
            pot_elites: bank.accepted_pool.len(),
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
                    .median_oos1_expectancy
                    .partial_cmp(&left.median_oos1_expectancy)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let recommended = rows
        .iter()
        .find(|row| row.pot_elites > 0 || row.elites > 0)
        .map(|row| row.family);
    Ok(FamilyBakeoffReport { rows, recommended })
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

    #[test]
    fn bakeoff_ranks_by_retention() {
        let mut rows = [
            FamilyBakeoffRow {
                family: SearchFamily::TrendPullback,
                median_oos1_expectancy: 0.2,
                median_retention: 0.5,
                pass_rate: 0.2,
                elites: 0,
                pot_elites: 1,
                evaluations: 10,
            },
            FamilyBakeoffRow {
                family: SearchFamily::MomentumBurst,
                median_oos1_expectancy: 0.1,
                median_retention: 0.9,
                pass_rate: 0.8,
                elites: 1,
                pot_elites: 2,
                evaluations: 10,
            },
        ];
        rows.sort_by(|left, right| {
            right
                .median_retention
                .partial_cmp(&left.median_retention)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        assert_eq!(rows[0].family, SearchFamily::MomentumBurst);
    }
}
