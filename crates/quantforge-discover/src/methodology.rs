//! Offline methodology factor grid: what keeps OOS1 per-trade R across grammar sizes.

use crate::grammar::{build_seed, rng_for};
use crate::model::{DiscoverError, SearchFamily, UniversalGrammarConfig};
use crate::{FIXED_RISK_PER_TRADE, FROZEN_ATR_PERIOD, GRAMMAR_VERSION};
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::{BacktestMetrics, ScoutConfig, evaluate_strategy};
use quantforge_ir::{
    EntryDistancePolicy, EntryOrderPolicy, ManagePolicy, PartialExit, ProtectiveStops,
    StopLossPolicy, StrategyIr, TakeProfitPolicy, TrailingPolicy,
};
use rand::Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Exit / entry-order recipe under test (orthogonal to grammar condition counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactorRecipe {
    /// Market entry, time stop only (production simple-exits shape).
    SimpleMarket,
    /// Market entry with a single break-even move.
    BreakEven,
    /// Market entry with a risk-multiple trailing stop.
    TrailingStop,
    /// Market entry with one partial exit.
    PartialExit,
    /// Stop pending entry, with no management feature.
    StopEntry,
    /// Limit pending entry, with no management feature.
    LimitEntry,
}

impl FactorRecipe {
    pub const ALL: [Self; 6] = [
        Self::SimpleMarket,
        Self::BreakEven,
        Self::TrailingStop,
        Self::PartialExit,
        Self::StopEntry,
        Self::LimitEntry,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SimpleMarket => "simple_market",
            Self::BreakEven => "break_even",
            Self::TrailingStop => "trailing_stop",
            Self::PartialExit => "partial_exit",
            Self::StopEntry => "stop_entry",
            Self::LimitEntry => "limit_entry",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodologyGridConfig {
    pub seed: u64,
    pub draws_per_cell: usize,
    pub entry_condition_counts: Vec<usize>,
    pub exit_condition_counts: Vec<usize>,
    pub recipes: Vec<FactorRecipe>,
    pub scout: ScoutConfig,
    /// Soft IS screen before counting a draw toward OOS stats.
    pub minimum_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub oos1_retention: f64,
}

impl Default for MethodologyGridConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            draws_per_cell: 40,
            entry_condition_counts: vec![2, 3, 4],
            exit_condition_counts: vec![1, 2, 3],
            recipes: FactorRecipe::ALL.to_vec(),
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
pub struct FactorDraw {
    pub entry_conditions: usize,
    pub exit_conditions: usize,
    pub recipe: FactorRecipe,
    pub entry_kind: String,
    pub complexity: usize,
    /// Mean per-trade R on the development partition.
    pub is_expectancy: f64,
    /// Mean per-trade R on the chronological OOS1 partition.
    pub oos1_expectancy: f64,
    pub retention: Option<f64>,
    pub is_trades: usize,
    pub passed_is_screen: bool,
    pub passed_oos1_pick: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorCellSummary {
    pub entry_conditions: usize,
    pub exit_conditions: usize,
    pub recipe: FactorRecipe,
    pub draws: usize,
    pub screened: usize,
    pub oos1_pass_rate: f64,
    pub median_retention: Option<f64>,
    pub median_oos1_expectancy: Option<f64>,
    pub median_complexity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorContrast {
    pub name: String,
    pub baseline: String,
    pub treatment: String,
    pub baseline_n: usize,
    pub treatment_n: usize,
    pub baseline_median_retention: Option<f64>,
    pub treatment_median_retention: Option<f64>,
    pub retention_lift: Option<f64>,
    pub baseline_oos1_pass_rate: f64,
    pub treatment_oos1_pass_rate: f64,
    pub pass_rate_lift: f64,
    pub p_value: f64,
    pub q_value: f64,
    pub significant_fdr_10: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodologyReport {
    pub grammar_version: String,
    pub config: MethodologyGridConfig,
    pub evaluations: usize,
    pub draws: Vec<FactorDraw>,
    pub cells: Vec<FactorCellSummary>,
    pub contrasts: Vec<FactorContrast>,
    pub recommendations: Vec<String>,
}

pub fn run_methodology_grid(
    is_dataset: &BarDataset,
    oos1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: MethodologyGridConfig,
) -> Result<MethodologyReport, DiscoverError> {
    if config.draws_per_cell == 0 {
        return Err(DiscoverError::InvalidConfig(
            "draws_per_cell must be > 0".into(),
        ));
    }
    if config.entry_condition_counts.is_empty()
        || config.exit_condition_counts.is_empty()
        || config.recipes.is_empty()
    {
        return Err(DiscoverError::InvalidConfig(
            "methodology grid needs entry counts, exit counts, and recipes".into(),
        ));
    }
    if config
        .entry_condition_counts
        .iter()
        .any(|count| !(2..=UniversalGrammarConfig::MAX_ENTRY_CONDITIONS).contains(count))
    {
        return Err(DiscoverError::InvalidConfig(
            "methodology entry condition counts must be within 2..=4".into(),
        ));
    }
    if config
        .exit_condition_counts
        .iter()
        .any(|count| !(1..=3).contains(count))
    {
        return Err(DiscoverError::InvalidConfig(
            "methodology exit condition counts must be within 1..=3".into(),
        ));
    }

    let mut jobs = Vec::new();
    for &entry_conditions in &config.entry_condition_counts {
        for &exit_conditions in &config.exit_condition_counts {
            for recipe in &config.recipes {
                for draw in 0..config.draws_per_cell {
                    jobs.push((entry_conditions, exit_conditions, *recipe, draw as u64));
                }
            }
        }
    }

    let scout = config.scout.clone();
    let seed = config.seed;
    let draws: Vec<FactorDraw> = jobs
        .into_par_iter()
        .map(|(entry_conditions, exit_conditions, recipe, draw)| {
            let sequence = mix_sequence(entry_conditions, exit_conditions, recipe, draw);
            let strategy =
                build_factor_strategy(seed, sequence, entry_conditions, exit_conditions, recipe);
            let entry_kind = entry_kind_label(&strategy.entry.order);
            let complexity = strategy.complexity().score;
            let is_result = evaluate_strategy(&strategy, is_dataset, broker, &scout);
            let oos_result = evaluate_strategy(&strategy, oos1_dataset, broker, &scout);
            match (is_result, oos_result) {
                (Ok(is_run), Ok(oos_run)) => {
                    let passed_is = passes_screen(
                        &is_run.metrics,
                        config.minimum_trades,
                        config.minimum_return_percent,
                        config.minimum_profit_factor,
                        config.maximum_drawdown_percent,
                    );
                    // Dollar expectancy changes with position sizing and is not
                    // comparable across strategies. Use the per-trade R measure
                    // everywhere the methodology compares expectancy or retention.
                    let is_e = risk_normalized_expectancy(&is_run.metrics);
                    let oos_e = risk_normalized_expectancy(&oos_run.metrics);
                    let retention = (is_e > 0.0 && is_e.is_finite() && oos_e.is_finite())
                        .then_some(oos_e / is_e);
                    let passed_oos1 = passed_is
                        && is_e > 0.0
                        && oos_e > 0.0
                        && retention.is_some_and(|value| value >= config.oos1_retention);
                    FactorDraw {
                        entry_conditions,
                        exit_conditions,
                        recipe,
                        entry_kind,
                        complexity,
                        is_expectancy: is_e,
                        oos1_expectancy: oos_e,
                        retention,
                        is_trades: is_run.metrics.trade_count,
                        passed_is_screen: passed_is,
                        passed_oos1_pick: passed_oos1,
                    }
                }
                _ => FactorDraw {
                    entry_conditions,
                    exit_conditions,
                    recipe,
                    entry_kind,
                    complexity,
                    is_expectancy: f64::NAN,
                    oos1_expectancy: f64::NAN,
                    retention: None,
                    is_trades: 0,
                    passed_is_screen: false,
                    passed_oos1_pick: false,
                },
            }
        })
        .collect();

    let evaluations = draws.len() * 2;
    let cells = summarize_cells(&draws, &config);
    let mut contrasts = build_contrasts(&draws);
    apply_benjamini_hochberg(&mut contrasts, 0.10);
    let recommendations = recommend(&cells, &contrasts);

    Ok(MethodologyReport {
        grammar_version: GRAMMAR_VERSION.into(),
        config,
        evaluations,
        draws,
        cells,
        contrasts,
        recommendations,
    })
}

fn mix_sequence(
    entry_conditions: usize,
    exit_conditions: usize,
    recipe: FactorRecipe,
    draw: u64,
) -> u64 {
    let recipe_i = FactorRecipe::ALL
        .iter()
        .position(|value| *value == recipe)
        .unwrap_or(0) as u64;
    (entry_conditions as u64)
        .wrapping_mul(1_000_003)
        .wrapping_add((exit_conditions as u64).wrapping_mul(10_007))
        .wrapping_add(recipe_i.wrapping_mul(97))
        .wrapping_add(draw)
}

pub(crate) fn build_factor_strategy(
    seed: u64,
    sequence: u64,
    entry_conditions: usize,
    exit_conditions: usize,
    recipe: FactorRecipe,
) -> StrategyIr {
    let mut rng = rng_for(seed, 17, sequence);
    let universal = UniversalGrammarConfig {
        minimum_entry_conditions: entry_conditions,
        maximum_entry_conditions: entry_conditions,
        minimum_exit_conditions: exit_conditions,
        maximum_exit_conditions: exit_conditions,
        ..UniversalGrammarConfig::default()
    };
    let mut strategy = build_seed(
        SearchFamily::Universal,
        &mut rng,
        format!(
            "method-e{}-x{}-{}-{sequence}",
            entry_conditions,
            exit_conditions,
            recipe.label()
        ),
        entry_conditions,
        true,
        true,
        &universal,
    );
    let order = match recipe {
        FactorRecipe::SimpleMarket
        | FactorRecipe::BreakEven
        | FactorRecipe::TrailingStop
        | FactorRecipe::PartialExit => EntryOrderPolicy::Market,
        FactorRecipe::StopEntry | FactorRecipe::LimitEntry => {
            let distance = EntryDistancePolicy::AtrMultiple {
                period: FROZEN_ATR_PERIOD,
                multiplier: [0.25, 0.5, 0.75, 1.0, 1.25, 1.5][rng.gen_range(0..6)],
            };
            let expiry_bars = rng.gen_range(2..=8);
            if recipe == FactorRecipe::StopEntry {
                EntryOrderPolicy::Stop {
                    distance,
                    expiry_bars,
                }
            } else {
                EntryOrderPolicy::Limit {
                    distance,
                    expiry_bars,
                }
            }
        }
    };
    let manage = match recipe {
        FactorRecipe::SimpleMarket | FactorRecipe::StopEntry | FactorRecipe::LimitEntry => {
            ManagePolicy {
                time_stop_bars: Some(rng.gen_range(4..=16)),
                break_even_at_r: None,
                trailing: None,
                partial_exits: Vec::new(),
                flatten_end_of_day: false,
                max_one_entry_per_day: true,
                ..Default::default()
            }
        }
        FactorRecipe::BreakEven => ManagePolicy {
            time_stop_bars: Some(rng.gen_range(4..=16)),
            break_even_at_r: Some(1.0),
            trailing: None,
            partial_exits: Vec::new(),
            flatten_end_of_day: false,
            max_one_entry_per_day: true,
            ..Default::default()
        },
        FactorRecipe::TrailingStop => ManagePolicy {
            time_stop_bars: Some(rng.gen_range(4..=16)),
            break_even_at_r: None,
            trailing: Some(TrailingPolicy::RiskMultiple {
                activate_at_r: 1.5,
                distance_r: 1.0,
            }),
            partial_exits: Vec::new(),
            flatten_end_of_day: false,
            max_one_entry_per_day: true,
            ..Default::default()
        },
        FactorRecipe::PartialExit => ManagePolicy {
            time_stop_bars: Some(rng.gen_range(4..=16)),
            break_even_at_r: None,
            trailing: None,
            partial_exits: vec![PartialExit {
                at_r: 1.0,
                fraction: 0.5,
            }],
            flatten_end_of_day: false,
            max_one_entry_per_day: true,
            ..Default::default()
        },
    };
    let atr_stop = 1.0 + (rng.gen_range(0..9) as f64) * 0.25;
    let risk_tp = 1.0 + (rng.gen_range(0..9) as f64) * 0.25;
    strategy.entry.order = order;
    strategy.risk = quantforge_ir::RiskPolicy::FixedCurrency {
        amount: FIXED_RISK_PER_TRADE,
    };
    strategy.stops = ProtectiveStops {
        stop_loss: StopLossPolicy::AtrMultiple {
            period: FROZEN_ATR_PERIOD,
            multiplier: atr_stop.clamp(1.0, 4.0),
        },
        take_profit: TakeProfitPolicy::RiskMultiple {
            multiple: risk_tp.clamp(1.0, 4.0),
        },
    };
    strategy.manage = manage;
    strategy.meta.complexity = strategy.complexity().score.min(u16::MAX as usize) as u16;
    strategy
}

pub(crate) fn passes_screen(
    metrics: &BacktestMetrics,
    minimum_trades: usize,
    minimum_return_percent: f64,
    minimum_profit_factor: f64,
    maximum_drawdown_percent: f64,
) -> bool {
    let pf = metrics
        .profit_factor
        .unwrap_or(if metrics.net_profit > 0.0 {
            f64::INFINITY
        } else {
            0.0
        });
    metrics.trade_count >= minimum_trades
        && metrics.return_percent > minimum_return_percent
        && pf >= minimum_profit_factor
        && metrics.max_drawdown_percent <= maximum_drawdown_percent
}

fn risk_normalized_expectancy(metrics: &BacktestMetrics) -> f64 {
    metrics.expectancy_r
}

fn summarize_cells(draws: &[FactorDraw], config: &MethodologyGridConfig) -> Vec<FactorCellSummary> {
    let mut cells = Vec::new();
    for &entry_conditions in &config.entry_condition_counts {
        for &exit_conditions in &config.exit_condition_counts {
            for recipe in &config.recipes {
                let group: Vec<&FactorDraw> = draws
                    .iter()
                    .filter(|draw| {
                        draw.entry_conditions == entry_conditions
                            && draw.exit_conditions == exit_conditions
                            && draw.recipe == *recipe
                    })
                    .collect();
                let screened: Vec<&FactorDraw> = group
                    .iter()
                    .copied()
                    .filter(|draw| draw.passed_is_screen)
                    .collect();
                let retentions: Vec<f64> =
                    screened.iter().filter_map(|draw| draw.retention).collect();
                let oos_values: Vec<f64> = screened
                    .iter()
                    .map(|draw| draw.oos1_expectancy)
                    .filter(|value| value.is_finite())
                    .collect();
                let complexities: Vec<f64> =
                    screened.iter().map(|draw| draw.complexity as f64).collect();
                let oos_pass = if screened.is_empty() {
                    0.0
                } else {
                    screened.iter().filter(|draw| draw.passed_oos1_pick).count() as f64
                        / screened.len() as f64
                };
                cells.push(FactorCellSummary {
                    entry_conditions,
                    exit_conditions,
                    recipe: *recipe,
                    draws: group.len(),
                    screened: screened.len(),
                    oos1_pass_rate: oos_pass,
                    median_retention: median(&retentions),
                    median_oos1_expectancy: median(&oos_values),
                    median_complexity: median(&complexities),
                });
            }
        }
    }
    cells.sort_by(|left, right| {
        right
            .oos1_pass_rate
            .total_cmp(&left.oos1_pass_rate)
            .then_with(|| {
                right
                    .median_retention
                    .unwrap_or(f64::NEG_INFINITY)
                    .total_cmp(&left.median_retention.unwrap_or(f64::NEG_INFINITY))
            })
    });
    cells
}

fn build_contrasts(draws: &[FactorDraw]) -> Vec<FactorContrast> {
    let screened: Vec<&FactorDraw> = draws
        .iter()
        .filter(|draw| draw.passed_is_screen && draw.retention.is_some())
        .collect();
    let mut contrasts = Vec::new();

    // Recipe vs simple_market (pooled across condition counts).
    let baseline: Vec<f64> = screened
        .iter()
        .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket)
        .filter_map(|draw| draw.retention)
        .collect();
    let baseline_pass = pass_rate(
        &screened
            .iter()
            .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket)
            .copied()
            .collect::<Vec<_>>(),
    );
    for recipe in FactorRecipe::ALL
        .iter()
        .copied()
        .filter(|recipe| *recipe != FactorRecipe::SimpleMarket)
    {
        let treatment: Vec<f64> = screened
            .iter()
            .filter(|draw| draw.recipe == recipe)
            .filter_map(|draw| draw.retention)
            .collect();
        let treatment_rows: Vec<&FactorDraw> = screened
            .iter()
            .copied()
            .filter(|draw| draw.recipe == recipe)
            .collect();
        contrasts.push(contrast(
            format!("recipe:{} vs simple_market", recipe.label()),
            "simple_market",
            recipe.label(),
            &baseline,
            &treatment,
            baseline_pass,
            pass_rate(&treatment_rows),
        ));
    }

    // Entry-condition count 3/4 vs 2 on simple_market only.
    let entry2: Vec<f64> = screened
        .iter()
        .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket && draw.entry_conditions == 2)
        .filter_map(|draw| draw.retention)
        .collect();
    let entry2_rows: Vec<&FactorDraw> = screened
        .iter()
        .copied()
        .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket && draw.entry_conditions == 2)
        .collect();
    for count in [3usize, 4] {
        let treatment: Vec<f64> = screened
            .iter()
            .filter(|draw| {
                draw.recipe == FactorRecipe::SimpleMarket && draw.entry_conditions == count
            })
            .filter_map(|draw| draw.retention)
            .collect();
        let treatment_rows: Vec<&FactorDraw> = screened
            .iter()
            .copied()
            .filter(|draw| {
                draw.recipe == FactorRecipe::SimpleMarket && draw.entry_conditions == count
            })
            .collect();
        contrasts.push(contrast(
            format!("entry_conditions:{count} vs 2 (simple_market)"),
            "entry_conditions=2",
            &format!("entry_conditions={count}"),
            &entry2,
            &treatment,
            pass_rate(&entry2_rows),
            pass_rate(&treatment_rows),
        ));
    }

    // Exit-condition count 2/3 vs 1 on simple_market only.
    let exit1: Vec<f64> = screened
        .iter()
        .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket && draw.exit_conditions == 1)
        .filter_map(|draw| draw.retention)
        .collect();
    let exit1_rows: Vec<&FactorDraw> = screened
        .iter()
        .copied()
        .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket && draw.exit_conditions == 1)
        .collect();
    for count in [2usize, 3] {
        let treatment: Vec<f64> = screened
            .iter()
            .filter(|draw| {
                draw.recipe == FactorRecipe::SimpleMarket && draw.exit_conditions == count
            })
            .filter_map(|draw| draw.retention)
            .collect();
        let treatment_rows: Vec<&FactorDraw> = screened
            .iter()
            .copied()
            .filter(|draw| {
                draw.recipe == FactorRecipe::SimpleMarket && draw.exit_conditions == count
            })
            .collect();
        contrasts.push(contrast(
            format!("exit_conditions:{count} vs 1 (simple_market)"),
            "exit_conditions=1",
            &format!("exit_conditions={count}"),
            &exit1,
            &treatment,
            pass_rate(&exit1_rows),
            pass_rate(&treatment_rows),
        ));
    }

    // High vs low complexity terciles on simple_market.
    let mut simple: Vec<&FactorDraw> = screened
        .iter()
        .copied()
        .filter(|draw| draw.recipe == FactorRecipe::SimpleMarket)
        .collect();
    if simple.len() >= 12 {
        simple.sort_by_key(|draw| draw.complexity);
        let third = simple.len() / 3;
        let low = &simple[..third];
        let high = &simple[simple.len() - third..];
        let low_r: Vec<f64> = low.iter().filter_map(|draw| draw.retention).collect();
        let high_r: Vec<f64> = high.iter().filter_map(|draw| draw.retention).collect();
        contrasts.push(contrast(
            "complexity:high vs low tercile (simple_market)".into(),
            "low_complexity",
            "high_complexity",
            &low_r,
            &high_r,
            pass_rate(low),
            pass_rate(high),
        ));
    }

    // Pending stop vs limit: isolated entry-order contrast.
    let stop: Vec<f64> = screened
        .iter()
        .filter(|draw| draw.entry_kind == "stop" && draw.recipe == FactorRecipe::StopEntry)
        .filter_map(|draw| draw.retention)
        .collect();
    let stop_rows: Vec<&FactorDraw> = screened
        .iter()
        .copied()
        .filter(|draw| draw.entry_kind == "stop" && draw.recipe == FactorRecipe::StopEntry)
        .collect();
    let limit: Vec<f64> = screened
        .iter()
        .filter(|draw| draw.entry_kind == "limit" && draw.recipe == FactorRecipe::LimitEntry)
        .filter_map(|draw| draw.retention)
        .collect();
    let limit_rows: Vec<&FactorDraw> = screened
        .iter()
        .copied()
        .filter(|draw| draw.entry_kind == "limit" && draw.recipe == FactorRecipe::LimitEntry)
        .collect();
    contrasts.push(contrast(
        "pending:stop vs limit".into(),
        "limit",
        "stop",
        &limit,
        &stop,
        pass_rate(&limit_rows),
        pass_rate(&stop_rows),
    ));

    contrasts
}

