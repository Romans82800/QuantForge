//! In-process executor for QuantForge task graphs (`*.qf-task.json`).

use crate::databank_filter::{filter_rows, row_from_value};
use crate::negate::{NegateMode, negate_strategy};
use crate::results_html::{render_results_html_from_json, render_results_html_from_scout};
use crate::task_graph::{
    TASK_GRAPH_PROTOCOL, TaskGraph, TaskGraphError, TaskRunReport, TaskStep, TaskStepKind,
    TaskStepResult, TaskStepStatus,
};
use crate::walk_forward_matrix::{WalkForwardMatrixConfig, run_walk_forward_matrix};
use crate::what_if::{WhatIfFilter, apply_what_if};
use crate::{ChallengeConfig, DataSplitPlan, run_challenge};
use quantforge_broker::SymbolSpecification;
use quantforge_data::{BarDataset, SourceTimezone};
use quantforge_eval::{CostModel, EntryWindow, ScoutConfig, evaluate_strategy};
use quantforge_export_mql5::{ExportStyle, Mql5ExportConfig, TesterConfig, generate_bundle};
use quantforge_ir::StrategyIr;
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct TaskRunOptions {
    pub work_dir: PathBuf,
    pub dry_run: bool,
    pub stop_on_failure: bool,
}

