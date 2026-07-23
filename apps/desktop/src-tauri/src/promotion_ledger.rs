use crate::data_lab::{display_path, load_bound_broker, load_data_source};
use crate::workflow::{
    ChallengeArtifact, IncubationFinalArtifact, IncubationObservationArtifact,
    IncubationStartArtifact, SealedFinalArtifact, SplitPlanArtifact, binding, manifest, read_json,
    read_json_hashed, recipe_path, verify_split, write_json_new,
};
use chrono::NaiveDate;
use quantforge_data::{DataQualityReport, QualityGrade};
use quantforge_ir::StrategyIr;
use quantforge_quality::{
    INCUBATION_PROTOCOL, IncubationKillRules, IncubationObservation, IncubationStart,
    SealedFinalConfig, run_incubation, run_sealed_final as evaluate_sealed_final,
};
use quantforge_storage::{claim_sealed_access_once, sealed_final_path, write_sealed_final_once};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    strategy_path: String,
    broker_path: String,
    split_plan_path: String,
    challenge_path: String,
    sealed_root: String,
    minimum_trades: usize,
    minimum_return_percent: f64,
    minimum_profit_factor: f64,
    maximum_drawdown_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SealedView {
    output_path: String,
    passed: bool,
    grade: &'static str,
    trades: usize,
    return_percent: f64,
    profit_factor: Option<f64>,
    maximum_drawdown_percent: f64,
    blockers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncubationStartRequest {
    strategy_path: String,
    broker_path: String,
    split_plan_path: String,
    root_directory: String,
    start_date: String,
    initial_balance: String,
    maximum_daily_loss_percent: String,
    maximum_total_drawdown_percent: String,
    minimum_observation_days: usize,
    minimum_total_trades: usize,
    maximum_consecutive_zero_trade_days: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncubationRecordRequest {
    start_path: String,
    date: String,
    ending_balance: f64,
    maximum_drawdown_percent: f64,
    trade_count: usize,
    note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncubationFinalizeRequest {
    start_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncubationView {
    start_path: String,
    final_path: Option<String>,
    status: &'static str,
    observation_days: usize,
    total_trades: usize,
    return_percent: Option<f64>,
    maximum_drawdown_percent: Option<f64>,
    passed: Option<bool>,
    blockers: Vec<String>,
}

#[tauri::command]
pub async fn run_sealed_final(request: SealedRequest) -> Result<SealedView, String> {
    tauri::async_runtime::spawn_blocking(move || run_sealed_final_sync(&request))
        .await
        .map_err(|error| format!("sealed-final task failed: {error}"))?
}

fn run_sealed_final_sync(request: &SealedRequest) -> Result<SealedView, String> {
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
    let split: SplitPlanArtifact = read_json(&request.split_plan_path)?;
    verify_split(&split)?;
    let (challenge, challenge_hash) =
        read_json_hashed::<ChallengeArtifact>(&request.challenge_path)?;
    challenge
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    challenge
        .report
        .validate_integrity()
        .map_err(|error| error.to_string())?;
    if challenge.manifest.command != "challenge" || !challenge.report.passed {
        return Err("sealed-final requires a passing Challenge artifact".into());
    }
    let broker = load_bound_broker(&request.broker_path, None)?;
    let candidate = binding(&strategy, &broker)?;
    let split_hash = split
        .plan
        .content_hash()
        .map_err(|error| error.to_string())?;
    if challenge.report.binding != candidate
        || challenge.report.split_plan_hash != split_hash
        || challenge.report.validation_data_hash != split.plan.validation.data_hash
    {
        return Err("strategy, broker, Challenge and split plan do not bind one candidate".into());
    }
    let mut config = SealedFinalConfig::default();
    config.scout = challenge.report.config.scout.clone();
    config.minimum_trades = request.minimum_trades;
    config.minimum_return_percent = request.minimum_return_percent;
    config.minimum_profit_factor = request.minimum_profit_factor;
    config.maximum_drawdown_percent = request.maximum_drawdown_percent;
    config
        .validate(&challenge.report)
        .map_err(|error| error.to_string())?;
    let final_path = sealed_final_path(
        &request.sealed_root,
        &candidate.strategy_fingerprint,
        &split_hash,
    );
    if final_path.exists() {
        return Err(format!(
            "sealed-final was already evaluated: {}",
            final_path.display()
        ));
    }
    let access_path = claim_sealed_access_once(
        &request.sealed_root,
        &candidate.strategy_fingerprint,
        &split_hash,
        &challenge_hash,
    )
    .map_err(|error| error.to_string())?;
    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let quality = DataQualityReport::analyze(&loaded.dataset);
    if quality.grade == QualityGrade::Fail {
        return Err(format!(
            "sealed access was claimed at {}, but data quality failed; this attempt cannot be retried",
            access_path.display()
        ));
    }
    load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    let report = evaluate_sealed_final(
        &strategy,
        &loaded.dataset,
        &broker,
        &split.plan,
        &challenge.report,
        challenge_hash,
        config,
    )
    .map_err(|error| error.to_string())?;
    let report_hash =
        quantforge_core::stable_json_hash(&report).map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "sealed-final",
        Some(report.sealed_data_hash.clone()),
        Some(report.binding.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("source".into(), recipe_path(&request.data_path)),
            ("strategy".into(), recipe_path(&request.strategy_path)),
            ("broker".into(), recipe_path(&request.broker_path)),
            ("split_plan".into(), recipe_path(&request.split_plan_path)),
            ("challenge".into(), recipe_path(&request.challenge_path)),
            ("access_claim".into(), json!(display_path(&access_path))),
            ("split_plan_hash".into(), json!(&report.split_plan_hash)),
            (
                "challenge_artifact_hash".into(),
                json!(&report.challenge_artifact_hash),
            ),
            (
                "strategy_fingerprint".into(),
                json!(&report.binding.strategy_fingerprint),
            ),
            (
                "protocol".into(),
                json!(quantforge_quality::SEALED_FINAL_PROTOCOL),
            ),
            ("report_hash".into(), json!(&report_hash)),
            (
                "sealed_config".into(),
                serde_json::to_value(&report.config).map_err(|error| error.to_string())?,
            ),
            ("report_passed".into(), json!(report.passed)),
            ("report_blockers".into(), json!(&report.blockers)),
        ]),
    )?;
    let artifact = SealedFinalArtifact {
        manifest: run_manifest,
        source: display_path(Path::new(&request.data_path)),
        metadata_hash: loaded.metadata.map(|value| value.metadata_hash),
        data_quality: quality,
        strategy_source: display_path(Path::new(&request.strategy_path)),
        broker_source: display_path(Path::new(&request.broker_path)),
        split_plan_source: display_path(Path::new(&request.split_plan_path)),
        challenge_source: display_path(Path::new(&request.challenge_path)),
        report,
    };
    let written = write_sealed_final_once(&request.sealed_root, &artifact.report, &artifact)
        .map_err(|error| error.to_string())?;
    Ok(SealedView {
        output_path: display_path(&written),
        passed: artifact.report.passed,
        grade: if artifact.report.passed {
            "challenged"
        } else {
            "illuminated"
        },
        trades: artifact.report.result.metrics.trade_count,
        return_percent: artifact.report.result.metrics.return_percent,
        profit_factor: artifact.report.result.metrics.profit_factor,
        maximum_drawdown_percent: artifact.report.result.metrics.max_drawdown_percent,
        blockers: artifact
            .report
            .blockers
            .iter()
            .map(|value| format!("{value:?}"))
            .collect(),
    })
}

#[tauri::command]
pub async fn start_incubation(request: IncubationStartRequest) -> Result<IncubationView, String> {
    tauri::async_runtime::spawn_blocking(move || start_incubation_sync(&request))
        .await
        .map_err(|error| format!("incubation start task failed: {error}"))?
}

fn start_incubation_sync(request: &IncubationStartRequest) -> Result<IncubationView, String> {
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
    let broker = load_bound_broker(&request.broker_path, None)?;
    let split: SplitPlanArtifact = read_json(&request.split_plan_path)?;
    verify_split(&split)?;
    let candidate = binding(&strategy, &broker)?;
    let start = IncubationStart {
        schema_version: quantforge_quality::INCUBATION_SCHEMA_VERSION,
        protocol_version: INCUBATION_PROTOCOL.into(),
        binding: candidate.clone(),
        split_plan_hash: split
            .plan
            .content_hash()
            .map_err(|error| error.to_string())?,
        started_on: parse_date(&request.start_date)?,
        initial_balance: parse_number("initial balance", &request.initial_balance)?,
        kill_rules: IncubationKillRules {
            maximum_daily_loss_percent: parse_number(
                "daily loss limit",
                &request.maximum_daily_loss_percent,
            )?,
            maximum_total_drawdown_percent: parse_number(
                "total drawdown limit",
                &request.maximum_total_drawdown_percent,
            )?,
            minimum_observation_days: request.minimum_observation_days,
            minimum_total_trades: request.minimum_total_trades,
            maximum_consecutive_zero_trade_days: request.maximum_consecutive_zero_trade_days,
        },
    };
    start.validate().map_err(|error| error.to_string())?;
    let path = PathBuf::from(&request.root_directory)
        .join(candidate.strategy_fingerprint.as_str())
        .join(start.split_plan_hash.as_str())
        .join("incubation-start.json");
    let run_manifest = manifest(
        "incubation-start",
        Some(split.plan.full_data_hash),
        Some(candidate.broker_spec_hash),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("strategy".into(), recipe_path(&request.strategy_path)),
            ("broker".into(), recipe_path(&request.broker_path)),
            ("split_plan".into(), recipe_path(&request.split_plan_path)),
            ("split_plan_hash".into(), json!(&start.split_plan_hash)),
            ("protocol".into(), json!(INCUBATION_PROTOCOL)),
            (
                "start".into(),
                serde_json::to_value(&start).map_err(|error| error.to_string())?,
            ),
        ]),
    )?;
    write_json_new(
        &path,
        &IncubationStartArtifact {
            manifest: run_manifest,
            strategy_source: display_path(Path::new(&request.strategy_path)),
            broker_source: display_path(Path::new(&request.broker_path)),
            split_plan_source: display_path(Path::new(&request.split_plan_path)),
            start,
        },
    )?;
    ledger_view(&path, None)
}

