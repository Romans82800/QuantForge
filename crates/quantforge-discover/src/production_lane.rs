//! Leakage-safe Development-only selector for the H4 Production Lane v1.
//!
//! The selector accepts only Development replay results. There is deliberately
//! no validation or sealed-result argument, so future rows cannot influence
//! eligibility, ranking, or the selected cohort.

use crate::holding_corr::pearson;
use crate::{Databank, Elite, GateConfig, entry_family_key};
use chrono::{DateTime, Datelike, Utc};
use quantforge_core::ContentHash;
use quantforge_eval::{BacktestMetrics, Trade};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PRODUCTION_LANE_SCHEMA_VERSION: u16 = 1;
pub const PRODUCTION_LANE_ID: &str = "h4_production_lane_v1";
pub const PRODUCTION_LANE_SCORE_FORMULA: &str = "development_expectancy_r * sqrt(trade_count)";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLaneConfig {
    pub selection_fraction: f64,
    pub minimum_full_expectancy_r: f64,
    pub minimum_3m_trades: usize,
    pub minimum_6m_trades: usize,
    pub minimum_12m_trades: usize,
    pub minimum_6m_usable_windows: usize,
    pub minimum_12m_usable_windows: usize,
    pub minimum_window_coverage_fraction: f64,
    pub minimum_6m_positive_fraction: f64,
    pub minimum_12m_positive_fraction: f64,
}

impl Default for ProductionLaneConfig {
    fn default() -> Self {
        Self {
            selection_fraction: 0.20,
            minimum_full_expectancy_r: 0.20,
            minimum_3m_trades: 5,
            minimum_6m_trades: 10,
            minimum_12m_trades: 20,
            minimum_6m_usable_windows: 3,
            minimum_12m_usable_windows: 2,
            minimum_window_coverage_fraction: 0.50,
            minimum_6m_positive_fraction: 0.50,
            minimum_12m_positive_fraction: 0.50,
        }
    }
}