impl Default for TaskRunOptions {
    fn default() -> Self {
        Self {
            work_dir: PathBuf::from("."),
            dry_run: false,
            stop_on_failure: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct TaskArtifactStore {
    /// step_id → artifact_name → absolute/relative path string
    pub by_step: BTreeMap<String, BTreeMap<String, String>>,
    /// Convenience: last artifact of a given name from any step.
    pub latest: BTreeMap<String, String>,
}

impl TaskArtifactStore {
    fn record(&mut self, step_id: &str, name: &str, path: impl AsRef<Path>) {
        let display = path.as_ref().display().to_string();
        self.by_step
            .entry(step_id.into())
            .or_default()
            .insert(name.into(), display.clone());
        self.latest.insert(name.into(), display);
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.latest.get(name).map(String::as_str)
    }
}

/// Execute a validated task graph end-to-end in-process.
pub fn run_task_graph(
    graph: &TaskGraph,
    options: &TaskRunOptions,
) -> Result<TaskRunReport, TaskGraphError> {
    graph.validate()?;
    fs::create_dir_all(&options.work_dir).map_err(|error| {
        TaskGraphError::StepFailed("_setup".into(), format!("work_dir: {error}"))
    })?;

    let ordered = graph.ordered_steps(true)?;
    let mut artifacts = TaskArtifactStore::default();
    let mut results = Vec::new();
    let mut passed = true;

    for step in ordered {
        if options.dry_run {
            results.push(TaskStepResult {
                id: step.id.clone(),
                kind: step.kind.clone(),
                status: TaskStepStatus::Passed,
                message: format!(
                    "dry-run planned {:?} ({} params)",
                    step.kind,
                    merged_params(graph, step).len()
                ),
                artifacts: BTreeMap::new(),
            });
            continue;
        }

        let params = merged_params(graph, step);
        let outcome = execute_step(step, &params, options, &mut artifacts);
        match outcome {
            Ok(result) => {
                if result.status == TaskStepStatus::Failed {
                    passed = false;
                    results.push(result);
                    if options.stop_on_failure {
                        break;
                    }
                } else {
                    results.push(result);
                }
            }
            Err(error) => {
                passed = false;
                results.push(TaskStepResult {
                    id: step.id.clone(),
                    kind: step.kind.clone(),
                    status: TaskStepStatus::Failed,
                    message: error.to_string(),
                    artifacts: BTreeMap::new(),
                });
                if options.stop_on_failure {
                    break;
                }
            }
        }
    }

    Ok(TaskRunReport {
        protocol: TASK_GRAPH_PROTOCOL.into(),
        graph_name: graph.name.clone(),
        passed,
        steps: results,
    })
}

fn merged_params(graph: &TaskGraph, step: &TaskStep) -> BTreeMap<String, Value> {
    let mut params = graph.inputs.clone();
    for (key, value) in &step.params {
        params.insert(key.clone(), value.clone());
    }
    params
}

fn execute_step(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    match step.kind {
        TaskStepKind::Note => Ok(TaskStepResult {
            id: step.id.clone(),
            kind: step.kind.clone(),
            status: TaskStepStatus::Passed,
            message: param_string(params, "text").unwrap_or_else(|| "note".into()),
            artifacts: BTreeMap::new(),
        }),
        TaskStepKind::Scout => exec_scout(step, params, options, artifacts),
        TaskStepKind::Challenge => exec_challenge(step, params, options, artifacts),
        TaskStepKind::WalkForwardMatrix => exec_wf_matrix(step, params, options, artifacts),
        TaskStepKind::Judge => exec_judge(step, params, options, artifacts),
        TaskStepKind::ExportMql5 => exec_export(step, params, options, artifacts),
        TaskStepKind::DatabankFilter => exec_filter(step, params, options, artifacts),
        TaskStepKind::WhatIf => exec_what_if(step, params, options, artifacts),
        TaskStepKind::Negate => exec_negate(step, params, options, artifacts),
        TaskStepKind::HtmlReport => exec_html(step, params, options, artifacts),
        TaskStepKind::MultiSymbolRetest => exec_multi_symbol(step, params, options, artifacts),
    }
}

fn exec_scout(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let broker = load_broker(params)?;
    let dataset = load_dataset(params, "data", "metadata", "source_timezone")?;
    let scout = scout_config(params);
    let result = evaluate_strategy(&strategy, &dataset, &broker, &scout)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-scout.json", step.id));
    write_json(
        &out,
        &json!({
            "kind": "scout",
            "strategy_id": strategy.id,
            "result": result,
        }),
    )?;
    artifacts.record(&step.id, "scout", &out);
    artifacts.record(&step.id, "trades_source", &out);
    Ok(ok_result(
        step,
        format!(
            "scout {} trades · return {:.2}%",
            result.metrics.trade_count, result.metrics.return_percent
        ),
        [("scout", out)],
    ))
}

fn exec_challenge(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let broker = load_broker(params)?;
    let dataset = load_dataset(params, "data", "metadata", "source_timezone")?;
    let plan = load_or_create_split(params, options, &dataset)?;
    let mut config = ChallengeConfig {
        scout: scout_config(params),
        ..ChallengeConfig::default()
    };
    if let Some(folds) = param_usize(params, "folds") {
        config.folds = folds;
    }
    if let Some(value) = param_usize(params, "monte_carlo_trials") {
        config.monte_carlo_trials = value;
    }
    if let Some(value) = param_usize(params, "neighborhood_samples") {
        config.neighborhood_samples = value;
    }
    if let Some(value) = param_usize(params, "minimum_validation_bars") {
        config.minimum_validation_bars = value;
    }
    if let Some(value) = param_usize(params, "minimum_baseline_trades") {
        config.minimum_baseline_trades = value;
    }
    if let Some(value) = param_usize(params, "minimum_fold_trades") {
        config.minimum_fold_trades = value;
    }
    if let Some(value) = param_f64(params, "minimum_return_percent") {
        config.minimum_return_percent = value;
    }
    if let Some(value) = param_f64(params, "minimum_profit_factor") {
        config.minimum_profit_factor = value;
    }
    if let Some(value) = param_f64(params, "maximum_drawdown_percent") {
        config.maximum_drawdown_percent = value;
    }
    if let Some(value) = param_f64(params, "minimum_passing_fold_fraction") {
        config.minimum_passing_fold_fraction = value;
    }
    if let Some(value) = param_f64(params, "monte_carlo_minimum_p05_net_profit") {
        config.monte_carlo_minimum_p05_net_profit = value;
    }
    if let Some(value) = param_f64(params, "minimum_neighborhood_survival_fraction") {
        config.minimum_neighborhood_survival_fraction = value;
    }
    if let Some(value) = param_u64(params, "evaluations_touched") {
        config.evaluations_touched = value.max(1);
    }
    if let Some(value) = param_u64(params, "seed") {
        config.seed = value;
    }
    let report = run_challenge(&strategy, &dataset, &broker, &plan, config)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-challenge.json", step.id));
    write_json(&out, &report)?;
    artifacts.record(&step.id, "challenge", &out);
    let status = if report.passed {
        TaskStepStatus::Passed
    } else {
        TaskStepStatus::Failed
    };
    Ok(TaskStepResult {
        id: step.id.clone(),
        kind: step.kind.clone(),
        status,
        message: if report.passed {
            "challenge passed".into()
        } else {
            format!("challenge blocked: {:?}", report.blockers)
        },
        artifacts: BTreeMap::from([("challenge".into(), out.display().to_string())]),
    })
}

fn exec_wf_matrix(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let broker = load_broker(params)?;
    let dataset = load_dataset(params, "data", "metadata", "source_timezone")?;
    let mut config = WalkForwardMatrixConfig {
        initial_balance: param_f64(params, "initial_balance").unwrap_or(100_000.0),
        costs: cost_model(params),
        entry_window: entry_window(params),
        ..WalkForwardMatrixConfig::default()
    };
    if let Some(Value::Array(folds)) = params.get("fold_counts") {
        config.fold_counts = folds
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as usize))
            .collect();
    }
    if let Some(Value::Array(lookbacks)) = params.get("lookback_bars") {
        config.lookback_bars = lookbacks
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as usize))
            .collect();
    }
    if let Some(value) = param_usize(params, "minimum_fold_trades") {
        config.minimum_fold_trades = value;
    }
    if let Some(value) = param_f64(params, "minimum_return_percent") {
        config.minimum_return_percent = value;
    }
    if let Some(value) = param_f64(params, "minimum_profit_factor") {
        config.minimum_profit_factor = value;
    }
    if let Some(value) = param_f64(params, "maximum_drawdown_percent") {
        config.maximum_drawdown_percent = value;
    }
    if let Some(value) = param_f64(params, "minimum_passing_fold_fraction") {
        config.minimum_passing_fold_fraction = value;
    }
    let report = run_walk_forward_matrix(&strategy, &dataset, &broker, &config)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-wf-matrix.json", step.id));
    write_json(&out, &report)?;
    artifacts.record(&step.id, "wf_matrix", &out);
    Ok(ok_result(
        step,
        format!(
            "wf-matrix {}/{} cells pass",
            report.passing_cells,
            report.cells.len()
        ),
        [("wf_matrix", out)],
    ))
}