fn entry_kind_label(order: &EntryOrderPolicy) -> String {
    match order {
        EntryOrderPolicy::Market => "market".into(),
        EntryOrderPolicy::Stop { .. } => "stop".into(),
        EntryOrderPolicy::Limit { .. } => "limit".into(),
    }
}

fn contrast(
    name: String,
    baseline_label: &str,
    treatment_label: &str,
    baseline: &[f64],
    treatment: &[f64],
    baseline_pass: f64,
    treatment_pass: f64,
) -> FactorContrast {
    let baseline_median = median(baseline);
    let treatment_median = median(treatment);
    let retention_lift = match (baseline_median, treatment_median) {
        (Some(base), Some(treat)) => Some(treat - base),
        _ => None,
    };
    FactorContrast {
        name,
        baseline: baseline_label.into(),
        treatment: treatment_label.into(),
        baseline_n: baseline.len(),
        treatment_n: treatment.len(),
        baseline_median_retention: baseline_median,
        treatment_median_retention: treatment_median,
        retention_lift,
        baseline_oos1_pass_rate: baseline_pass,
        treatment_oos1_pass_rate: treatment_pass,
        pass_rate_lift: treatment_pass - baseline_pass,
        p_value: mann_whitney_p(baseline, treatment),
        q_value: 1.0,
        significant_fdr_10: false,
    }
}