#[tauri::command]
pub async fn record_incubation(request: IncubationRecordRequest) -> Result<IncubationView, String> {
    tauri::async_runtime::spawn_blocking(move || record_incubation_sync(&request))
        .await
        .map_err(|error| format!("incubation record task failed: {error}"))?
}

fn record_incubation_sync(request: &IncubationRecordRequest) -> Result<IncubationView, String> {
    let start_path = PathBuf::from(&request.start_path);
    let ledger = load_ledger(&start_path)?;
    if ledger.final_path.exists() {
        return Err("incubation is already finalized".into());
    }
    let date = parse_date(&request.date)?;
    if ledger
        .observations
        .last()
        .is_some_and(|item| date <= item.observation.date)
    {
        return Err("observation date must be later than every recorded date".into());
    }
    let starting_balance = ledger
        .observations
        .last()
        .map_or(ledger.start.start.initial_balance, |item| {
            item.observation.ending_balance
        });
    let observation = IncubationObservation {
        date,
        starting_balance,
        ending_balance: request.ending_balance,
        maximum_drawdown_percent: request.maximum_drawdown_percent,
        trade_count: request.trade_count,
        note: request.note.clone(),
    };
    observation.validate().map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "incubation-record",
        ledger.start.manifest.recipe.data_hash.clone(),
        Some(ledger.start.start.binding.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("start".into(), json!(display_path(&start_path))),
            ("start_artifact_hash".into(), json!(&ledger.start_hash)),
            ("protocol".into(), json!(INCUBATION_PROTOCOL)),
            (
                "observation".into(),
                serde_json::to_value(&observation).map_err(|error| error.to_string())?,
            ),
        ]),
    )?;
    let path = start_path
        .parent()
        .ok_or_else(|| "start path has no parent".to_owned())?
        .join("observations")
        .join(format!("{}.json", observation.date));
    write_json_new(
        &path,
        &IncubationObservationArtifact {
            manifest: run_manifest,
            start_source: display_path(&start_path),
            start_artifact_hash: ledger.start_hash,
            observation,
        },
    )?;
    ledger_view(&start_path, None)
}