fn exec_judge(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let broker = load_broker(params)?;
    let decision = load_dataset(params, "data", "metadata", "source_timezone")?;
    let m1 = load_dataset(params, "m1", "m1_metadata", "m1_source_timezone").or_else(|_| {
        // Fall back to decision bars when callers intentionally share one series.
        load_dataset(params, "data", "metadata", "source_timezone")
    })?;
    let config = JudgeConfig {
        initial_balance: param_f64(params, "initial_balance").unwrap_or(100_000.0),
        costs: cost_model(params),
        allow_execution_gaps: param_bool(params, "allow_execution_gaps").unwrap_or(false),
        indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
        entry_window: entry_window(params),
    };
    let result = evaluate_strategy_m1(&strategy, &decision, &m1, &broker, &config)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-judge.json", step.id));
    write_json(
        &out,
        &json!({
            "kind": "judge",
            "strategy_id": strategy.id,
            "result": result,
        }),
    )?;
    artifacts.record(&step.id, "judge", &out);
    artifacts.record(&step.id, "trades_source", &out);
    Ok(ok_result(
        step,
        format!(
            "judge {} trades · return {:.2}%",
            result.metrics.trade_count, result.metrics.return_percent
        ),
        [("judge", out)],
    ))
}

fn exec_export(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let broker = load_broker(params)?;
    let expert_name = param_string(params, "expert_name")
        .unwrap_or_else(|| format!("QF_{}", sanitize_id(&strategy.id)));
    let out_dir = options.work_dir.join(format!("{}-export", step.id));
    fs::create_dir_all(&out_dir).map_err(|error| {
        TaskGraphError::StepFailed(step.id.clone(), format!("export dir: {error}"))
    })?;
    let config = Mql5ExportConfig {
        expert_name: expert_name.clone(),
        expert_directory: param_string(params, "expert_directory").unwrap_or_else(|| "QuantForge".into()),
        timeframe: param_string(params, "timeframe").unwrap_or_else(|| "H1".into()),
        magic: param_u64(params, "magic").unwrap_or(42_424_242),
        deviation_points: param_u64(params, "deviation_points").unwrap_or(10) as u32,
        max_spread_points: param_f64(params, "max_spread_points"),
        estimated_slippage_points_per_side: param_f64(params, "slippage_points_per_side")
            .unwrap_or(0.0),
        commission_per_lot_round_turn: param_f64(params, "commission_per_lot_round_turn")
            .unwrap_or(7.0),
        allow_live_trading_default: false,
        export_style: ExportStyle::Sqx,
        entry_window_start_hour: param_u64(params, "entry_window_start_hour").unwrap_or(2) as u32,
        entry_window_end_hour: param_u64(params, "entry_window_end_hour").unwrap_or(19) as u32,
        tester: TesterConfig {
            from_date: None,
            to_date: None,
            deposit: param_f64(params, "initial_balance").unwrap_or(100_000.0),
            currency: param_string(params, "currency").unwrap_or_else(|| "USD".into()),
            leverage: param_u64(params, "leverage").unwrap_or(100) as u32,
            model: param_u64(params, "tester_model").unwrap_or(1) as u8,
        },
    };
    let bundle = generate_bundle(&strategy, &broker, &config)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let source_path = out_dir.join(format!("{expert_name}.mq5"));
    fs::write(&source_path, &bundle.source).map_err(|error| {
        TaskGraphError::StepFailed(step.id.clone(), format!("write mq5: {error}"))
    })?;
    fs::write(out_dir.join(format!("{expert_name}.set")), &bundle.set_file).map_err(|error| {
        TaskGraphError::StepFailed(step.id.clone(), format!("write set: {error}"))
    })?;
    fs::write(
        out_dir.join(format!("{expert_name}.tester.ini")),
        &bundle.tester_ini,
    )
    .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), format!("write tester: {error}")))?;
    write_json(out_dir.join(format!("{expert_name}.evidence.json")), &bundle.evidence)?;
    artifacts.record(&step.id, "export_dir", &out_dir);
    artifacts.record(&step.id, "mq5", &source_path);
    Ok(ok_result(
        step,
        format!("exported {expert_name}.mq5"),
        [("export_dir", out_dir), ("mq5", source_path)],
    ))
}