impl ProductionLaneConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0 < self.selection_fraction && self.selection_fraction <= 1.0) {
            return Err("selection_fraction must be in (0, 1]".into());
        }
        if !self.minimum_full_expectancy_r.is_finite() || self.minimum_full_expectancy_r < 0.0 {
            return Err("minimum_full_expectancy_r must be finite and non-negative".into());
        }
        if self.minimum_3m_trades == 0
            || self.minimum_6m_trades == 0
            || self.minimum_12m_trades == 0
            || self.minimum_6m_usable_windows == 0
            || self.minimum_12m_usable_windows == 0
        {
            return Err("window trade and usable-window minimums must be positive".into());
        }
        for (name, value) in [
            (
                "minimum_window_coverage_fraction",
                self.minimum_window_coverage_fraction,
            ),
            (
                "minimum_6m_positive_fraction",
                self.minimum_6m_positive_fraction,
            ),
            (
                "minimum_12m_positive_fraction",
                self.minimum_12m_positive_fraction,
            ),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("{name} must be finite and in [0, 1]"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProductionLaneReplay {
    pub metrics: BacktestMetrics,
    pub trades: Vec<Trade>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLaneWindow {
    pub start_timestamp_ms: i64,
    pub end_timestamp_ms_exclusive: i64,
    pub trades: usize,
    pub expectancy_r: Option<f64>,
    pub positive: Option<bool>,
    pub usable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLaneWindowSummary {
    pub months: usize,
    pub total_windows: usize,
    pub usable_windows: usize,
    pub coverage_fraction: f64,
    pub positive_windows: usize,
    pub positive_fraction: Option<f64>,
    pub median_expectancy_r: Option<f64>,
    pub median_trade_count: Option<f64>,
    pub windows: Vec<ProductionLaneWindow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLaneCandidateRow {
    pub strategy_id: String,
    pub fingerprint: String,
    pub eligible: bool,
    pub selected: bool,
    pub rejection_reasons: Vec<String>,
    pub warnings: Vec<String>,
    /// Absent when the Development replay failed. Keeping this optional avoids
    /// non-finite sentinel values in the immutable JSON report.
    pub score: Option<f64>,
    pub development_expectancy_r: f64,
    pub development_trade_count: usize,
    pub development_return_percent: f64,
    pub development_drawdown_percent: f64,
    pub development_recovery_factor: f64,
    pub three_month: ProductionLaneWindowSummary,
    pub six_month: ProductionLaneWindowSummary,
    pub twelve_month: ProductionLaneWindowSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductionLaneReport {
    pub schema_version: u16,
    pub lane_id: String,
    pub score_formula: String,
    pub config: ProductionLaneConfig,
    pub development_data_hash: ContentHash,
    pub development_start_timestamp_ms: i64,
    pub development_end_timestamp_ms_exclusive: i64,
    pub source_cohort_size: usize,
    pub replayed: usize,
    pub eligible: usize,
    pub selection_budget: usize,
    pub selected: usize,
    pub selected_ids: Vec<String>,
    pub selected_fingerprints: Vec<String>,
    pub basic_gates: GateConfig,
    pub rows: Vec<ProductionLaneCandidateRow>,
}

#[derive(Debug, Clone)]
struct RankedRow {
    index: usize,
    score: f64,
    twelve_month_median: f64,
    six_month_median: f64,
    recovery_factor: f64,
    drawdown_percent: f64,
}

pub fn run_production_lane(
    bank: &Databank,
    candidates: &[Elite],
    replays: &BTreeMap<String, ProductionLaneReplay>,
    development_data_hash: ContentHash,
    development_start_timestamp_ms: i64,
    development_end_timestamp_ms_exclusive: i64,
    config: ProductionLaneConfig,
) -> Result<ProductionLaneReport, String> {
    config.validate()?;
    if candidates.is_empty() {
        return Err("Production Lane requires at least one Holding candidate".into());
    }
    if development_start_timestamp_ms >= development_end_timestamp_ms_exclusive {
        return Err("Development boundary is empty or reversed".into());
    }
    let mut unique = BTreeSet::new();
    for candidate in candidates {
        if !unique.insert(candidate.structural_fingerprint.to_string()) {
            return Err("Production Lane cohort contains a duplicate fingerprint".into());
        }
    }
    for (fingerprint, replay) in replays {
        if !unique.contains(fingerprint) {
            return Err(format!(
                "Development replay {fingerprint} is absent from the frozen Holding cohort"
            ));
        }
        if replay.trades.iter().any(|trade| {
            trade.entry_timestamp_ms < development_start_timestamp_ms
                || trade.entry_timestamp_ms >= development_end_timestamp_ms_exclusive
                || trade.exit_timestamp_ms >= development_end_timestamp_ms_exclusive
        }) {
            return Err(format!(
                "Development replay {fingerprint} contains a trade outside the Development boundary"
            ));
        }
    }

    let mut rows = candidates
        .iter()
        .map(|candidate| {
            candidate_row(
                candidate,
                replays.get(&candidate.structural_fingerprint.to_string()),
                &bank.config.deposit_gates,
                development_start_timestamp_ms,
                development_end_timestamp_ms_exclusive,
                &config,
            )
        })
        .collect::<Vec<_>>();
    let eligible_indices = rows
        .iter()
        .enumerate()
        .filter_map(|(index, row)| row.eligible.then_some(index))
        .collect::<Vec<_>>();
    let selection_budget = if eligible_indices.is_empty() {
        0
    } else {
        ((eligible_indices.len() as f64 * config.selection_fraction).ceil() as usize).max(1)
    };
    let mut ranked = eligible_indices
        .iter()
        .map(|&index| RankedRow {
            index,
            score: rows[index].score.unwrap_or(f64::NEG_INFINITY),
            twelve_month_median: rows[index]
                .twelve_month
                .median_expectancy_r
                .unwrap_or(f64::NEG_INFINITY),
            six_month_median: rows[index]
                .six_month
                .median_expectancy_r
                .unwrap_or(f64::NEG_INFINITY),
            recovery_factor: rows[index].development_recovery_factor,
            drawdown_percent: rows[index].development_drawdown_percent,
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| {
                right
                    .twelve_month_median
                    .total_cmp(&left.twelve_month_median)
            })
            .then_with(|| right.six_month_median.total_cmp(&left.six_month_median))
            .then_with(|| right.recovery_factor.total_cmp(&left.recovery_factor))
            .then_with(|| left.drawdown_percent.total_cmp(&right.drawdown_percent))
            .then_with(|| left.index.cmp(&right.index))
    });
    let ordered = ranked.iter().map(|row| row.index).collect::<Vec<_>>();
    let selected_indices = select_with_diversity(bank, candidates, &ordered, selection_budget);
    for &index in &selected_indices {
        rows[index].selected = true;
    }
    let selected_ids = selected_indices
        .iter()
        .map(|&index| candidates[index].strategy.id.clone())
        .collect::<Vec<_>>();
    let selected_fingerprints = selected_indices
        .iter()
        .map(|&index| candidates[index].structural_fingerprint.to_string())
        .collect::<Vec<_>>();

    Ok(ProductionLaneReport {
        schema_version: PRODUCTION_LANE_SCHEMA_VERSION,
        lane_id: PRODUCTION_LANE_ID.into(),
        score_formula: PRODUCTION_LANE_SCORE_FORMULA.into(),
        config,
        development_data_hash,
        development_start_timestamp_ms,
        development_end_timestamp_ms_exclusive,
        source_cohort_size: candidates.len(),
        replayed: replays.len(),
        eligible: eligible_indices.len(),
        selection_budget,
        selected: selected_indices.len(),
        selected_ids,
        selected_fingerprints,
        basic_gates: bank.config.deposit_gates.clone(),
        rows,
    })
}

fn candidate_row(
    candidate: &Elite,
    replay: Option<&ProductionLaneReplay>,
    gates: &GateConfig,
    start_timestamp_ms: i64,
    end_timestamp_ms_exclusive: i64,
    config: &ProductionLaneConfig,
) -> ProductionLaneCandidateRow {
    let empty = || window_summary(&[], start_timestamp_ms, end_timestamp_ms_exclusive, 3, 1);
    let Some(replay) = replay else {
        return ProductionLaneCandidateRow {
            strategy_id: candidate.strategy.id.clone(),
            fingerprint: candidate.structural_fingerprint.to_string(),
            eligible: false,
            selected: false,
            rejection_reasons: vec!["Development M1 replay failed".into()],
            warnings: Vec::new(),
            score: None,
            development_expectancy_r: 0.0,
            development_trade_count: 0,
            development_return_percent: 0.0,
            development_drawdown_percent: 0.0,
            development_recovery_factor: 0.0,
            three_month: empty(),
            six_month: window_summary(&[], start_timestamp_ms, end_timestamp_ms_exclusive, 6, 1),
            twelve_month: window_summary(
                &[],
                start_timestamp_ms,
                end_timestamp_ms_exclusive,
                12,
                1,
            ),
        };
    };
    let three_month = window_summary(
        &replay.trades,
        start_timestamp_ms,
        end_timestamp_ms_exclusive,
        3,
        config.minimum_3m_trades,
    );
    let six_month = window_summary(
        &replay.trades,
        start_timestamp_ms,
        end_timestamp_ms_exclusive,
        6,
        config.minimum_6m_trades,
    );
    let twelve_month = window_summary(
        &replay.trades,
        start_timestamp_ms,
        end_timestamp_ms_exclusive,
        12,
        config.minimum_12m_trades,
    );
    let mut rejection_reasons = Vec::new();
    if !passes_basic_gates(&replay.metrics, gates) {
        rejection_reasons.push("basic Development gates".into());
    }
    if replay.metrics.expectancy_r < config.minimum_full_expectancy_r {
        rejection_reasons.push(format!(
            "Development expectancy below {:.2}R",
            config.minimum_full_expectancy_r
        ));
    }
    check_required_windows(
        &six_month,
        config.minimum_6m_usable_windows,
        config.minimum_window_coverage_fraction,
        config.minimum_6m_positive_fraction,
        "6-month",
        &mut rejection_reasons,
    );
    check_required_windows(
        &twelve_month,
        config.minimum_12m_usable_windows,
        config.minimum_window_coverage_fraction,
        config.minimum_12m_positive_fraction,
        "12-month",
        &mut rejection_reasons,
    );
    let mut warnings = Vec::new();
    if three_month
        .positive_fraction
        .is_some_and(|value| value < 0.50)
    {
        warnings.push("fewer than half of usable 3-month blocks are positive".into());
    }
    if !candidate.fold_r.passes_stability() {
        warnings.push("calendar-year stability warning".into());
    }
    if let Some(robustness) = candidate.robustness.as_ref() {
        if !robustness.monte_carlo.passed {
            warnings.push("Monte Carlo warning".into());
        }
        if robustness.parameter_neighborhood.survival_fraction
            < robustness.parameter_neighborhood.required_survival_fraction
        {
            warnings.push("parameter-neighborhood warning".into());
        }
    }
    ProductionLaneCandidateRow {
        strategy_id: candidate.strategy.id.clone(),
        fingerprint: candidate.structural_fingerprint.to_string(),
        eligible: rejection_reasons.is_empty(),
        selected: false,
        rejection_reasons,
        warnings,
        score: Some(replay.metrics.expectancy_r * (replay.metrics.trade_count as f64).sqrt()),
        development_expectancy_r: replay.metrics.expectancy_r,
        development_trade_count: replay.metrics.trade_count,
        development_return_percent: replay.metrics.return_percent,
        development_drawdown_percent: replay.metrics.max_drawdown_percent,
        development_recovery_factor: replay.metrics.recovery_factor(),
        three_month,
        six_month,
        twelve_month,
    }
}

fn check_required_windows(
    summary: &ProductionLaneWindowSummary,
    minimum_usable: usize,
    minimum_coverage: f64,
    minimum_positive: f64,
    label: &str,
    reasons: &mut Vec<String>,
) {
    if summary.usable_windows < minimum_usable {
        reasons.push(format!("too few usable {label} blocks"));
    }
    if summary.coverage_fraction < minimum_coverage {
        reasons.push(format!("insufficient {label} trade coverage"));
    }
    if summary
        .positive_fraction
        .is_none_or(|value| value < minimum_positive)
    {
        reasons.push(format!("fewer than required positive {label} blocks"));
    }
    if summary.median_expectancy_r.is_none_or(|value| value <= 0.0) {
        reasons.push(format!("non-positive median {label} expectancy"));
    }
}

fn window_summary(
    trades: &[Trade],
    start_timestamp_ms: i64,
    end_timestamp_ms_exclusive: i64,
    months: usize,
    minimum_trades: usize,
) -> ProductionLaneWindowSummary {
    let start_month = month_index(start_timestamp_ms);
    let end_month = month_index(end_timestamp_ms_exclusive.saturating_sub(1));
    let month_span = end_month.saturating_sub(start_month).saturating_add(1) as usize;
    let total_windows = month_span.div_ceil(months).max(1);
    let mut buckets = vec![Vec::<&Trade>::new(); total_windows];
    for trade in trades {
        let offset = month_index(trade.entry_timestamp_ms).saturating_sub(start_month) as usize;
        let index = (offset / months).min(total_windows - 1);
        buckets[index].push(trade);
    }
    let mut windows = Vec::with_capacity(total_windows);
    for (index, bucket) in buckets.into_iter().enumerate() {
        let start_month_index = start_month + (index * months) as i64;
        let window_start = month_start_timestamp(start_month_index);
        let natural_end = month_start_timestamp(start_month_index + months as i64);
        let window_end = natural_end.min(end_timestamp_ms_exclusive);
        let usable = bucket.len() >= minimum_trades;
        let expectancy = usable.then(|| {
            bucket
                .iter()
                .map(|trade| trade.r_multiple)
                .filter(|value| value.is_finite())
                .sum::<f64>()
                / bucket.len() as f64
        });
        windows.push(ProductionLaneWindow {
            start_timestamp_ms: window_start.max(start_timestamp_ms),
            end_timestamp_ms_exclusive: window_end,
            trades: bucket.len(),
            expectancy_r: expectancy,
            positive: expectancy.map(|value| value > 0.0),
            usable,
        });
    }
    let usable = windows.iter().filter(|window| window.usable).count();
    let positive = windows
        .iter()
        .filter(|window| window.positive == Some(true))
        .count();
    let expectancy = windows
        .iter()
        .filter_map(|window| window.expectancy_r)
        .collect::<Vec<_>>();
    let trade_counts = windows
        .iter()
        .filter(|window| window.usable)
        .map(|window| window.trades as f64)
        .collect::<Vec<_>>();
    ProductionLaneWindowSummary {
        months,
        total_windows,
        usable_windows: usable,
        coverage_fraction: usable as f64 / total_windows as f64,
        positive_windows: positive,
        positive_fraction: (usable > 0).then_some(positive as f64 / usable as f64),
        median_expectancy_r: median(expectancy),
        median_trade_count: median(trade_counts),
        windows,
    }
}

fn select_with_diversity(
    bank: &Databank,
    candidates: &[Elite],
    order: &[usize],
    target: usize,
) -> Vec<usize> {
    let mut selected = Vec::<usize>::new();
    let mut niches = BTreeMap::new();
    let mut families = BTreeMap::new();
    for &index in order {
        if selected.len() >= target {
            break;
        }
        let candidate = &candidates[index];
        if bank.config.max_promoted_per_niche > 0
            && niches.get(&candidate.niche).copied().unwrap_or(0)
                >= bank.config.max_promoted_per_niche
        {
            continue;
        }
        let family = entry_family_key(&candidate.strategy);
        if bank.config.max_per_entry_family > 0
            && families.get(&family).copied().unwrap_or(0) >= bank.config.max_per_entry_family
        {
            continue;
        }
        if selected.iter().any(|&peer| {
            pearson(
                &candidate.equity_signature,
                &candidates[peer].equity_signature,
            ) > bank.config.correlation_threshold
        }) {
            continue;
        }
        selected.push(index);
        *niches.entry(candidate.niche.clone()).or_insert(0usize) += 1;
        *families.entry(family).or_insert(0usize) += 1;
    }
    selected
}

fn passes_basic_gates(metrics: &BacktestMetrics, gates: &GateConfig) -> bool {
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

fn month_index(timestamp_ms: i64) -> i64 {
    let time =
        DateTime::<Utc>::from_timestamp_millis(timestamp_ms).unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    time.year() as i64 * 12 + time.month0() as i64
}

fn month_start_timestamp(index: i64) -> i64 {
    let year = index.div_euclid(12) as i32;
    let month = index.rem_euclid(12) as u32 + 1;
    chrono::NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|time| time.and_utc().timestamp_millis())
        .unwrap_or(0)
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    values.retain(|value| value.is_finite());
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BehaviorDescriptor, EvidenceComponents, FoldRStats, NicheKey};
    use quantforge_core::FloatPolicy;
    use quantforge_eval::{ExitReason, PositionSide};

    fn timestamp(year: i32, month: u32, day: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .timestamp_millis()
    }

    fn trade(timestamp_ms: i64, r: f64) -> Trade {
        Trade {
            side: PositionSide::Long,
            entry_timestamp_ms: timestamp_ms,
            exit_timestamp_ms: timestamp_ms + 60_000,
            entry_price: 1.0,
            exit_price: 1.0,
            volume: 1.0,
            initial_stop_loss: 0.9,
            initial_take_profit: 1.1,
            gross_profit: r * 1_000.0,
            commission: 0.0,
            swap: 0.0,
            net_profit: r * 1_000.0,
            bars_held: 1,
            exit_reason: ExitReason::TakeProfit,
            r_multiple: r,
        }
    }

    fn metrics(expectancy_r: f64, trade_count: usize) -> BacktestMetrics {
        BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 110_000.0,
            net_profit: 10_000.0,
            return_percent: 10.0,
            trade_count,
            winning_trades: trade_count,
            losing_trades: 0,
            win_rate: 100.0,
            profit_factor: None,
            max_drawdown: 2_000.0,
            max_drawdown_percent: 2.0,
            sharpe_ratio: Some(1.0),
            expectancy: expectancy_r * 1_000.0,
            expectancy_r,
            median_r: expectancy_r,
        }
    }

    fn elite(sequence: u64, expectancy_r: f64, trade_count: usize) -> Elite {
        let strategy = crate::generate_seed(42, sequence);
        let fingerprint = strategy
            .structural_fingerprint(FloatPolicy::default())
            .unwrap();
        Elite {
            strategy,
            structural_fingerprint: fingerprint,
            descriptor: BehaviorDescriptor {
                entry_conditions: 2,
                exit_conditions: 1,
                trades_per_1000_bars: 0.1,
                average_bars_held: 4.0,
                drawdown_percent: 2.0,
                win_rate_percent: 55.0,
                long_short_skew: 0.0,
            },
            niche: NicheKey {
                entry_conditions: 2,
                trade_frequency: crate::ThreeLevelBucket::Medium,
                hold_time: crate::ThreeLevelBucket::Medium,
                drawdown: crate::ThreeLevelBucket::Low,
                win_rate: crate::ThreeLevelBucket::Medium,
                long_short_skew: crate::LongShortSkewBucket::Balanced,
            },
            evidence: EvidenceComponents {
                return_component: 1.0,
                profit_factor_component: 1.0,
                trade_count_bonus: 1.0,
                drawdown_penalty: 0.0,
                complexity_penalty: 0.0,
                total: 3.0,
            },
            novelty: 1.0,
            complexity: 2,
            metrics: metrics(expectancy_r, trade_count),
            is_expectancy: expectancy_r * 1_000.0,
            oos1_expectancy: None,
            oos1_expectancy_ratio: None,
            fold_r: FoldRStats {
                fold_count: 3,
                median_fold_r: expectancy_r,
                fold_spread: 0.01,
                pooled_r: expectancy_r,
                max_year_share: 0.4,
                has_negative_fold: false,
                usable: true,
            },
            observed_trade_sharpe: None,
            expected_max_lucky_sharpe: None,
            deflated_trade_sharpe: None,
            multi_symbol_results: Vec::new(),
            gate_results: Vec::new(),
            robustness: None,
            equity_signature: vec![0.0, sequence as f64, sequence as f64 * 0.5],
            discovered_generation: 1,
            island_id: 0,
        }
    }

    fn bank(candidates: Vec<Elite>) -> Databank {
        Databank {
            schema_version: crate::DATABANK_SCHEMA_VERSION,
            grammar_version: crate::GRAMMAR_VERSION.into(),
            data_hash: ContentHash::sha256("development"),
            execution_data_hash: ContentHash::sha256("m1-development"),
            broker_spec_hash: ContentHash::sha256("broker"),
            config: crate::DiscoverConfig {
                deposit_gates: GateConfig {
                    minimum_trades: 20,
                    maximum_drawdown_percent: 30.0,
                    minimum_return_percent: 0.0,
                    minimum_profit_factor: 1.0,
                    minimum_recovery_factor: 0.0,
                },
                correlation_threshold: 1.0,
                max_promoted_per_niche: 0,
                max_per_entry_family: 0,
                ..Default::default()
            },
            completed_generations: 0,
            evaluation_count: 0,
            accepted_pool: Vec::new(),
            accepted_coverage_map: BTreeMap::new(),
            specialist_pool: Vec::new(),
            specialist_coverage_map: BTreeMap::new(),
            holding: candidates,
            holding_coverage_map: BTreeMap::new(),
            elites: Vec::new(),
            coverage_map: BTreeMap::new(),
            telemetry: Default::default(),
        }
    }

    fn stable_trades(r: f64) -> Vec<Trade> {
        let mut trades = Vec::new();
        for year in 2016..2022 {
            for month in 1..=12 {
                for day in [2, 7, 12, 17] {
                    trades.push(trade(timestamp(year, month, day), r));
                }
            }
        }
        trades
    }

    #[test]
    fn selects_the_stronger_stable_candidate_deterministically() {
        let candidates = vec![elite(1, 0.25, 288), elite(2, 0.40, 288)];
        let bank = bank(candidates.clone());
        let replays = candidates
            .iter()
            .map(|candidate| {
                let r = candidate.metrics.expectancy_r;
                (
                    candidate.structural_fingerprint.to_string(),
                    ProductionLaneReplay {
                        metrics: metrics(r, 288),
                        trades: stable_trades(r),
                    },
                )
            })
            .collect();
        let report = run_production_lane(
            &bank,
            &candidates,
            &replays,
            ContentHash::sha256("development"),
            timestamp(2016, 1, 1),
            timestamp(2022, 1, 1),
            ProductionLaneConfig {
                selection_fraction: 0.50,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(report.selected, 1);
        assert_eq!(report.selected_ids, vec![candidates[1].strategy.id.clone()]);
        assert!(report.rows[1].selected);
    }

    #[test]
    fn rejects_a_replay_that_crosses_the_development_boundary() {
        let candidate = elite(1, 0.30, 100);
        let bank = bank(vec![candidate.clone()]);
        let end = timestamp(2022, 1, 1);
        let mut trades = stable_trades(0.30);
        trades.push(trade(end, 10.0));
        let replays = BTreeMap::from([(
            candidate.structural_fingerprint.to_string(),
            ProductionLaneReplay {
                metrics: metrics(0.30, trades.len()),
                trades,
            },
        )]);
        let error = run_production_lane(
            &bank,
            &[candidate],
            &replays,
            ContentHash::sha256("development"),
            timestamp(2016, 1, 1),
            end,
            ProductionLaneConfig::default(),
        )
        .unwrap_err();
        assert!(error.contains("outside the Development boundary"));
    }

    #[test]
    fn six_and_twelve_month_stability_are_real_eligibility_gates() {
        let candidate = elite(1, 0.30, 288);
        let bank = bank(vec![candidate.clone()]);
        let mut trades = stable_trades(0.30);
        for trade in &mut trades {
            if DateTime::<Utc>::from_timestamp_millis(trade.entry_timestamp_ms)
                .unwrap()
                .year()
                >= 2019
            {
                trade.r_multiple = -0.30;
            }
        }
        let replays = BTreeMap::from([(
            candidate.structural_fingerprint.to_string(),
            ProductionLaneReplay {
                metrics: metrics(0.30, trades.len()),
                trades,
            },
        )]);
        let report = run_production_lane(
            &bank,
            &[candidate],
            &replays,
            ContentHash::sha256("development"),
            timestamp(2016, 1, 1),
            timestamp(2022, 1, 1),
            ProductionLaneConfig::default(),
        )
        .unwrap();
        assert_eq!(report.eligible, 0);
        assert!(
            report.rows[0]
                .rejection_reasons
                .iter()
                .any(|reason| reason.contains("12-month"))
        );
    }

    #[test]
    fn failed_replays_remain_valid_immutable_json_rows() {
        let candidates = vec![elite(9, 0.30, 200)];
        let bank = bank(candidates.clone());
        let report = run_production_lane(
            &bank,
            &candidates,
            &BTreeMap::new(),
            ContentHash::sha256("development"),
            timestamp(2016, 1, 1),
            timestamp(2022, 1, 1),
            ProductionLaneConfig::default(),
        )
        .unwrap();

        assert_eq!(report.rows[0].score, None);
        serde_json::to_vec(&report).expect("report must never contain non-finite JSON numbers");
    }
}
