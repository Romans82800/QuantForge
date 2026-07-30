use crate::data_lab::{display_path, load_bound_broker, load_data_source};
use crate::workflow::{
    ChallengeArtifact, SplitPlanArtifact, ensure_new, manifest, read_json, recipe_path,
    write_json_new,
};
use quantforge_data::{DataQualityReport, QualityGrade};
use quantforge_eval::{CostModel, ScoutConfig};
use quantforge_ir::StrategyIr;
use quantforge_quality::{ChallengeConfig, DataSplitPlan, run_challenge};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    /// Single-strategy path (legacy). Prefer `strategy_paths` for batches.
    #[serde(default)]
    strategy_path: String,
    /// Any number of strategy IR paths. When non-empty, runs in parallel.
    #[serde(default)]
    strategy_paths: Vec<String>,
    broker_path: String,
    output_directory: String,
    validation_fraction: f64,
    sealed_fraction: f64,
    evaluations_touched: u64,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    fallback_spread_points: Option<f64>,
    max_spread_points: Option<f64>,
    initial_balance: f64,
    /// Broker-local hour from which entries may be placed (inclusive).
    entry_window_start_hour: Option<u32>,
    /// Broker-local hour from which entries stop being placed (exclusive).
    entry_window_end_hour: Option<u32>,
    folds: usize,
    monte_carlo_trials: usize,
    neighborhood_samples: usize,
    seed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeItemView {
    strategy_path: String,
    strategy_id: String,
    passed: bool,
    grade: &'static str,
    challenge_path: String,
    validation_trades: usize,
    return_percent: f64,
    profit_factor: Option<f64>,
    maximum_drawdown_percent: f64,
    passing_folds: usize,
    total_folds: usize,
    passing_cost_shocks: usize,
    total_cost_shocks: usize,
    blockers: Vec<String>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeView {
    passed: bool,
    grade: &'static str,
    split_plan_path: String,
    challenge_path: String,
    /// Alias: IS bars.
    development_bars: usize,
    /// Alias: OOS1 bars.
    validation_bars: usize,
    /// Alias: OOS2 bars (sealed; not used by Challenge).
    sealed_bars: usize,
    is_bars: usize,
    oos1_bars: usize,
    oos2_bars: usize,
    validation_trades: usize,
    return_percent: f64,
    profit_factor: Option<f64>,
    maximum_drawdown_percent: f64,
    passing_folds: usize,
    total_folds: usize,
    passing_cost_shocks: usize,
    total_cost_shocks: usize,
    blockers: Vec<String>,
    /// Parallel batch results when more than one strategy was submitted.
    #[serde(default)]
    results: Vec<ChallengeItemView>,
    passed_count: usize,
    failed_count: usize,
    total_count: usize,
}

#[tauri::command]
pub async fn run_challenge_workflow(request: ChallengeRequest) -> Result<ChallengeView, String> {
    tauri::async_runtime::spawn_blocking(move || run_challenge_sync(&request))
        .await
        .map_err(|error| format!("Challenge task failed: {error}"))?
}

fn strategy_paths(request: &ChallengeRequest) -> Result<Vec<String>, String> {
    let mut raw = request.strategy_paths.clone();
    if !request.strategy_path.trim().is_empty()
        && !raw.iter().any(|path| path == &request.strategy_path)
    {
        raw.push(request.strategy_path.clone());
    }
    if raw.is_empty() {
        return Err("at least one strategy IR path (or batch index) is required".into());
    }

    let mut expanded = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in raw {
        for strategy_path in expand_strategy_input(&path)? {
            if seen.insert(strategy_path.clone()) {
                expanded.push(strategy_path);
            }
        }
    }
    if expanded.is_empty() {
        return Err("batch index contained no strategy paths".into());
    }
    Ok(expanded)
}

/// Accept either a Strategy IR JSON or a Databank batch index
/// (`quantforge-strategy-batch.json` with `strategies[].path`).
fn expand_strategy_input(path: &str) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{path} is not valid JSON: {error}"))?;

    if let Some(strategies) = value.get("strategies").and_then(|entry| entry.as_array()) {
        let mut paths = Vec::with_capacity(strategies.len());
        for (index, entry) in strategies.iter().enumerate() {
            let strategy_path = entry
                .get("path")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    format!("{path} strategies[{index}] is missing a string \"path\" field")
                })?;
            if !Path::new(strategy_path).is_file() {
                return Err(format!(
                    "batch entry strategies[{index}] path does not exist: {strategy_path}"
                ));
            }
            paths.push(strategy_path.to_owned());
        }
        if paths.is_empty() {
            return Err(format!("{path} is a batch index with an empty strategies list"));
        }
        return Ok(paths);
    }

    if value.get("id").is_some() {
        return Ok(vec![path.to_owned()]);
    }

    Err(format!(
        "{path} is neither a Strategy IR (missing \"id\") nor a QuantForge batch index (missing \"strategies\")"
    ))
}