#[tauri::command]
pub async fn finalize_incubation(
    request: IncubationFinalizeRequest,
) -> Result<IncubationView, String> {
    tauri::async_runtime::spawn_blocking(move || finalize_incubation_sync(&request))
        .await
        .map_err(|error| format!("incubation final task failed: {error}"))?
}

fn finalize_incubation_sync(request: &IncubationFinalizeRequest) -> Result<IncubationView, String> {
    let start_path = PathBuf::from(&request.start_path);
    let ledger = load_ledger(&start_path)?;
    if ledger.final_path.exists() {
        return Err("incubation final already exists".into());
    }
    let observations: Vec<_> = ledger
        .observations
        .iter()
        .map(|value| value.observation.clone())
        .collect();
    let report = run_incubation(
        &ledger.start.start,
        &observations,
        ledger.start_hash.clone(),
        ledger.observation_hashes.clone(),
    )
    .map_err(|error| error.to_string())?;
    let report_hash =
        quantforge_core::stable_json_hash(&report).map_err(|error| error.to_string())?;
    let sources: Vec<_> = ledger
        .observation_paths
        .iter()
        .map(|value| display_path(value))
        .collect();
    let run_manifest = manifest(
        "incubation-final",
        ledger.start.manifest.recipe.data_hash.clone(),
        Some(ledger.start.start.binding.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("start".into(), json!(display_path(&start_path))),
            ("start_artifact_hash".into(), json!(&ledger.start_hash)),
            ("observation_sources".into(), json!(&sources)),
            (
                "observation_artifact_hashes".into(),
                json!(&ledger.observation_hashes),
            ),
            ("protocol".into(), json!(INCUBATION_PROTOCOL)),
            ("report_hash".into(), json!(&report_hash)),
            ("report_passed".into(), json!(report.passed)),
            ("report_blockers".into(), json!(&report.blockers)),
        ]),
    )?;
    let artifact = IncubationFinalArtifact {
        manifest: run_manifest,
        start_source: display_path(&start_path),
        observation_sources: sources,
        start: ledger.start.start,
        observations,
        report,
    };
    write_json_new(&ledger.final_path, &artifact)?;
    ledger_view(&start_path, Some(&artifact))
}