fn pass_rate(rows: &[&FactorDraw]) -> f64 {
    if rows.is_empty() {
        0.0
    } else {
        rows.iter().filter(|draw| draw.passed_oos1_pick).count() as f64 / rows.len() as f64
    }
}

fn recommend(cells: &[FactorCellSummary], contrasts: &[FactorContrast]) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(best) = cells.iter().find(|cell| cell.screened >= 5) {
        lines.push(format!(
            "Best cell: entries={} · exits={} · {} · OOS1 pass {:.0}% · median retention {}",
            best.entry_conditions,
            best.exit_conditions,
            best.recipe.label(),
            best.oos1_pass_rate * 100.0,
            best.median_retention
                .map(|value| format!("{value:.3}"))
                .unwrap_or_else(|| "n/a".into())
        ));
    }
    for contrast in contrasts
        .iter()
        .filter(|contrast| contrast.significant_fdr_10)
    {
        lines.push(format!(
            "FDR≤10%: {} (retention lift {:+.3}, pass lift {:+.1}pp, q={:.3})",
            contrast.name,
            contrast.retention_lift.unwrap_or(0.0),
            contrast.pass_rate_lift * 100.0,
            contrast.q_value
        ));
    }
    if !contrasts.iter().any(|contrast| contrast.significant_fdr_10) {
        lines.push(
            "No factor contrast survived FDR 10% — prefer the simplest condition counts and production recipe (simple_market)."
                .into(),
        );
    }
    lines
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let mid = sorted.len() / 2;
    Some(if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    })
}