fn exec_filter(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let expr = param_string(params, "expr")
        .ok_or_else(|| TaskGraphError::StepFailed(step.id.clone(), "missing expr".into()))?;
    let elites_path = param_path(params, "elites")
        .or_else(|| artifacts.get("elites").map(PathBuf::from))
        .ok_or_else(|| TaskGraphError::StepFailed(step.id.clone(), "missing elites".into()))?;
    let raw: Value = read_json_value(&elites_path)?;
    let list = if let Some(arr) = raw.as_array() {
        arr.clone()
    } else if let Some(arr) = raw.get("elites").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        return Err(TaskGraphError::StepFailed(
            step.id.clone(),
            "elites must be an array or {elites:[...]}".into(),
        ));
    };
    let mut rows = Vec::new();
    for item in &list {
        rows.push(
            row_from_value(item)
                .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?,
        );
    }
    let report = filter_rows(&expr, &rows)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-filter.json", step.id));
    write_json(&out, &report)?;
    artifacts.record(&step.id, "filter", &out);
    Ok(ok_result(
        step,
        format!("filter matched {} / {}", report.matched, report.total),
        [("filter", out)],
    ))
}

fn exec_what_if(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let trades_path = param_path(params, "trades")
        .or_else(|| artifacts.get("trades_source").map(PathBuf::from))
        .or_else(|| artifacts.get("scout").map(PathBuf::from))
        .ok_or_else(|| TaskGraphError::StepFailed(step.id.clone(), "missing trades".into()))?;
    let raw: Value = read_json_value(&trades_path)?;
    let trades: Vec<quantforge_eval::Trade> = if let Some(arr) = raw.as_array() {
        serde_json::from_value(Value::Array(arr.clone()))
            .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?
    } else if let Some(result) = raw.get("result") {
        serde_json::from_value(result.get("trades").cloned().unwrap_or(json!([])))
            .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?
    } else {
        serde_json::from_value(raw.get("trades").cloned().unwrap_or(json!([])))
            .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?
    };
    let mut filters = Vec::new();
    if let Some(percent) = param_f64(params, "exclude_pct_biggest_pl") {
        filters.push(WhatIfFilter::ExcludePctBiggestPl { percent });
    }
    if let Some(percent) = param_f64(params, "exclude_pct_lowest_pl") {
        filters.push(WhatIfFilter::ExcludePctLowestPl { percent });
    }
    if param_bool(params, "exclude_short_trades").unwrap_or(false) {
        filters.push(WhatIfFilter::ExcludeShortTrades);
    }
    if param_bool(params, "exclude_long_trades").unwrap_or(false) {
        filters.push(WhatIfFilter::ExcludeLongTrades);
    }
    if let Some(n) = param_usize(params, "take_every_nth_trade") {
        filters.push(WhatIfFilter::TakeEveryNthTrade { n });
    }
    if let Some(max) = param_usize(params, "take_max_trades_per_day") {
        filters.push(WhatIfFilter::TakeMaxTradesPerDay { max });
    }
    if filters.is_empty() {
        filters.push(WhatIfFilter::ExcludePctBiggestPl { percent: 5.0 });
    }
    let report = apply_what_if(
        &trades,
        param_f64(params, "initial_balance").unwrap_or(100_000.0),
        &filters,
    )
    .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-what-if.json", step.id));
    write_json(&out, &report)?;
    artifacts.record(&step.id, "what_if", &out);
    Ok(ok_result(
        step,
        format!(
            "what-if kept {} / {} trades",
            report.filtered_trade_count, report.original_trade_count
        ),
        [("what_if", out)],
    ))
}