struct Ledger {
    start: IncubationStartArtifact,
    start_hash: quantforge_core::ContentHash,
    observations: Vec<IncubationObservationArtifact>,
    observation_hashes: Vec<quantforge_core::ContentHash>,
    observation_paths: Vec<PathBuf>,
    final_path: PathBuf,
}

fn load_ledger(start_path: &Path) -> Result<Ledger, String> {
    let (start, start_hash) = read_json_hashed::<IncubationStartArtifact>(start_path)?;
    start
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    start.start.validate().map_err(|error| error.to_string())?;
    if start.manifest.command != "incubation-start" {
        return Err("invalid incubation start artifact".into());
    }
    let root = start_path
        .parent()
        .ok_or_else(|| "start path has no parent".to_owned())?;
    let mut paths = Vec::new();
    let observations_dir = root.join("observations");
    if observations_dir.exists() {
        for entry in fs::read_dir(&observations_dir).map_err(|error| error.to_string())? {
            paths.push(entry.map_err(|error| error.to_string())?.path());
        }
        paths.sort();
    }
    let mut observations = Vec::new();
    let mut hashes = Vec::new();
    for path in &paths {
        let (item, hash) = read_json_hashed::<IncubationObservationArtifact>(path)?;
        if item.start_artifact_hash != start_hash {
            return Err("incubation observation is bound to another ledger".into());
        }
        item.observation
            .validate()
            .map_err(|error| error.to_string())?;
        observations.push(item);
        hashes.push(hash);
    }
    Ok(Ledger {
        start,
        start_hash,
        observations,
        observation_hashes: hashes,
        observation_paths: paths,
        final_path: root.join("incubation-final.json"),
    })
}

fn ledger_view(
    start_path: &Path,
    final_artifact: Option<&IncubationFinalArtifact>,
) -> Result<IncubationView, String> {
    let ledger = load_ledger(start_path)?;
    let existing = if let Some(value) = final_artifact {
        Some(value.clone())
    } else if ledger.final_path.exists() {
        Some(read_json(&ledger.final_path)?)
    } else {
        None
    };
    let total_trades = ledger
        .observations
        .iter()
        .map(|value| value.observation.trade_count)
        .sum();
    Ok(IncubationView {
        start_path: display_path(start_path),
        final_path: existing.as_ref().map(|_| display_path(&ledger.final_path)),
        status: if existing.is_some() {
            "finalized"
        } else {
            "open"
        },
        observation_days: ledger.observations.len(),
        total_trades,
        return_percent: existing.as_ref().map(|value| value.report.return_percent),
        maximum_drawdown_percent: existing
            .as_ref()
            .map(|value| value.report.maximum_observed_drawdown_percent),
        passed: existing.as_ref().map(|value| value.report.passed),
        blockers: existing
            .as_ref()
            .map(|value| {
                value
                    .report
                    .blockers
                    .iter()
                    .map(|item| format!("{item:?}"))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "date must use YYYY-MM-DD".into())
}
fn parse_number(label: &str, value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("{label} must be numeric"))
}