/// Two-sided Mann–Whitney U via normal approximation (with tie correction omitted for speed).
fn mann_whitney_p(left: &[f64], right: &[f64]) -> f64 {
    let n1 = left.len() as f64;
    let n2 = right.len() as f64;
    if left.is_empty() || right.is_empty() || (n1 + n2) < 8.0 {
        return 1.0;
    }
    let mut ranks = Vec::with_capacity(left.len() + right.len());
    ranks.extend(left.iter().map(|value| (*value, 0u8)));
    ranks.extend(right.iter().map(|value| (*value, 1u8)));
    ranks.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut u1 = 0.0;
    for (rank_index, (_, group)) in ranks.iter().enumerate() {
        if *group == 0 {
            u1 += (rank_index + 1) as f64;
        }
    }
    u1 -= n1 * (n1 + 1.0) / 2.0;
    let mean = n1 * n2 / 2.0;
    let var = n1 * n2 * (n1 + n2 + 1.0) / 12.0;
    if var <= 0.0 {
        return 1.0;
    }
    let z = ((u1 - mean).abs() - 0.5) / var.sqrt();
    // erfc approximation for two-sided normal p
    2.0 * norm_sf(z)
}

fn norm_sf(z: f64) -> f64 {
    // Abramowitz & Stegun 7.1.26 erfc approximation on z/sqrt(2)
    let x = z / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x.abs());
    let poly = t
        * (0.254829592
            + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    let erfc = poly * (-x * x).exp();
    (erfc / 2.0).clamp(0.0, 1.0)
}

