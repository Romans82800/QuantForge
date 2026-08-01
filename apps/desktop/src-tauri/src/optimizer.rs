//! Optimizer + Walk-Forward matrix desktop task shells.

use crate::data_lab::{display_path, load_bound_broker, load_data_source};
use crate::workflow::{ensure_new, write_json_new};
use quantforge_core::FloatPolicy;
use quantforge_eval::{CostModel, EntryWindow, ScoutConfig, evaluate_strategy};
use quantforge_ir::StrategyIr;
use quantforge_quality::{
    WalkForwardMatrixConfig, perturb_strategy_parameters, run_walk_forward_matrix,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardMatrixRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    strategy_path: String,
    broker_path: String,
    output_path: String,
    fold_counts: Vec<usize>,
    lookback_bars: Vec<usize>,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    initial_balance: f64,
    entry_window_start_hour: Option<u32>,
    entry_window_end_hour: Option<u32>,
    minimum_fold_trades: usize,
    minimum_return_percent: f64,
    minimum_profit_factor: f64,
    maximum_drawdown_percent: f64,
    minimum_passing_fold_fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardMatrixCellView {
    fold_count: usize,
    lookback_bars: usize,
    total_folds: usize,
    passing_folds: usize,
    passing_fraction: f64,
    passed: bool,
    mean_return_percent: f64,
    mean_profit_factor: f64,
    mean_max_drawdown_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WalkForwardMatrixView {
    output_path: String,
    protocol: String,
    fold_counts: Vec<usize>,
    lookback_bars: Vec<usize>,
    cells: Vec<WalkForwardMatrixCellView>,
    best_cell_index: Option<usize>,
    passing_cells: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    strategy_path: String,
    broker_path: String,
    output_path: String,
    neighborhood_samples: usize,
    perturbation_fraction: f64,
    seed: u64,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    initial_balance: f64,
    entry_window_start_hour: Option<u32>,
    entry_window_end_hour: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerNeighborView {
    sample: usize,
    strategy_fingerprint: String,
    return_percent: f64,
    profit_factor: Option<f64>,
    maximum_drawdown_percent: f64,
    trade_count: usize,
    return_ratio: Option<f64>,
    drawdown_ratio: f64,
    passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerView {
    output_path: String,
    baseline_return_percent: f64,
    baseline_profit_factor: Option<f64>,
    baseline_drawdown_percent: f64,
    baseline_trades: usize,
    neighbors: Vec<OptimizerNeighborView>,
    passed_count: usize,
    total_count: usize,
    survival_fraction: f64,
}

#[tauri::command]
pub async fn run_walk_forward_matrix_workflow(
    request: WalkForwardMatrixRequest,
) -> Result<WalkForwardMatrixView, String> {
    tauri::async_runtime::spawn_blocking(move || run_walk_forward_matrix_sync(&request))
        .await
        .map_err(|error| format!("Walk-forward matrix task failed: {error}"))?
}

#[tauri::command]
pub async fn run_optimizer_neighborhood(
    request: OptimizerRequest,
) -> Result<OptimizerView, String> {
    tauri::async_runtime::spawn_blocking(move || run_optimizer_sync(&request))
        .await
        .map_err(|error| format!("Optimizer task failed: {error}"))?
}

fn entry_window(start: Option<u32>, end: Option<u32>) -> Result<EntryWindow, String> {
    Ok(EntryWindow::new(start.unwrap_or(2), end.unwrap_or(19)))
}

fn run_walk_forward_matrix_sync(
    request: &WalkForwardMatrixRequest,
) -> Result<WalkForwardMatrixView, String> {
    let output = PathBuf::from(&request.output_path);
    ensure_new(&output, "walk-forward matrix artifact")?;
    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    let strategy: StrategyIr = serde_json::from_str(
        &std::fs::read_to_string(&request.strategy_path)
            .map_err(|error| format!("strategy read failed: {error}"))?,
    )
    .map_err(|error| format!("strategy parse failed: {error}"))?;

    let config = WalkForwardMatrixConfig {
        fold_counts: if request.fold_counts.is_empty() {
            vec![3, 4, 5, 6]
        } else {
            request.fold_counts.clone()
        },
        lookback_bars: if request.lookback_bars.is_empty() {
            vec![60, 120, 240]
        } else {
            request.lookback_bars.clone()
        },
        initial_balance: request.initial_balance,
        costs: CostModel {
            commission_per_lot_round_turn: request.commission_per_lot_round_turn,
            adverse_slippage_points_per_side: request.slippage_points_per_side,
            ..CostModel::default()
        },
        entry_window: entry_window(
            request.entry_window_start_hour,
            request.entry_window_end_hour,
        )?,
        minimum_fold_trades: request.minimum_fold_trades,
        minimum_return_percent: request.minimum_return_percent,
        minimum_profit_factor: request.minimum_profit_factor,
        maximum_drawdown_percent: request.maximum_drawdown_percent,
        minimum_passing_fold_fraction: request.minimum_passing_fold_fraction,
    };

    let report = run_walk_forward_matrix(&strategy, &loaded.dataset, &broker, &config)
        .map_err(|error| error.to_string())?;
    write_json_new(
        &output,
        &json!({
            "kind": "walk_forward_matrix",
            "report": report,
            "inputs": {
                "data": display_path(Path::new(&request.data_path)),
                "strategy": display_path(Path::new(&request.strategy_path)),
                "broker": display_path(Path::new(&request.broker_path)),
            }
        }),
    )?;

    Ok(WalkForwardMatrixView {
        output_path: display_path(&output),
        protocol: report.protocol.clone(),
        fold_counts: report.fold_counts.clone(),
        lookback_bars: report.lookback_bars.clone(),
        cells: report
            .cells
            .iter()
            .map(|cell| WalkForwardMatrixCellView {
                fold_count: cell.fold_count,
                lookback_bars: cell.lookback_bars,
                total_folds: cell.total_folds,
                passing_folds: cell.passing_folds,
                passing_fraction: cell.passing_fraction,
                passed: cell.passed,
                mean_return_percent: cell.mean_return_percent,
                mean_profit_factor: cell.mean_profit_factor,
                mean_max_drawdown_percent: cell.mean_max_drawdown_percent,
            })
            .collect(),
        best_cell_index: report.best_cell_index,
        passing_cells: report.passing_cells,
    })
}

fn run_optimizer_sync(request: &OptimizerRequest) -> Result<OptimizerView, String> {
    let output = PathBuf::from(&request.output_path);
    ensure_new(&output, "optimizer artifact")?;
    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    let strategy: StrategyIr = serde_json::from_str(
        &std::fs::read_to_string(&request.strategy_path)
            .map_err(|error| format!("strategy read failed: {error}"))?,
    )
    .map_err(|error| format!("strategy parse failed: {error}"))?;

    let scout = ScoutConfig {
        initial_balance: request.initial_balance,
        costs: CostModel {
            commission_per_lot_round_turn: request.commission_per_lot_round_turn,
            adverse_slippage_points_per_side: request.slippage_points_per_side,
            ..CostModel::default()
        },
        entry_window: entry_window(
            request.entry_window_start_hour,
            request.entry_window_end_hour,
        )?,
        ..ScoutConfig::default()
    };
    let baseline = evaluate_strategy(&strategy, &loaded.dataset, &broker, &scout)
        .map_err(|error| error.to_string())?;

    let samples = request.neighborhood_samples.max(1);
    let mut neighbors = Vec::with_capacity(samples);
    for sample in 0..samples {
        let neighbor = perturb_strategy_parameters(
            &strategy,
            request.perturbation_fraction,
            sample,
            request.seed,
        )
        .map_err(|error| error.to_string())?;
        let fingerprint = neighbor
            .structural_fingerprint(FloatPolicy::default())
            .map_err(|error| error.to_string())?;
        let result = evaluate_strategy(&neighbor, &loaded.dataset, &broker, &scout)
            .map_err(|error| error.to_string())?;
        let return_ratio = (baseline.metrics.return_percent > 0.0)
            .then_some(result.metrics.return_percent / baseline.metrics.return_percent);
        let drawdown_denominator = baseline.metrics.max_drawdown_percent.max(1.0e-9);
        let drawdown_ratio = result.metrics.max_drawdown_percent / drawdown_denominator;
        let trade_count_ratio = if baseline.metrics.trade_count == 0 {
            0.0
        } else {
            result.metrics.trade_count as f64 / baseline.metrics.trade_count as f64
        };
        let passed = result.metrics.return_percent > 0.0
            && return_ratio.is_none_or(|ratio| ratio >= 0.7)
            && drawdown_ratio <= 1.5
            && trade_count_ratio >= 0.5;
        neighbors.push(OptimizerNeighborView {
            sample,
            strategy_fingerprint: fingerprint.to_string(),
            return_percent: result.metrics.return_percent,
            profit_factor: result.metrics.profit_factor,
            maximum_drawdown_percent: result.metrics.max_drawdown_percent,
            trade_count: result.metrics.trade_count,
            return_ratio,
            drawdown_ratio,
            passed,
        });
    }

    let passed_count = neighbors.iter().filter(|row| row.passed).count();
    let total_count = neighbors.len();
    let survival_fraction = if total_count == 0 {
        0.0
    } else {
        passed_count as f64 / total_count as f64
    };

    write_json_new(
        &output,
        &json!({
            "kind": "optimizer_neighborhood",
            "baseline": {
                "return_percent": baseline.metrics.return_percent,
                "profit_factor": baseline.metrics.profit_factor,
                "max_drawdown_percent": baseline.metrics.max_drawdown_percent,
                "trade_count": baseline.metrics.trade_count,
            },
            "neighbors": neighbors,
            "survival_fraction": survival_fraction,
            "inputs": {
                "data": display_path(Path::new(&request.data_path)),
                "strategy": display_path(Path::new(&request.strategy_path)),
                "broker": display_path(Path::new(&request.broker_path)),
            }
        }),
    )?;

    Ok(OptimizerView {
        output_path: display_path(&output),
        baseline_return_percent: baseline.metrics.return_percent,
        baseline_profit_factor: baseline.metrics.profit_factor,
        baseline_drawdown_percent: baseline.metrics.max_drawdown_percent,
        baseline_trades: baseline.metrics.trade_count,
        neighbors,
        passed_count,
        total_count,
        survival_fraction,
    })
}
