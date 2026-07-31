//! SQX-style walk-forward matrix: grid of fold-count × lookback cells.

use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::{BacktestMetrics, CostModel, EntryWindow, ScoutConfig, evaluate_strategy_from};
use quantforge_ir::StrategyIr;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WALK_FORWARD_MATRIX_PROTOCOL: &str = "walk-forward-matrix-v1";

#[derive(Debug, Error)]
pub enum WalkForwardMatrixError {
    #[error("fold counts must be non-empty and each >= 2")]
    InvalidFoldCounts,
    #[error("lookback bars must be non-empty")]
    InvalidLookbacks,
    #[error("dataset too short for matrix (need at least {needed} bars, have {have})")]
    DatasetTooShort { needed: usize, have: usize },
    #[error(transparent)]
    Eval(#[from] quantforge_eval::EvalError),
    #[error(transparent)]
    Ir(#[from] quantforge_ir::IrError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardMatrixConfig {
    pub fold_counts: Vec<usize>,
    pub lookback_bars: Vec<usize>,
    pub initial_balance: f64,
    pub costs: CostModel,
    pub entry_window: EntryWindow,
    pub minimum_fold_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub minimum_passing_fold_fraction: f64,
}

impl Default for WalkForwardMatrixConfig {
    fn default() -> Self {
        Self {
            fold_counts: vec![3, 4, 5, 6],
            lookback_bars: vec![60, 120, 240],
            initial_balance: 100_000.0,
            costs: CostModel::default(),
            entry_window: EntryWindow::default(),
            minimum_fold_trades: 3,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 35.0,
            minimum_passing_fold_fraction: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardMatrixFold {
    pub fold: usize,
    pub start_timestamp_ms: i64,
    pub end_timestamp_ms: i64,
    pub decision_bars: usize,
    pub trades_in_fold: usize,
    pub metrics: BacktestMetrics,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardMatrixCell {
    pub fold_count: usize,
    pub lookback_bars: usize,
    pub total_folds: usize,
    pub passing_folds: usize,
    pub passing_fraction: f64,
    pub passed: bool,
    pub mean_return_percent: f64,
    pub mean_profit_factor: f64,
    pub mean_max_drawdown_percent: f64,
    pub folds: Vec<WalkForwardMatrixFold>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardMatrixReport {
    pub protocol: String,
    pub fold_counts: Vec<usize>,
    pub lookback_bars: Vec<usize>,
    pub cells: Vec<WalkForwardMatrixCell>,
    pub best_cell_index: Option<usize>,
    pub passing_cells: usize,
}

/// Evaluate a grid of contiguous OOS fold counts × lookback warmups on Scout.
pub fn run_walk_forward_matrix(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &WalkForwardMatrixConfig,
) -> Result<WalkForwardMatrixReport, WalkForwardMatrixError> {
    if config.fold_counts.is_empty() || config.fold_counts.iter().any(|n| *n < 2) {
        return Err(WalkForwardMatrixError::InvalidFoldCounts);
    }
    if config.lookback_bars.is_empty() {
        return Err(WalkForwardMatrixError::InvalidLookbacks);
    }
    let max_folds = *config.fold_counts.iter().max().unwrap_or(&2);
    let needed = max_folds * 4;
    if dataset.bars.len() < needed {
        return Err(WalkForwardMatrixError::DatasetTooShort {
            needed,
            have: dataset.bars.len(),
        });
    }

    let strategy = strategy.canonicalized(quantforge_core::FloatPolicy::default())?;
    let scout = ScoutConfig {
        initial_balance: config.initial_balance,
        costs: config.costs.clone(),
        entry_window: config.entry_window,
        ..ScoutConfig::default()
    };

    let mut cells = Vec::new();
    for &fold_count in &config.fold_counts {
        for &lookback in &config.lookback_bars {
            cells.push(evaluate_cell(
                &strategy,
                dataset,
                broker,
                &scout,
                config,
                fold_count,
                lookback,
            )?);
        }
    }

    let passing_cells = cells.iter().filter(|cell| cell.passed).count();
    let best_cell_index = cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| cell.passed)
        .max_by(|(_, a), (_, b)| {
            a.passing_fraction
                .partial_cmp(&b.passing_fraction)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    a.mean_return_percent
                        .partial_cmp(&b.mean_return_percent)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|(index, _)| index);

    Ok(WalkForwardMatrixReport {
        protocol: WALK_FORWARD_MATRIX_PROTOCOL.into(),
        fold_counts: config.fold_counts.clone(),
        lookback_bars: config.lookback_bars.clone(),
        cells,
        best_cell_index,
        passing_cells,
    })
}

fn evaluate_cell(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    scout: &ScoutConfig,
    config: &WalkForwardMatrixConfig,
    fold_count: usize,
    lookback: usize,
) -> Result<WalkForwardMatrixCell, WalkForwardMatrixError> {
    let ranges = contiguous_fold_ranges(dataset.bars.len(), fold_count);
    let mut folds = Vec::with_capacity(ranges.len());
    let mut passing = 0usize;
    let mut sum_ret = 0.0;
    let mut sum_pf = 0.0;
    let mut sum_dd = 0.0;

    for (index, (start, end)) in ranges.iter().enumerate() {
        if *end <= *start + 1 {
            continue;
        }
        let slice_start = start.saturating_sub(lookback);
        let slice = slice_dataset(dataset, slice_start, *end);
        let start_ms = dataset.bars[*start].timestamp_ms;
        let end_ms = dataset.bars[*end - 1].timestamp_ms;
        let result = evaluate_strategy_from(strategy, &slice, broker, scout, start_ms)?;
        let trades_in_fold = result
            .trades
            .iter()
            .filter(|trade| {
                trade.entry_timestamp_ms >= start_ms && trade.entry_timestamp_ms <= end_ms
            })
            .count();
        let pf = effective_pf(&result.metrics);
        let passed = trades_in_fold >= config.minimum_fold_trades
            && result.metrics.return_percent > config.minimum_return_percent
            && pf >= config.minimum_profit_factor
            && result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent;
        if passed {
            passing += 1;
        }
        sum_ret += result.metrics.return_percent;
        sum_pf += pf;
        sum_dd += result.metrics.max_drawdown_percent;
        folds.push(WalkForwardMatrixFold {
            fold: index,
            start_timestamp_ms: start_ms,
            end_timestamp_ms: end_ms,
            decision_bars: end.saturating_sub(*start),
            trades_in_fold,
            metrics: result.metrics.clone(),
            passed,
        });
    }

    let total = folds.len().max(1);
    let fraction = passing as f64 / total as f64;
    let n = folds.len().max(1) as f64;
    Ok(WalkForwardMatrixCell {
        fold_count,
        lookback_bars: lookback,
        total_folds: folds.len(),
        passing_folds: passing,
        passing_fraction: fraction,
        passed: fraction + 1e-12 >= config.minimum_passing_fold_fraction,
        mean_return_percent: sum_ret / n,
        mean_profit_factor: sum_pf / n,
        mean_max_drawdown_percent: sum_dd / n,
        folds,
    })
}

fn contiguous_fold_ranges(bar_count: usize, folds: usize) -> Vec<(usize, usize)> {
    (0..folds)
        .map(|fold| {
            let start = bar_count * fold / folds;
            let end = bar_count * (fold + 1) / folds;
            (start, end)
        })
        .collect()
}

fn slice_dataset(source: &BarDataset, start: usize, end: usize) -> BarDataset {
    let bars = source.bars[start..end].to_vec();
    BarDataset {
        data_hash: source.data_hash.clone(),
        source_rows: bars.len(),
        bars,
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: source.delimiter,
        source_timezone: source.source_timezone.clone(),
    }
}

fn effective_pf(metrics: &BacktestMetrics) -> f64 {
    metrics.profit_factor.unwrap_or(f64::INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_core::ContentHash;
    use quantforge_data::Bar;
    use quantforge_ir::{
        BoolExpr, ComparisonOp, EntrySignals, NumericExpr, PriceField, ProtectiveStops, RiskPolicy,
        Side, StopLossPolicy, StrategyIr, StrategyMeta, TakeProfitPolicy,
    };

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Fixture".into(),
            symbol: "TEST".into(),
            digits: 2,
            point: 1.0,
            tick_size: 1.0,
            tick_value: 1.0,
            contract_size: 1.0,
            volume_min: 1.0,
            volume_step: 1.0,
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

    fn strategy() -> StrategyIr {
        StrategyIr {
            id: "wf-matrix".into(),
            version: 1,
            entry: EntrySignals {
                long: Some(BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Price {
                        field: PriceField::Close,
                        shift: 1,
                    },
                    right: NumericExpr::Constant { value: 0.0 },
                }),
                short: None,
                order: Default::default(),
            },
            exit: None,
            exit_long: None,
            exit_short: None,
            filters: vec![],
            side: Side::LongOnly,
            risk: RiskPolicy::FixedLots { lots: 1.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::FixedPoints { points: 2.0 },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
            },
            manage: Default::default(),
            meta: StrategyMeta {
                thesis_hint: "test".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    fn dataset(bars: usize) -> BarDataset {
        let base = 1_704_067_200_000i64;
        let rows: Vec<_> = (0..bars)
            .map(|i| {
                let t = base + i as i64 * 3_600_000;
                let up = i % 3 != 0;
                Bar {
                    timestamp_ms: t,
                    open: 100.0,
                    high: if up { 105.0 } else { 101.0 },
                    low: if up { 99.0 } else { 97.0 },
                    close: if up { 104.0 } else { 98.0 },
                    tick_volume: 1,
                    real_volume: 0,
                    spread_points: Some(0),
                }
            })
            .collect();
        BarDataset {
            data_hash: ContentHash::sha256(b"wf-matrix-fixture"),
            source_rows: rows.len(),
            bars: rows,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        }
    }

    #[test]
    fn matrix_emits_fold_by_lookback_grid() {
        let config = WalkForwardMatrixConfig {
            fold_counts: vec![2, 3],
            lookback_bars: vec![2, 4],
            minimum_fold_trades: 0,
            minimum_return_percent: -999.0,
            minimum_profit_factor: 0.0,
            maximum_drawdown_percent: 100.0,
            minimum_passing_fold_fraction: 0.0,
            ..WalkForwardMatrixConfig::default()
        };
        let report = run_walk_forward_matrix(&strategy(), &dataset(24), &broker(), &config).unwrap();
        assert_eq!(report.cells.len(), 4);
        assert_eq!(report.protocol, WALK_FORWARD_MATRIX_PROTOCOL);
        assert!(report.cells.iter().all(|cell| cell.total_folds >= 2));
    }
}