fn exec_negate(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let report = negate_strategy(&strategy, NegateMode::FlipSides)
        .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
    let out = options.work_dir.join(format!("{}-negate.json", step.id));
    write_json(&out, &report)?;
    let strategy_out = options.work_dir.join(format!("{}-negated.ir.json", step.id));
    write_json(&strategy_out, &report.strategy)?;
    artifacts.record(&step.id, "negate", &out);
    artifacts.record(&step.id, "strategy", &strategy_out);
    Ok(ok_result(
        step,
        "negated strategy sides",
        [("negate", out), ("strategy", strategy_out)],
    ))
}

fn exec_html(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let source = param_path(params, "source")
        .or_else(|| artifacts.get("scout").map(PathBuf::from))
        .or_else(|| artifacts.get("judge").map(PathBuf::from))
        .or_else(|| artifacts.get("trades_source").map(PathBuf::from))
        .ok_or_else(|| {
            TaskGraphError::StepFailed(step.id.clone(), "missing source for html report".into())
        })?;
    let raw: Value = read_json_value(&source)?;
    let title = param_string(params, "title").unwrap_or_else(|| "QuantForge results".into());
    let html = if let Ok(scout) = serde_json::from_value::<quantforge_eval::ScoutResult>(
        raw.get("result").cloned().unwrap_or(raw.clone()),
    ) {
        let strategy_id = raw
            .get("strategy_id")
            .and_then(|v| v.as_str())
            .unwrap_or("strategy");
        render_results_html_from_scout(&title, strategy_id, &scout, &[])
    } else {
        render_results_html_from_json(&title, &raw)
            .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error))?
    };
    let out = options.work_dir.join(format!("{}-report.html", step.id));
    fs::write(&out, html).map_err(|error| {
        TaskGraphError::StepFailed(step.id.clone(), format!("write html: {error}"))
    })?;
    artifacts.record(&step.id, "html_report", &out);
    Ok(ok_result(step, "wrote HTML report", [("html_report", out)]))
}

