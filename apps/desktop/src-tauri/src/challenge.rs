use crate::data_lab::{display_path, load_bound_broker, load_data_source};
use crate::workflow::{
    ChallengeArtifact, SplitPlanArtifact, ensure_new, manifest, read_json, recipe_path,
    write_json_new,
};
use quantforge_data::{DataQualityReport, QualityGrade};
use quantforge_eval::{CostModel, ScoutConfig};
use quantforge_ir::StrategyIr;
use quantforge_quality::{ChallengeConfig, DataSplitPlan, run_challenge};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    strategy_path: String,
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
    folds: usize,
    monte_carlo_trials: usize,
    neighborhood_samples: usize,
    seed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeView {
    passed: bool,
    grade: &'static str,
    split_plan_path: String,
    challenge_path: String,
    development_bars: usize,
    validation_bars: usize,
    sealed_bars: usize,
    validation_trades: usize,
    return_percent: f64,
    profit_factor: Option<f64>,
    maximum_drawdown_percent: f64,
    passing_folds: usize,
    total_folds: usize,
    passing_cost_shocks: usize,
    total_cost_shocks: usize,
    blockers: Vec<String>,
}

#[tauri::command]
pub async fn run_challenge_workflow(request: ChallengeRequest) -> Result<ChallengeView, String> {
    tauri::async_runtime::spawn_blocking(move || run_challenge_sync(&request))
        .await
        .map_err(|error| format!("Challenge task failed: {error}"))?
}

fn run_challenge_sync(request: &ChallengeRequest) -> Result<ChallengeView, String> {
    let output_directory = PathBuf::from(&request.output_directory);
    if output_directory.exists() {
        return Err(format!(
            "Challenge output directory already exists and will not be replaced: {}",
            output_directory.display()
        ));
    }
    let split_path = output_directory.join("split-plan.json");
    let challenge_path = output_directory.join("challenge.json");
    ensure_new(&split_path, "split plan")?;
    ensure_new(&challenge_path, "Challenge artifact")?;

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
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
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
        plan,
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
        },
        folds: request.folds,
        monte_carlo_trials: request.monte_carlo_trials,
        neighborhood_samples: request.neighborhood_samples,
        evaluations_touched: request.evaluations_touched,
        seed: request.seed,
        ..ChallengeConfig::default()
    };
    let report = run_challenge(
        &strategy,
        &loaded.dataset,
        &broker,
        &split_artifact.plan,
        config,
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
            ("strategy".into(), recipe_path(&request.strategy_path)),
            ("broker".into(), recipe_path(&request.broker_path)),
            ("split_plan".into(), recipe_path(&split_path)),
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
        metadata_hash: loaded.metadata.map(|metadata| metadata.metadata_hash),
        data_quality: quality,
        strategy_source: display_path(Path::new(&request.strategy_path)),
        broker_source: display_path(Path::new(&request.broker_path)),
        split_plan_source: display_path(&split_path),
        report,
    };

    std::fs::create_dir_all(&output_directory)
        .map_err(|error| format!("cannot create Challenge directory: {error}"))?;
    if let Err(error) = write_json_new(&split_path, &split_artifact)
        .and_then(|()| write_json_new(&challenge_path, &artifact))
    {
        return Err(format!(
            "Challenge output is incomplete in {}: {error}",
            output_directory.display()
        ));
    }

    let metrics = &artifact.report.baseline.metrics;
    Ok(ChallengeView {
        passed: artifact.report.passed,
        grade: if artifact.report.passed {
            "challenged"
        } else {
            "illuminated"
        },
        split_plan_path: display_path(&split_path),
        challenge_path: display_path(&challenge_path),
        development_bars: split_artifact.plan.development.bar_count,
        validation_bars: split_artifact.plan.validation.bar_count,
        sealed_bars: split_artifact.plan.sealed_final.bar_count,
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
    })
}