fn run_challenge_sync(request: &ChallengeRequest) -> Result<ChallengeView, String> {
    let paths = strategy_paths(request)?;
    let output_directory = PathBuf::from(&request.output_directory);
    if output_directory.exists() {
        return Err(format!(
            "Challenge output directory already exists and will not be replaced: {}",
            output_directory.display()
        ));
    }
    let split_path = output_directory.join("split-plan.json");
    ensure_new(&split_path, "split plan")?;

    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let quality = DataQualityReport::analyze(&loaded.dataset);
    if quality.grade == QualityGrade::Fail {
        return Err(format!(
            "Challenge refuses failed-quality data (score {})",
            quality.score
        ));
    }
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;

    let plan = DataSplitPlan::chronological(
        &loaded.dataset,
        request.validation_fraction,
        request.sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let split_manifest = manifest(
        "split-plan",
        Some(loaded.dataset.data_hash.clone()),
        None,
        None,
        None,
        BTreeMap::from([
            ("source".into(), recipe_path(&request.data_path)),
            (
                "source_timezone".into(),
                json!(&loaded.dataset.source_timezone),
            ),
            (
                "validation_fraction".into(),
                json!(request.validation_fraction),
            ),
            ("sealed_fraction".into(), json!(request.sealed_fraction)),
            ("is_label".into(), json!("in_sample")),
            ("oos1_label".into(), json!("out_of_sample_1_pick")),
            ("oos2_label".into(), json!("out_of_sample_2_sealed")),
            ("data_quality_grade".into(), json!(quality.grade)),
            ("data_quality_score".into(), json!(quality.score)),
        ]),
    )?;
    let split_artifact = SplitPlanArtifact {
        manifest: split_manifest,
        source: display_path(Path::new(&request.data_path)),
        metadata_hash: loaded
            .metadata
            .as_ref()
            .map(|metadata| metadata.metadata_hash.clone()),
        data_quality: quality.clone(),
        validation_fraction: request.validation_fraction,
        sealed_fraction: request.sealed_fraction,
        plan: plan.clone(),
    };

    let config = ChallengeConfig {
        scout: ScoutConfig {
            initial_balance: request.initial_balance,
            same_bar_policy: quantforge_eval::SameBarPolicy::Conservative,
            costs: CostModel {
                fallback_spread_points: request.fallback_spread_points,
                adverse_slippage_points_per_side: request.slippage_points_per_side,
                commission_per_lot_round_turn: request.commission_per_lot_round_turn,
                max_spread_points: request.max_spread_points,
                include_costs_in_risk: true,
            },
            indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
            entry_window: crate::discover::entry_window(
                request.entry_window_start_hour,
                request.entry_window_end_hour,
            ),
            // Challenge reports its metrics, so it must always replay in full.
            abandon_above_drawdown_percent: None,
        },
        folds: request.folds,
        monte_carlo_trials: request.monte_carlo_trials,
        neighborhood_samples: request.neighborhood_samples,
        evaluations_touched: request.evaluations_touched,
        seed: request.seed,
        // Require non-negative deflated Sharpe proxy by default (anti-overfit).
        minimum_deflated_trade_sharpe: Some(0.0),
        ..ChallengeConfig::default()
    };

    std::fs::create_dir_all(&output_directory)
        .map_err(|error| format!("cannot create Challenge directory: {error}"))?;
    write_json_new(&split_path, &split_artifact).map_err(|error| error.to_string())?;

    let completed = AtomicUsize::new(0);
    let total = paths.len();
    let mut results: Vec<ChallengeItemView> = paths
        .par_iter()
        .enumerate()
        .map(|(index, strategy_path)| {
            let item = challenge_one(
                strategy_path,
                index,
                request,
                &loaded,
                &broker,
                &split_artifact,
                &split_path,
                &config,
                &output_directory,
            );
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            let _ = done;
            let _ = total;
            item
        })
        .collect();
    results.sort_by(|left, right| left.strategy_path.cmp(&right.strategy_path));

    let passed_count = results.iter().filter(|item| item.passed).count();
    let failed_count = results.len() - passed_count;
    let first = results
        .first()
        .cloned()
        .ok_or_else(|| "Challenge produced no results".to_owned())?;

    Ok(ChallengeView {
        passed: passed_count > 0 && failed_count == 0,
        grade: if passed_count > 0 && failed_count == 0 {
            "challenged"
        } else if passed_count > 0 {
            "partial"
        } else {
            "illuminated"
        },
        split_plan_path: display_path(&split_path),
        challenge_path: first.challenge_path.clone(),
        development_bars: split_artifact.plan.development.bar_count,
        validation_bars: split_artifact.plan.validation.bar_count,
        sealed_bars: split_artifact.plan.sealed_final.bar_count,
        is_bars: split_artifact.plan.development.bar_count,
        oos1_bars: split_artifact.plan.validation.bar_count,
        oos2_bars: split_artifact.plan.sealed_final.bar_count,
        validation_trades: first.validation_trades,
        return_percent: first.return_percent,
        profit_factor: first.profit_factor,
        maximum_drawdown_percent: first.maximum_drawdown_percent,
        passing_folds: first.passing_folds,
        total_folds: first.total_folds,
        passing_cost_shocks: first.passing_cost_shocks,
        total_cost_shocks: first.total_cost_shocks,
        blockers: first.blockers.clone(),
        results,
        passed_count,
        failed_count,
        total_count: paths.len(),
    })
}

#[allow(clippy::too_many_arguments)]
fn challenge_one(
    strategy_path: &str,
    index: usize,
    request: &ChallengeRequest,
    loaded: &crate::data_lab::LoadedDataSource,
    broker: &quantforge_broker::SymbolSpecification,
    split_artifact: &SplitPlanArtifact,
    split_path: &Path,
    config: &ChallengeConfig,
    output_directory: &Path,
) -> ChallengeItemView {
    let stem = Path::new(strategy_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("strategy");
    let safe_stem: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    let challenge_path = output_directory.join(format!("{safe_stem}-{index:04}.challenge.json"));

    let run = (|| -> Result<ChallengeItemView, String> {
        ensure_new(&challenge_path, "Challenge artifact")?;
        let strategy: StrategyIr = read_json(strategy_path)?;
        let report = run_challenge(
            &strategy,
            &loaded.dataset,
            broker,
            &split_artifact.plan,
            config.clone(),
        )
        .map_err(|error| error.to_string())?;
        let challenge_manifest = manifest(
            "challenge",
            Some(report.validation_data_hash.clone()),
            Some(report.binding.broker_spec_hash.clone()),
            Some(quantforge_discover::GRAMMAR_VERSION.into()),
            Some(report.config.seed),
            BTreeMap::from([
                ("source".into(), recipe_path(&request.data_path)),
                ("strategy".into(), recipe_path(strategy_path)),
                ("broker".into(), recipe_path(&request.broker_path)),
                ("split_plan".into(), recipe_path(split_path)),
                ("split_plan_hash".into(), json!(&report.split_plan_hash)),
                (
                    "full_data_hash".into(),
                    json!(&split_artifact.plan.full_data_hash),
                ),
                (
                    "strategy_fingerprint".into(),
                    json!(&report.binding.strategy_fingerprint),
                ),
                (
                    "protocol".into(),
                    json!(quantforge_quality::CHALLENGE_PROTOCOL),
                ),
                ("report_passed".into(), json!(report.passed)),
                ("report_blockers".into(), json!(&report.blockers)),
                (
                    "evaluations_touched".into(),
                    json!(report.config.evaluations_touched),
                ),
                (
                    "challenge_config".into(),
                    serde_json::to_value(&report.config).map_err(|error| error.to_string())?,
                ),
            ]),
        )?;
        let artifact = ChallengeArtifact {
            manifest: challenge_manifest,
            source: display_path(Path::new(&request.data_path)),
            metadata_hash: loaded
                .metadata
                .as_ref()
                .map(|metadata| metadata.metadata_hash.clone()),
            data_quality: split_artifact.data_quality.clone(),
            strategy_source: display_path(Path::new(strategy_path)),
            broker_source: display_path(Path::new(&request.broker_path)),
            split_plan_source: display_path(split_path),
            report,
        };
        write_json_new(&challenge_path, &artifact).map_err(|error| error.to_string())?;
        let metrics = &artifact.report.baseline.metrics;
        Ok(ChallengeItemView {
            strategy_path: strategy_path.into(),
            strategy_id: strategy.id,
            passed: artifact.report.passed,
            grade: if artifact.report.passed {
                "challenged"
            } else {
                "illuminated"
            },
            challenge_path: display_path(&challenge_path),
            validation_trades: metrics.trade_count,
            return_percent: metrics.return_percent,
            profit_factor: metrics.profit_factor,
            maximum_drawdown_percent: metrics.max_drawdown_percent,
            passing_folds: artifact
                .report
                .purged_folds
                .iter()
                .filter(|fold| fold.passed)
                .count(),
            total_folds: artifact.report.purged_folds.len(),
            passing_cost_shocks: artifact.report.cost_shocks.passing_points,
            total_cost_shocks: artifact.report.cost_shocks.points.len(),
            blockers: artifact
                .report
                .blockers
                .iter()
                .map(|blocker| format!("{blocker:?}"))
                .collect(),
            error: None,
        })
    })();

    match run {
        Ok(view) => view,
        Err(error) => ChallengeItemView {
            strategy_path: strategy_path.into(),
            strategy_id: Path::new(strategy_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("strategy")
                .into(),
            passed: false,
            grade: "error",
            challenge_path: display_path(&challenge_path),
            validation_trades: 0,
            return_percent: 0.0,
            profit_factor: None,
            maximum_drawdown_percent: 0.0,
            passing_folds: 0,
            total_folds: 0,
            passing_cost_shocks: 0,
            total_cost_shocks: 0,
            blockers: vec![error.clone()],
            error: Some(error),
        },
    }
}