fn exec_multi_symbol(
    step: &TaskStep,
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    artifacts: &mut TaskArtifactStore,
) -> Result<TaskStepResult, TaskGraphError> {
    let strategy = load_strategy(params, artifacts)?;
    let broker = load_broker(params)?;
    let scout = scout_config(params);
    let Some(Value::Array(symbols)) = params.get("symbols") else {
        return Err(TaskGraphError::StepFailed(
            step.id.clone(),
            "multi_symbol_retest requires params.symbols array".into(),
        ));
    };
    let mut rows = Vec::new();
    for entry in symbols {
        let map = entry.as_object().ok_or_else(|| {
            TaskGraphError::StepFailed(step.id.clone(), "symbol entry must be object".into())
        })?;
        let label = map
            .get("id")
            .or_else(|| map.get("symbol"))
            .and_then(|v| v.as_str())
            .unwrap_or("symbol")
            .to_string();
        let data = map
            .get("data")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                TaskGraphError::StepFailed(step.id.clone(), format!("{label}: missing data"))
            })?;
        let mut local = params.clone();
        local.insert("data".into(), Value::String(data.into()));
        if let Some(metadata) = map.get("metadata") {
            local.insert("metadata".into(), metadata.clone());
        }
        if let Some(tz) = map.get("source_timezone") {
            local.insert("source_timezone".into(), tz.clone());
        }
        let dataset = load_dataset(&local, "data", "metadata", "source_timezone")?;
        let result = evaluate_strategy(&strategy, &dataset, &broker, &scout)
            .map_err(|error| TaskGraphError::StepFailed(step.id.clone(), error.to_string()))?;
        rows.push(json!({
            "id": label,
            "trades": result.metrics.trade_count,
            "return_percent": result.metrics.return_percent,
            "profit_factor": result.metrics.profit_factor,
            "max_drawdown_percent": result.metrics.max_drawdown_percent,
            "net_profit": result.metrics.net_profit,
        }));
    }
    let out = options
        .work_dir
        .join(format!("{}-multi-symbol.json", step.id));
    write_json(
        &out,
        &json!({
            "kind": "multi_symbol_retest",
            "strategy_id": strategy.id,
            "symbols": rows,
        }),
    )?;
    artifacts.record(&step.id, "multi_symbol", &out);
    Ok(ok_result(
        step,
        format!("retested {} symbols", symbols.len()),
        [("multi_symbol", out)],
    ))
}

fn ok_result(
    step: &TaskStep,
    message: impl Into<String>,
    arts: impl IntoIterator<Item = (&'static str, PathBuf)>,
) -> TaskStepResult {
    let mut artifacts = BTreeMap::new();
    for (name, path) in arts {
        artifacts.insert(name.into(), path.display().to_string());
    }
    TaskStepResult {
        id: step.id.clone(),
        kind: step.kind.clone(),
        status: TaskStepStatus::Passed,
        message: message.into(),
        artifacts,
    }
}

fn load_strategy(
    params: &BTreeMap<String, Value>,
    artifacts: &TaskArtifactStore,
) -> Result<StrategyIr, TaskGraphError> {
    let path = param_path(params, "strategy")
        .or_else(|| artifacts.get("strategy").map(PathBuf::from))
        .ok_or_else(|| TaskGraphError::StepFailed("_inputs".into(), "missing strategy".into()))?;
    read_json_value(&path).and_then(|value| {
        serde_json::from_value(value)
            .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))
    })
}

fn load_broker(params: &BTreeMap<String, Value>) -> Result<SymbolSpecification, TaskGraphError> {
    let path = param_path(params, "broker")
        .ok_or_else(|| TaskGraphError::StepFailed("_inputs".into(), "missing broker".into()))?;
    read_json_value(&path).and_then(|value| {
        serde_json::from_value(value)
            .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))
    })
}

fn load_dataset(
    params: &BTreeMap<String, Value>,
    data_key: &str,
    metadata_key: &str,
    tz_key: &str,
) -> Result<BarDataset, TaskGraphError> {
    let path = param_path(params, data_key)
        .ok_or_else(|| TaskGraphError::StepFailed("_inputs".into(), format!("missing {data_key}")))?;
    let timezone = if let Some(metadata_path) = param_path(params, metadata_key) {
        let metadata = quantforge_data::Mt5ExportMetadata::load(&metadata_path)
            .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))?;
        metadata
            .source_timezone()
            .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))?
    } else if let Some(tz) = param_string(params, tz_key) {
        tz.parse::<SourceTimezone>()
            .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))?
    } else {
        "Etc/UTC"
            .parse::<SourceTimezone>()
            .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))?
    };
    BarDataset::load_mt5(&path, timezone)
        .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))
}