fn apply_benjamini_hochberg(contrasts: &mut [FactorContrast], q: f64) {
    let mut order: Vec<usize> = (0..contrasts.len()).collect();
    order.sort_by(|&left, &right| contrasts[left].p_value.total_cmp(&contrasts[right].p_value));
    let m = order.len().max(1) as f64;
    let mut prev = 1.0_f64;
    for (i, &index) in order.iter().enumerate().rev() {
        let rank = (i + 1) as f64;
        let bh = (contrasts[index].p_value * m / rank).min(prev);
        prev = bh;
        contrasts[index].q_value = bh;
        contrasts[index].significant_fdr_10 = bh <= q;
    }
}

#[cfg(test)]
mod tests {
    use super::risk_normalized_expectancy;
    use quantforge_eval::BacktestMetrics;

    #[test]
    fn methodology_expectancy_is_per_trade_r_not_account_currency() {
        let metrics = BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 100_250.0,
            net_profit: 250.0,
            return_percent: 0.25,
            trade_count: 10,
            winning_trades: 6,
            losing_trades: 4,
            win_rate: 60.0,
            profit_factor: Some(1.5),
            max_drawdown: 100.0,
            max_drawdown_percent: 0.1,
            sharpe_ratio: None,
            expectancy: 25.0,
            expectancy_r: 0.125,
            median_r: 0.1,
        };

        assert_eq!(risk_normalized_expectancy(&metrics), 0.125);
    }
}
