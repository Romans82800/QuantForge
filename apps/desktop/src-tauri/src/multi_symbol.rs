use crate::data_lab::{display_path, load_bound_broker, load_data_source};
use crate::workflow::{ensure_new, read_json, write_json_new};
use quantforge_eval::{CostModel, ScoutConfig};
use quantforge_ir::StrategyIr;
use quantforge_quality::{MatrixSymbolInput, run_multi_symbol_matrix};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_FX_PACK: &[&str] = &[
    "AUDUSD", "EURGBP", "EURJPY", "EURNZD", "GBPJPY", "GBPUSD", "NZDUSD", "USDCHF", "USDJPY",
];

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSymbolMatrixRequest {
    strategy_path: String,
    pack_dir: String,
    #[serde(default)]
    symbols: Vec<String>,
    source_timezone: Option<String>,
    initial_balance: f64,
    commission_per_lot_round_turn: f64,
    required_pass: usize,
    minimum_net_profit: f64,
    output_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSymbolMatrixRowView {
    symbol: String,
    passed: bool,
    trade_count: usize,
    return_percent: f64,
    profit_factor: Option<f64>,
    max_drawdown_percent: f64,
    net_profit: f64,
    win_rate: f64,
    expectancy: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairwiseCorrelationView {
    left: String,
    right: String,
    correlation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiSymbolMatrixView {
    passed: bool,
    strategy_id: String,
    output_path: String,
    passing_count: usize,
    required_pass: usize,
    symbol_count: usize,
    mean_return_percent: f64,
    mean_net_profit: f64,
    maximum_pairwise_correlation: f64,
    rows: Vec<MultiSymbolMatrixRowView>,
    pairwise: Vec<PairwiseCorrelationView>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResultsPackRequest {
    input_path: String,
    title: String,
    output_directory: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResultsPackView {
    directory: String,
    html_path: String,
    trades_csv_path: String,
    metrics_json_path: String,
    pdf_path: String,
}

#[tauri::command]
pub async fn run_multi_symbol_matrix_workflow(
    request: MultiSymbolMatrixRequest,
) -> Result<MultiSymbolMatrixView, String> {
    tauri::async_runtime::spawn_blocking(move || run_matrix_sync(&request))
        .await
        .map_err(|error| format!("multi-symbol matrix failed: {error}"))?
}

#[tauri::command]
pub async fn export_results_pack_workflow(
    request: ExportResultsPackRequest,
) -> Result<ExportResultsPackView, String> {
    tauri::async_runtime::spawn_blocking(move || export_pack_sync(&request))
        .await
        .map_err(|error| format!("results pack export failed: {error}"))?
}

fn export_pack_sync(request: &ExportResultsPackRequest) -> Result<ExportResultsPackView, String> {
    let raw: serde_json::Value = read_json(PathBuf::from(&request.input_path))?;
    let out = ensure_new(&request.output_directory, "results pack directory")?;
    let paths = quantforge_quality::write_results_pack_from_json(&out, &request.title, &raw)?;
    Ok(ExportResultsPackView {
        directory: display_path(&paths.directory),
        html_path: display_path(&paths.html),
        trades_csv_path: display_path(&paths.trades_csv),
        metrics_json_path: display_path(&paths.metrics_json),
        pdf_path: display_path(&paths.pdf),
    })
}

fn run_matrix_sync(request: &MultiSymbolMatrixRequest) -> Result<MultiSymbolMatrixView, String> {
    let strategy: StrategyIr = read_json(PathBuf::from(&request.strategy_path))?;
    let pack = PathBuf::from(&request.pack_dir);
    if !pack.is_dir() {
        return Err(format!("pack directory missing: {}", pack.display()));
    }
    let symbols = if request.symbols.is_empty() {
        DEFAULT_FX_PACK.iter().map(|s| (*s).to_string()).collect()
    } else {
        request.symbols.clone()
    };
    let timezone = request
        .source_timezone
        .clone()
        .unwrap_or_else(|| "ICMarkets/EST+7".into());
    let mut markets = Vec::new();
    for symbol in &symbols {
        let data = find_h1(&pack, symbol)?;
        let broker_path = pack.join(format!("{symbol}.broker.json"));
        let loaded = load_data_source(&data.display().to_string(), None, Some(timezone.as_str()))?;
        let broker = load_bound_broker(&broker_path.display().to_string(), loaded.metadata.as_ref())?;
        markets.push(MatrixSymbolInput {
            symbol: symbol.clone(),
            dataset: loaded.dataset,
            broker,
        });
    }
    let scout = ScoutConfig {
        initial_balance: request.initial_balance,
        costs: CostModel {
            commission_per_lot_round_turn: request.commission_per_lot_round_turn,
            ..CostModel::default()
        },
        ..ScoutConfig::default()
    };
    let report = run_multi_symbol_matrix(
        &strategy,
        &markets,
        &scout,
        request.required_pass,
        request.minimum_net_profit,
    )
    .map_err(|error| error.to_string())?;
    let out = ensure_new(&request.output_path, "multi-symbol matrix artifact")?;
    write_json_new(&out, &report)?;
    Ok(MultiSymbolMatrixView {
        passed: report.matrix_passed,
        strategy_id: report.strategy_id,
        output_path: display_path(&out),
        passing_count: report.passing_count,
        required_pass: report.required_pass,
        symbol_count: report.symbols.len(),
        mean_return_percent: report.mean_return_percent,
        mean_net_profit: report.mean_net_profit,
        maximum_pairwise_correlation: report.maximum_pairwise_correlation,
        rows: report
            .symbols
            .into_iter()
            .map(|row| MultiSymbolMatrixRowView {
                symbol: row.symbol,
                passed: row.passed,
                trade_count: row.trade_count,
                return_percent: row.return_percent,
                profit_factor: row.profit_factor,
                max_drawdown_percent: row.max_drawdown_percent,
                net_profit: row.net_profit,
                win_rate: row.win_rate,
                expectancy: row.expectancy,
            })
            .collect(),
        pairwise: report
            .pairwise_correlations
            .into_iter()
            .take(12)
            .map(|pair| PairwiseCorrelationView {
                left: pair.left,
                right: pair.right,
                correlation: pair.correlation,
            })
            .collect(),
    })
}

fn find_h1(pack: &std::path::Path, symbol: &str) -> Result<PathBuf, String> {
    let exact = pack.join(format!("ICMarketsSC-Demo_{symbol}_H1_2020_present.tsv"));
    if exact.is_file() {
        return Ok(exact);
    }
    for entry in std::fs::read_dir(pack).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains(&format!("_{symbol}_H1_")) && name.ends_with(".tsv") {
            return Ok(path);
        }
    }
    Err(format!(
        "H1 TSV for {symbol} not found in {}",
        pack.display()
    ))
}