fn load_or_create_split(
    params: &BTreeMap<String, Value>,
    options: &TaskRunOptions,
    dataset: &BarDataset,
) -> Result<DataSplitPlan, TaskGraphError> {
    if let Some(path) = param_path(params, "split_plan") {
        let value = read_json_value(&path)?;
        if let Ok(plan) = serde_json::from_value::<DataSplitPlan>(value.clone()) {
            return Ok(plan);
        }
        if let Some(plan) = value.get("plan") {
            return serde_json::from_value(plan.clone())
                .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()));
        }
        return Err(TaskGraphError::StepFailed(
            "_inputs".into(),
            "split_plan JSON is not a DataSplitPlan".into(),
        ));
    }
    let validation = param_f64(params, "validation_fraction").unwrap_or(0.2);
    let sealed = param_f64(params, "sealed_fraction").unwrap_or(0.2);
    let plan = DataSplitPlan::chronological(dataset, validation, sealed)
        .map_err(|error| TaskGraphError::StepFailed("_inputs".into(), error.to_string()))?;
    let out = options.work_dir.join("auto-split-plan.json");
    write_json(&out, &plan)?;
    Ok(plan)
}

fn scout_config(params: &BTreeMap<String, Value>) -> ScoutConfig {
    ScoutConfig {
        initial_balance: param_f64(params, "initial_balance").unwrap_or(100_000.0),
        costs: cost_model(params),
        entry_window: entry_window(params),
        ..ScoutConfig::default()
    }
}

fn cost_model(params: &BTreeMap<String, Value>) -> CostModel {
    CostModel {
        commission_per_lot_round_turn: param_f64(params, "commission_per_lot_round_turn")
            .unwrap_or(0.0),
        adverse_slippage_points_per_side: param_f64(params, "slippage_points_per_side")
            .unwrap_or(0.0),
        fallback_spread_points: param_f64(params, "fallback_spread_points"),
        max_spread_points: param_f64(params, "max_spread_points"),
        include_costs_in_risk: true,
        ..CostModel::default()
    }
}

fn entry_window(params: &BTreeMap<String, Value>) -> EntryWindow {
    EntryWindow::new(
        param_u64(params, "entry_window_start_hour").unwrap_or(2) as u32,
        param_u64(params, "entry_window_end_hour").unwrap_or(19) as u32,
    )
}

fn param_string(params: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    params.get(key).and_then(|v| v.as_str().map(str::to_string))
}

fn param_path(params: &BTreeMap<String, Value>, key: &str) -> Option<PathBuf> {
    param_string(params, key).map(PathBuf::from)
}

fn param_f64(params: &BTreeMap<String, Value>, key: &str) -> Option<f64> {
    params.get(key).and_then(|v| v.as_f64())
}

fn param_usize(params: &BTreeMap<String, Value>, key: &str) -> Option<usize> {
    params
        .get(key)
        .and_then(|v| v.as_u64().map(|n| n as usize))
}

fn param_u64(params: &BTreeMap<String, Value>, key: &str) -> Option<u64> {
    params.get(key).and_then(|v| v.as_u64())
}

fn param_bool(params: &BTreeMap<String, Value>, key: &str) -> Option<bool> {
    params.get(key).and_then(|v| v.as_bool())
}

fn read_json_value(path: &Path) -> Result<Value, TaskGraphError> {
    let text = fs::read_to_string(path).map_err(|error| {
        TaskGraphError::StepFailed("_io".into(), format!("{}: {error}", path.display()))
    })?;
    serde_json::from_str(&text).map_err(|error| {
        TaskGraphError::StepFailed("_io".into(), format!("{}: {error}", path.display()))
    })
}

fn write_json(path: impl AsRef<Path>, value: &impl serde::Serialize) -> Result<(), TaskGraphError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            TaskGraphError::StepFailed("_io".into(), format!("{}: {error}", path.display()))
        })?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| TaskGraphError::StepFailed("_io".into(), error.to_string()))?;
    fs::write(path, text).map_err(|error| {
        TaskGraphError::StepFailed("_io".into(), format!("{}: {error}", path.display()))
    })
}

fn sanitize_id(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_graph::TaskStep;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_ir::{
        BoolExpr, ComparisonOp, EntrySignals, NumericExpr, PriceField, ProtectiveStops, RiskPolicy,
        Side, StopLossPolicy, StrategyIr, StrategyMeta, TakeProfitPolicy,
    };
    use tempfile::tempdir;

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
            id: "task-exec".into(),
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

    fn write_mt5_csv(path: &Path, bars: usize) {
        let mut text = String::from(
            "<DATE>\t<TIME>\t<OPEN>\t<HIGH>\t<LOW>\t<CLOSE>\t<TICKVOL>\t<VOL>\t<SPREAD>\n",
        );
        for i in 0..bars {
            let day = 1 + (i / 24);
            let hour = i % 24;
            let up = i % 3 != 0;
            let _ = std::fmt::Write::write_fmt(
                &mut text,
                format_args!(
                    "2024.01.{day:02}\t{hour:02}:00:00\t100\t{}\t{}\t{}\t1\t0\t0\n",
                    if up { 105 } else { 101 },
                    if up { 99 } else { 97 },
                    if up { 104 } else { 98 },
                ),
            );
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn executes_scout_what_if_html_export_graph() {
        let dir = tempdir().unwrap();
        let data = dir.path().join("data.tsv");
        let broker_path = dir.path().join("broker.json");
        let strategy_path = dir.path().join("strategy.json");
        write_mt5_csv(&data, 48);
        fs::write(&broker_path, serde_json::to_string_pretty(&broker()).unwrap()).unwrap();
        fs::write(
            &strategy_path,
            serde_json::to_string_pretty(&strategy()).unwrap(),
        )
        .unwrap();
        let work = dir.path().join("work");
        let graph = TaskGraph {
            protocol: TASK_GRAPH_PROTOCOL.into(),
            schema_version: 1,
            name: "exec-test".into(),
            description: String::new(),
            inputs: BTreeMap::from([
                ("strategy".into(), json!(strategy_path.to_string_lossy())),
                ("broker".into(), json!(broker_path.to_string_lossy())),
                ("data".into(), json!(data.to_string_lossy())),
                ("source_timezone".into(), json!("Etc/UTC")),
                ("initial_balance".into(), json!(10_000.0)),
                ("entry_window_start_hour".into(), json!(0)),
                ("entry_window_end_hour".into(), json!(24)),
            ]),
            steps: vec![
                TaskStep {
                    id: "scout".into(),
                    kind: TaskStepKind::Scout,
                    depends_on: vec![],
                    params: BTreeMap::new(),
                    enabled: Some(true),
                },
                TaskStep {
                    id: "what_if".into(),
                    kind: TaskStepKind::WhatIf,
                    depends_on: vec!["scout".into()],
                    params: BTreeMap::from([("exclude_pct_biggest_pl".into(), json!(10.0))],),
                    enabled: Some(true),
                },
                TaskStep {
                    id: "html".into(),
                    kind: TaskStepKind::HtmlReport,
                    depends_on: vec!["scout".into()],
                    params: BTreeMap::new(),
                    enabled: Some(true),
                },
                TaskStep {
                    id: "export".into(),
                    kind: TaskStepKind::ExportMql5,
                    depends_on: vec!["scout".into()],
                    params: BTreeMap::new(),
                    enabled: Some(true),
                },
                TaskStep {
                    id: "wf".into(),
                    kind: TaskStepKind::WalkForwardMatrix,
                    depends_on: vec!["scout".into()],
                    params: BTreeMap::from([
                        ("fold_counts".into(), json!([2, 3])),
                        ("lookback_bars".into(), json!([2, 4])),
                        ("minimum_fold_trades".into(), json!(0)),
                        ("minimum_return_percent".into(), json!(-999.0)),
                        ("minimum_profit_factor".into(), json!(0.0)),
                        ("maximum_drawdown_percent".into(), json!(100.0)),
                        ("minimum_passing_fold_fraction".into(), json!(0.0)),
                    ]),
                    enabled: Some(true),
                },
            ],
        };
        let report = run_task_graph(
            &graph,
            &TaskRunOptions {
                work_dir: work.clone(),
                dry_run: false,
                stop_on_failure: true,
            },
        )
        .unwrap();
        assert!(report.passed, "{report:?}");
        assert_eq!(report.steps.len(), 5);
        assert!(report.steps.iter().any(|s| {
            s.artifacts
                .get("html_report")
                .is_some_and(|path| PathBuf::from(path).exists())
        }));
        assert!(report.steps.iter().any(|s| s.artifacts.contains_key("scout")));
        assert!(report.steps.iter().any(|s| s.artifacts.contains_key("mq5")));
        assert!(report
            .steps
            .iter()
            .any(|s| s.artifacts.contains_key("wf_matrix")));
    }
}
