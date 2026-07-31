use quantforge_core::{ContentHash, FloatPolicy};
use quantforge_data::{BarDataset, DataQualityReport, QualityGrade, bar_content_hash};
use quantforge_discover::{
    Databank, DiscoverConfig, Elite, LongShortSkewBucket, NicheKey, RobustnessConfig,
    RobustnessEvidence, RobustnessReject, ThreeLevelBucket, niche_label,
    run_m1_predeposit_robustness,
};
use crate::data_lab::{build_decision_from_m1, load_bound_broker};
use quantforge_eval::evaluate_strategy;
use quantforge_export_mql5::{generate_bundle, Mql5ExportConfig, TesterConfig};
use quantforge_quality::DataSplitPlan;
use quantforge_storage::{RunManifest, RunRecipe, write_json_new, write_text_new};
use quantforge_ir::{BoolExpr, RiskPolicy, StrategyIr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tauri::State;
use thiserror::Error;

/// Entry-condition counts the grammar can emit; the first MAP-Elites axis.
const ENTRY_CONDITION_COUNTS: [usize; 3] = [2, 3, 4];
const TOTAL_NICHES: usize = ENTRY_CONDITION_COUNTS.len() * 3usize.pow(5);
const LEGACY_TOTAL_NICHES: usize = 10 * 3usize.pow(5);
const SELECTION_BIAS_WARNING_THRESHOLD: u64 = 1_500;
const LEGACY_DATABANK_SCHEMA_VERSION: u16 = 5;
const LEGACY_GRAMMAR_VERSION: &str = "search-families-v5-selected-tf-parity";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EvolveArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) source: String,
    pub(crate) broker: String,
    pub(crate) metadata_hash: Option<ContentHash>,
    pub(crate) data_quality: DataQualityReport,
    pub(crate) coverage: usize,
    pub(crate) qd_score: f64,
    pub(crate) databank: Databank,
}

#[derive(Debug)]
struct LoadedDatabank {
    bank: Databank,
    legacy_read_only: bool,
    databank_path: String,
    source: String,
    broker: String,
    metadata_path: Option<String>,
    m1_source: Option<String>,
    m1_metadata_path: Option<String>,
    validation_fraction: f64,
    sealed_fraction: f64,
}

#[derive(Debug, Clone)]
struct RobustnessSnapshot {
    elite: Elite,
    databank_path: String,
    source: String,
    broker: String,
    metadata_path: Option<String>,
    m1_source: String,
    m1_metadata_path: Option<String>,
    validation_fraction: f64,
    sealed_fraction: f64,
    data_hash: ContentHash,
    broker_spec_hash: ContentHash,
    grammar_version: String,
    config: DiscoverConfig,
}

#[derive(Default)]
pub struct DesktopState {
    loaded: RwLock<Option<LoadedDatabank>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabankWorkspace {
    source_path: String,
    data_path: String,
    metadata_path: Option<String>,
    m1_data_path: Option<String>,
    m1_metadata_path: Option<String>,
    broker_path: String,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    initial_balance: f64,
    artifact_hash: String,
    run_id: String,
    created_at: String,
    data_hash: String,
    broker_spec_hash: String,
    grammar_version: String,
    legacy_read_only: bool,
    quality_grade: String,
    quality_score: u8,
    coverage: usize,
    total_niches: usize,
    qd_score: f64,
    completed_generations: u64,
    selection_bias: SelectionBiasView,
    rejections: RejectionTelemetry,
    research_grade: bool,
    require_m1_precision: bool,
    m1_fidelity_verified: bool,
    simple_exits: bool,
    allow_break_even: bool,
    allow_trailing_stops: bool,
    allow_partial_exits: bool,
    allow_market_entries: bool,
    allow_stop_entries: bool,
    allow_limit_entries: bool,
    allow_stop_limit_entries: bool,
    max_one_entry_per_day: bool,
    validation_fraction: f64,
    sealed_fraction: f64,
    condition_groups: Vec<ConditionCoverage>,
    elites: Vec<EliteRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExportView {
    directory: String,
    index_path: String,
    strategy_paths: Vec<String>,
}

/// A self-contained set of MQL5 experts produced directly from a databank
/// selection.  This is deliberately separate from the individual parity
/// workflow: these are research exports, with live trading disabled, not a
/// deployment certification.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEaExportRequest {
    fingerprints: Vec<String>,
    directory: String,
    timeframe: String,
    base_magic: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchEaExportView {
    directory: String,
    index_path: String,
    expert_paths: Vec<String>,
    settings_paths: Vec<String>,
    tester_paths: Vec<String>,
    evidence_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectionBiasView {
    evaluation_count: u64,
    level: &'static str,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct RejectionTelemetry {
    gate: u64,
    #[serde(default)]
    deposit_gate: u64,
    clone: u64,
    correlated: u64,
    niche_not_improved: u64,
    precision: u64,
    #[serde(default)]
    ambiguous: u64,
    oos1: u64,
    evaluation: u64,
    total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConditionCoverage {
    entry_conditions: usize,
    label: String,
    occupied: usize,
    total: usize,
    cells: Vec<CoverageCell>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CoverageCell {
    index: usize,
    niche: String,
    occupied: bool,
    fingerprint: Option<String>,
    intensity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct EliteRow {
    fingerprint: String,
    strategy_id: String,
    entry_conditions: usize,
    exit_conditions: usize,
    evidence: f64,
    novelty: f64,
    trades: usize,
    return_percent: f64,
    drawdown_percent: f64,
    /// MT5-style recovery factor (net profit / absolute equity DD).
    recovery_factor: Option<f64>,
    profit_factor: Option<f64>,
    sharpe_ratio: Option<f64>,
    is_expectancy: f64,
    oos1_expectancy: Option<f64>,
    oos1_expectancy_ratio: Option<f64>,
    complexity: usize,
    generation: u64,
    grade: &'static str,
    parity: &'static str,
    equity_signature: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EliteRobustnessView {
    #[serde(skip_serializing_if = "Option::is_none")]
    monte_carlo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    walk_forward: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    param_permutation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
    /// Serialized `quantforge_discover::RobustnessEvidence`: walk-forward fold
    /// rows, Monte Carlo percentiles and parameter-neighborhood survival as
    /// recorded when the M1 battery ran. Absent for elites deposited before the
    /// evidence field existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EliteDetail {
    fingerprint: String,
    strategy_id: String,
    thesis: String,
    entry_conditions: usize,
    exit_conditions: usize,
    niche: String,
    grade: &'static str,
    parity: &'static str,
    evidence: Value,
    descriptor: Value,
    metrics: Value,
    strategy_ir: Value,
    equity_signature: Vec<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    robustness: Option<EliteRobustnessView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionEquityPoint {
    timestamp_ms: i64,
    equity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartitionEquityView {
    fingerprint: String,
    strategy_id: String,
    execution_engine: String,
    initial_balance: f64,
    points: Vec<PartitionEquityPoint>,
    is_end_timestamp_ms: i64,
    oos1_end_timestamp_ms: i64,
    oos2_end_timestamp_ms: i64,
    is_bars: usize,
    oos1_bars: usize,
    oos2_bars: usize,
    is_expectancy: f64,
    oos1_expectancy: f64,
    oos1_expectancy_ratio: Option<f64>,
    oos2_expectancy: f64,
    is_return_percent: f64,
    oos1_return_percent: f64,
    oos2_return_percent: f64,
    is_trades: usize,
    oos1_trades: usize,
    oos2_trades: usize,
    /// Full-run M1 replay metrics — same decision bars and judge config as Parity Lab.
    full_run_trades: usize,
    full_run_return_percent: f64,
    full_run_net_profit: f64,
    full_run_max_drawdown: f64,
    full_run_max_drawdown_percent: f64,
    full_run_profit_factor: Option<f64>,
    full_run_win_rate: f64,
    full_run_sharpe_ratio: Option<f64>,
    full_run_recovery_factor: Option<f64>,
    trades: Vec<TradeRowView>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultsRobustnessMode {
    Standard,
    Deep,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultsRobustnessRequest {
    fingerprint: String,
    mode: ResultsRobustnessMode,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultsRobustnessView {
    fingerprint: String,
    strategy_id: String,
    mode: ResultsRobustnessMode,
    passed: bool,
    blocker: Option<String>,
    message: String,
    artifact_path: String,
    folds: usize,
    monte_carlo_trials: usize,
    neighborhood_samples: usize,
    evidence: Option<RobustnessEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ResultsRobustnessArtifact {
    schema_version: u16,
    manifest: RunManifest,
    databank_source: String,
    decision_source: String,
    m1_source: String,
    broker_source: String,
    strategy_id: String,
    strategy_fingerprint: ContentHash,
    mode: ResultsRobustnessMode,
    validation_fraction: f64,
    sealed_fraction: f64,
    selected_timeframe_metrics: quantforge_eval::BacktestMetrics,
    passed: bool,
    blocker: Option<String>,
    evidence: Option<RobustnessEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TradeRowView {
    side: String,
    entry_timestamp_ms: i64,
    exit_timestamp_ms: i64,
    entry_price: f64,
    exit_price: f64,
    net_profit: f64,
    exit_reason: String,
}

#[derive(Debug, Error)]
pub(crate) enum DesktopError {
    #[error("cannot read databank: {0}")]
    Io(#[from] std::io::Error),
    #[error("databank JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("databank artifact is invalid: {0}")]
    InvalidArtifact(String),
    #[error("no databank is loaded")]
    NoDatabank,
    #[error("elite {0} is not present in the loaded databank")]
    MissingElite(String),
    #[error("batch export is invalid: {0}")]
    InvalidExport(String),
    #[error("desktop databank state is unavailable")]
    StateUnavailable,
}

#[tauri::command]
pub fn load_databank(
    path: String,
    state: State<'_, DesktopState>,
) -> Result<DatabankWorkspace, String> {
    load_databank_path(Path::new(&path), &state).map_err(|error| error.to_string())
}

fn load_databank_path(
    path: &Path,
    state: &DesktopState,
) -> Result<DatabankWorkspace, DesktopError> {
    let bytes = fs::read(path)?;
    let artifact_hash = ContentHash::sha256(&bytes);
    let (artifact, legacy_read_only) = parse_evolve_artifact(&bytes)?;
    let source_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let workspace = workspace_view(&artifact, &source_path, &artifact_hash);
    let metadata_path = companion_metadata_path(&artifact.source);
    let m1_source = manifest_path(&artifact, "m1_source");
    let m1_metadata_path = m1_source
        .as_deref()
        .and_then(companion_metadata_path);
    let validation_fraction = manifest_fraction(&artifact, "validation_fraction", 0.2);
    let sealed_fraction = manifest_fraction(&artifact, "sealed_fraction", 0.2);
    *state
        .loaded
        .write()
        .map_err(|_| DesktopError::StateUnavailable)? = Some(LoadedDatabank {
        bank: artifact.databank,
        legacy_read_only,
        databank_path: source_path.display().to_string(),
        source: artifact.source,
        broker: artifact.broker,
        metadata_path,
        m1_source,
        m1_metadata_path,
        validation_fraction,
        sealed_fraction,
    });
    Ok(workspace)
}

/// Open current archives with the full v6 verifier. Schema-v5 archives use a
/// frozen, read-only adapter: their original family grammar, raw config,
/// coverage identities and fingerprints remain validated, while the three
/// condition-count fields needed by the current Results UI are derived in
/// memory from each stored strategy. The source file is never rewritten or
/// relabelled as v6.
fn parse_evolve_artifact(bytes: &[u8]) -> Result<(EvolveArtifact, bool), DesktopError> {
    let mut raw: Value = serde_json::from_slice(bytes)?;
    let schema_version = raw
        .pointer("/databank/schema_version")
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok());
    let grammar_version = raw
        .pointer("/databank/grammar_version")
        .and_then(Value::as_str);
    let legacy = schema_version == Some(LEGACY_DATABANK_SCHEMA_VERSION)
        && grammar_version == Some(LEGACY_GRAMMAR_VERSION);

    if !legacy {
        let artifact: EvolveArtifact = serde_json::from_value(raw)?;
        verify_artifact(&artifact)?;
        return Ok((artifact, false));
    }

    verify_legacy_raw_bindings(&raw)?;
    adapt_legacy_archive_axes(&mut raw)?;
    let mut artifact: EvolveArtifact = serde_json::from_value(raw)?;
    verify_legacy_artifact(&artifact)?;

    // Current coverage cards key on condition count rather than v5 family.
    // This is a display-only index over the already-verified elite identities.
    artifact.databank.coverage_map = artifact
        .databank
        .elites
        .iter()
        .map(|elite| {
            (
                niche_label(&elite.niche),
                elite.structural_fingerprint.clone(),
            )
        })
        .collect();
    Ok((artifact, true))
}

fn verify_legacy_raw_bindings(raw: &Value) -> Result<(), DesktopError> {
    let manifest_config = raw.pointer("/manifest/recipe/config/discover_config");
    let databank_config = raw.pointer("/databank/config");
    if manifest_config.is_none() || manifest_config != databank_config {
        return Err(DesktopError::InvalidArtifact(
            "legacy manifest and databank configs do not match".into(),
        ));
    }

    verify_legacy_raw_coverage(raw, "elites", "coverage_map", true)?;
    verify_legacy_raw_coverage(
        raw,
        "accepted_pool",
        "accepted_coverage_map",
        false,
    )
}

fn verify_legacy_raw_coverage(
    raw: &Value,
    entries_key: &str,
    coverage_key: &str,
    niche_keyed: bool,
) -> Result<(), DesktopError> {
    let entries = raw
        .pointer(&format!("/databank/{entries_key}"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            DesktopError::InvalidArtifact(format!(
                "legacy databank is missing {entries_key}"
            ))
        })?;
    let coverage = raw
        .pointer(&format!("/databank/{coverage_key}"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            DesktopError::InvalidArtifact(format!(
                "legacy databank is missing {coverage_key}"
            ))
        })?;
    if entries.len() != coverage.len() {
        return Err(DesktopError::InvalidArtifact(format!(
            "legacy {entries_key} and {coverage_key} sizes differ"
        )));
    }

    let mut fingerprints = BTreeSet::new();
    for entry in entries {
        let fingerprint = entry
            .get("structural_fingerprint")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                DesktopError::InvalidArtifact(format!(
                    "legacy {entries_key} contains an entry without a fingerprint"
                ))
            })?;
        if !fingerprints.insert(fingerprint) {
            return Err(DesktopError::InvalidArtifact(format!(
                "legacy {entries_key} contains a duplicate fingerprint"
            )));
        }
        let key = if niche_keyed {
            legacy_niche_label(
                entry.get("niche").ok_or_else(|| {
                    DesktopError::InvalidArtifact(format!(
                        "legacy {entries_key} contains an entry without a niche"
                    ))
                })?,
            )?
        } else {
            fingerprint.to_owned()
        };
        if coverage.get(&key).and_then(Value::as_str) != Some(fingerprint) {
            return Err(DesktopError::InvalidArtifact(format!(
                "legacy {coverage_key} does not bind {fingerprint}"
            )));
        }
    }
    Ok(())
}

fn legacy_niche_label(niche: &Value) -> Result<String, DesktopError> {
    let field = |name: &str| {
        niche
            .get(name)
            .and_then(Value::as_str)
            .map(|value| value.replace('_', ""))
            .ok_or_else(|| {
                DesktopError::InvalidArtifact(format!(
                    "legacy niche is missing {name}"
                ))
            })
    };
    Ok([
        field("family")?,
        field("trade_frequency")?,
        field("hold_time")?,
        field("drawdown")?,
        field("win_rate")?,
        field("long_short_skew")?,
    ]
    .join("/"))
}

fn adapt_legacy_archive_axes(raw: &mut Value) -> Result<(), DesktopError> {
    for entries_key in ["accepted_pool", "elites"] {
        let entries = raw
            .pointer_mut(&format!("/databank/{entries_key}"))
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                DesktopError::InvalidArtifact(format!(
                    "legacy databank is missing {entries_key}"
                ))
            })?;
        for entry in entries {
            let strategy: StrategyIr = serde_json::from_value(
                entry
                    .get("strategy")
                    .cloned()
                    .ok_or_else(|| {
                        DesktopError::InvalidArtifact(format!(
                            "legacy {entries_key} contains an entry without a strategy"
                        ))
                    })?,
            )?;
            let entry_conditions = strategy_entry_condition_count(&strategy);
            let exit_conditions = strategy_exit_condition_count(&strategy);
            let descriptor = entry
                .get_mut("descriptor")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    DesktopError::InvalidArtifact(format!(
                        "legacy {entries_key} contains an invalid descriptor"
                    ))
                })?;
            rename_legacy_field(descriptor, "trades_per_1000_bars", "tradesPer1000Bars");
            rename_legacy_field(descriptor, "average_bars_held", "averageBarsHeld");
            rename_legacy_field(descriptor, "drawdown_percent", "drawdownPercent");
            rename_legacy_field(descriptor, "win_rate_percent", "winRatePercent");
            rename_legacy_field(descriptor, "long_short_skew", "longShortSkew");
            descriptor.insert("entryConditions".into(), json!(entry_conditions));
            descriptor.insert("exitConditions".into(), json!(exit_conditions));

            let niche = entry
                .get_mut("niche")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| {
                    DesktopError::InvalidArtifact(format!(
                        "legacy {entries_key} contains an invalid niche"
                    ))
                })?;
            rename_legacy_field(niche, "trade_frequency", "tradeFrequency");
            rename_legacy_field(niche, "hold_time", "holdTime");
            rename_legacy_field(niche, "win_rate", "winRate");
            rename_legacy_field(niche, "long_short_skew", "longShortSkew");
            niche.insert("entryConditions".into(), json!(entry_conditions));
        }
    }
    Ok(())
}

fn rename_legacy_field(
    object: &mut serde_json::Map<String, Value>,
    legacy: &str,
    current: &str,
) {
    if let Some(value) = object.remove(legacy) {
        object.insert(current.into(), value);
    }
}

#[tauri::command]
pub fn get_elite(
    fingerprint: String,
    state: State<'_, DesktopState>,
) -> Result<EliteDetail, String> {
    get_elite_from_state(&fingerprint, &state).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_elite_partition_equity(
    fingerprint: String,
    state: State<'_, DesktopState>,
) -> Result<PartitionEquityView, String> {
    let snapshot = {
        let loaded = state
            .loaded
            .read()
            .map_err(|_| DesktopError::StateUnavailable.to_string())?;
        let loaded = loaded
            .as_ref()
            .ok_or_else(|| DesktopError::NoDatabank.to_string())?;
        let elite = loaded
            .bank
            .elites
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            .ok_or_else(|| DesktopError::MissingElite(fingerprint.clone()).to_string())?
            .clone();
        (
            elite,
            loaded.source.clone(),
            loaded.broker.clone(),
            loaded.metadata_path.clone(),
            loaded.m1_source.clone(),
            loaded.m1_metadata_path.clone(),
            loaded.bank.config.scout.clone(),
            loaded.validation_fraction,
            loaded.sealed_fraction,
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        let (
            elite,
            source,
            broker_path,
            metadata_path,
            m1_source,
            m1_metadata_path,
            scout,
            validation_fraction,
            sealed_fraction,
        ) = snapshot;
        partition_equity_for_elite(
            &elite,
            &source,
            metadata_path.as_deref(),
            &broker_path,
            m1_source.as_deref(),
            m1_metadata_path.as_deref(),
            &scout,
            validation_fraction,
            sealed_fraction,
        )
    })
    .await
    .map_err(|error| format!("partition equity task failed: {error}"))?
}

#[tauri::command]
pub async fn run_elite_robustness(
    request: ResultsRobustnessRequest,
    state: State<'_, DesktopState>,
) -> Result<ResultsRobustnessView, String> {
    let snapshot = {
        let loaded = state
            .loaded
            .read()
            .map_err(|_| DesktopError::StateUnavailable.to_string())?;
        let loaded = loaded
            .as_ref()
            .ok_or_else(|| DesktopError::NoDatabank.to_string())?;
        let elite = loaded
            .bank
            .elites
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == request.fingerprint)
            .ok_or_else(|| DesktopError::MissingElite(request.fingerprint.clone()).to_string())?
            .clone();
        RobustnessSnapshot {
            elite,
            databank_path: loaded.databank_path.clone(),
            source: loaded.source.clone(),
            broker: loaded.broker.clone(),
            metadata_path: loaded.metadata_path.clone(),
            m1_source: loaded.m1_source.clone().ok_or_else(|| {
                "This legacy databank does not bind its M1 source. Run a new M1-verified Discover search before launching Results robustness.".to_owned()
            })?,
            m1_metadata_path: loaded.m1_metadata_path.clone(),
            validation_fraction: loaded.validation_fraction,
            sealed_fraction: loaded.sealed_fraction,
            data_hash: loaded.bank.data_hash.clone(),
            broker_spec_hash: loaded.bank.broker_spec_hash.clone(),
            grammar_version: loaded.bank.grammar_version.clone(),
            config: loaded.bank.config.clone(),
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        run_elite_robustness_sync(&request, &snapshot)
    })
    .await
    .map_err(|error| format!("Results robustness task failed: {error}"))?
}

fn run_elite_robustness_sync(
    request: &ResultsRobustnessRequest,
    snapshot: &RobustnessSnapshot,
) -> Result<ResultsRobustnessView, String> {
    let decision_source = crate::data_lab::load_data_source(
        &snapshot.source,
        snapshot.metadata_path.as_deref(),
        None,
    )?;
    let m1_source = crate::data_lab::load_data_source(
        &snapshot.m1_source,
        snapshot.m1_metadata_path.as_deref(),
        None,
    )?;
    let broker = load_bound_broker(&snapshot.broker, decision_source.metadata.as_ref())?;
    load_bound_broker(&snapshot.broker, m1_source.metadata.as_ref())?;

    // Reconstruct the exact Selected-TF candles from M1, then recover the same
    // IS partition that Discover hashed into the databank. This prevents a
    // Results retest from silently drifting onto full history or OOS1/OOS2.
    let full_decision =
        build_decision_from_m1(&m1_source.dataset, Some(&decision_source.dataset))?;
    let is_decision = databank_decision_partition(
        &full_decision,
        &snapshot.data_hash,
        snapshot.validation_fraction,
        snapshot.sealed_fraction,
    )?;
    let selected_timeframe = evaluate_strategy(
        &snapshot.elite.strategy,
        &is_decision,
        &broker,
        &snapshot.config.scout,
    )
    .map_err(|error| format!("Selected-timeframe IS replay failed: {error}"))?;

    let (folds, monte_carlo_trials, neighborhood_samples) =
        robustness_depth(&snapshot.config, request.mode);
    let search = &snapshot.config.search_ranges;
    let config = RobustnessConfig {
        folds,
        monte_carlo_trials,
        neighborhood_samples,
        seed: snapshot.config.seed,
        initial_balance: snapshot.config.scout.initial_balance,
        costs: snapshot.config.scout.costs.clone(),
        entry_window: snapshot.config.scout.entry_window,
        minimum_return_retention: snapshot.config.precision.minimum_return_retention,
        minimum_fold_trades: snapshot.config.deposit_gates.minimum_trades.clamp(1, 2),
        minimum_return_percent: snapshot.config.deposit_gates.minimum_return_percent,
        minimum_profit_factor: snapshot.config.deposit_gates.minimum_profit_factor.min(1.0),
        maximum_drawdown_percent: snapshot
            .config
            .deposit_gates
            .maximum_drawdown_percent
            .max(30.0),
        minimum_passing_fold_fraction: 0.6,
        minimum_neighborhood_survival_fraction: snapshot
            .config
            .minimum_neighborhood_survival_fraction
            .clamp(0.25, 1.0),
        parameter_perturbation_fraction: snapshot.config.robustness_perturbation_fraction,
        adx_period_min: search.indicator_period.minimum.round().max(2.0) as u16,
        adx_period_max: search.indicator_period.maximum.round().max(2.0) as u16,
        adx_period_step: search.indicator_period.step.round().max(1.0) as u16,
        adx_threshold_min: search.adx_threshold.minimum,
        adx_threshold_max: search.adx_threshold.maximum,
        adx_threshold_step: search.adx_threshold.step,
        indicator_engine: snapshot.config.scout.indicator_engine,
        calendar_year_folds: snapshot.config.calendar_year_folds,
    };
    let outcome = run_m1_predeposit_robustness(
        &snapshot.elite.strategy,
        &is_decision,
        &m1_source.dataset,
        &broker,
        &config,
        &selected_timeframe.metrics,
    );
    let (passed, blocker, message, evidence) = match outcome {
        Ok(outcome) => (
            true,
            None,
            "Passed M1 retention, walk-forward, Monte Carlo and parameter-neighborhood gates."
                .to_owned(),
            outcome.evidence,
        ),
        Err(reject) => {
            let (blocker, message) = robustness_reject_detail(reject);
            (false, Some(blocker.to_owned()), message.to_owned(), None)
        }
    };

    let manifest = RunManifest::new(
        "elite-robustness",
        RunRecipe {
            data_hash: Some(snapshot.data_hash.clone()),
            broker_spec_hash: Some(snapshot.broker_spec_hash.clone()),
            grammar_version: Some(snapshot.grammar_version.clone()),
            seed: Some(snapshot.config.seed),
            config: BTreeMap::from([
                ("databank_source".into(), json!(&snapshot.databank_path)),
                ("decision_source".into(), json!(&snapshot.source)),
                ("m1_source".into(), json!(&snapshot.m1_source)),
                ("strategy_fingerprint".into(), json!(&request.fingerprint)),
                ("mode".into(), json!(request.mode)),
                ("folds".into(), json!(folds)),
                ("monte_carlo_trials".into(), json!(monte_carlo_trials)),
                (
                    "monte_carlo_skip_trade_probability".into(),
                    json!(quantforge_discover::MONTE_CARLO_SKIP_TRADE_PROBABILITY),
                ),
                ("neighborhood_samples".into(), json!(neighborhood_samples)),
                (
                    "parameter_perturbation_fraction".into(),
                    json!(config.parameter_perturbation_fraction),
                ),
                (
                    "minimum_return_retention".into(),
                    json!(config.minimum_return_retention),
                ),
                (
                    "minimum_neighborhood_survival_fraction".into(),
                    json!(config.minimum_neighborhood_survival_fraction),
                ),
                ("passed".into(), json!(passed)),
                ("blocker".into(), json!(&blocker)),
            ]),
            override_flags: Vec::new(),
        },
    )
    .map_err(|error| error.to_string())?;
    let report_directory = Path::new(&snapshot.databank_path)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("Robustness");
    fs::create_dir_all(&report_directory)
        .map_err(|error| format!("cannot create Results robustness directory: {error}"))?;
    let short_fingerprint = request.fingerprint.chars().take(12).collect::<String>();
    let artifact_path = report_directory.join(format!(
        "{}-{}-{}.robustness.json",
        safe_file_stem(&snapshot.elite.strategy.id),
        short_fingerprint,
        manifest.run_id
    ));
    let artifact = ResultsRobustnessArtifact {
        schema_version: 1,
        manifest,
        databank_source: snapshot.databank_path.clone(),
        decision_source: snapshot.source.clone(),
        m1_source: snapshot.m1_source.clone(),
        broker_source: snapshot.broker.clone(),
        strategy_id: snapshot.elite.strategy.id.clone(),
        strategy_fingerprint: snapshot.elite.structural_fingerprint.clone(),
        mode: request.mode,
        validation_fraction: snapshot.validation_fraction,
        sealed_fraction: snapshot.sealed_fraction,
        selected_timeframe_metrics: selected_timeframe.metrics,
        passed,
        blocker: blocker.clone(),
        evidence: evidence.clone(),
    };
    write_json_new(&artifact_path, &artifact).map_err(|error| error.to_string())?;

    Ok(ResultsRobustnessView {
        fingerprint: request.fingerprint.clone(),
        strategy_id: snapshot.elite.strategy.id.clone(),
        mode: request.mode,
        passed,
        blocker,
        message,
        artifact_path: canonical_display(&artifact_path),
        folds,
        monte_carlo_trials,
        neighborhood_samples,
        evidence,
    })
}

fn databank_decision_partition(
    full_decision: &BarDataset,
    expected_hash: &ContentHash,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<BarDataset, String> {
    if &full_decision.data_hash == expected_hash {
        return Ok(full_decision.clone());
    }
    let split = DataSplitPlan::chronological(
        full_decision,
        validation_fraction,
        sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let bars = full_decision.bars[..split.development.bar_count].to_vec();
    let development = BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: full_decision.delimiter,
        source_timezone: full_decision.source_timezone.clone(),
        bars,
    };
    if &development.data_hash != expected_hash {
        return Err(
            "The bound decision/M1 files no longer reproduce this databank's IS hash; robustness refuses to test a different sample."
                .into(),
        );
    }
    Ok(development)
}

fn robustness_reject_detail(reject: RobustnessReject) -> (&'static str, &'static str) {
    match reject {
        RobustnessReject::M1Fidelity => (
            "m1_fidelity",
            "Failed Selected-TF to M1 return, trade-count or drawdown retention.",
        ),
        RobustnessReject::WalkForward => (
            "walk_forward",
            "Failed the M1 walk-forward stability requirement.",
        ),
        RobustnessReject::MonteCarlo => (
            "monte_carlo",
            "Failed the block-bootstrap Monte Carlo requirement.",
        ),
        RobustnessReject::ParamNeighborhood => (
            "parameter_neighborhood",
            "Failed the ±20% parameter-neighborhood or ADX plateau requirement.",
        ),
    }
}

fn robustness_depth(
    config: &DiscoverConfig,
    mode: ResultsRobustnessMode,
) -> (usize, usize, usize) {
    match mode {
        ResultsRobustnessMode::Standard => (
            config.robustness_folds,
            config.robustness_monte_carlo_trials,
            config.robustness_neighborhood_samples,
        ),
        ResultsRobustnessMode::Deep => (
            config.robustness_folds.max(5),
            config.robustness_monte_carlo_trials.max(1_000),
            config.robustness_neighborhood_samples.max(20),
        ),
    }
}

fn partition_equity_for_elite(
    elite: &Elite,
    source: &str,
    metadata_path: Option<&str>,
    broker_path: &str,
    m1_source: Option<&str>,
    m1_metadata_path: Option<&str>,
    scout: &quantforge_eval::ScoutConfig,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<PartitionEquityView, String> {
    let loaded = crate::data_lab::load_data_source(source, metadata_path, None)?;
    // Prefer full decision history. If the databank was built on an IS-only
    // slice whose path still points at full history, this is the right series.
    let broker = crate::data_lab::load_bound_broker(broker_path, loaded.metadata.as_ref())?;
    let m1_source = m1_source.ok_or_else(|| {
        "This legacy databank does not bind its M1 source; reopen it through Discover and run a new M1-verified search before using its full-run curve.".to_owned()
    })?;
    let m1 = crate::data_lab::load_data_source(m1_source, m1_metadata_path, None)?;
    crate::data_lab::load_bound_broker(broker_path, m1.metadata.as_ref())?;
    // Match Discover/Parity Lab: decision OHLC is synthesized from M1 so aggregates
    // align with the exported EA and external MT5 backtests.
    let decision_dataset = build_decision_from_m1(&m1.dataset, Some(&loaded.dataset))?;
    // Use the databank's sealed split, not a hardcoded 20/20. Discover gated this
    // elite on IS/OOS1 cut with these fractions; a mismatched chart invents a
    // different OOS1 window and a false retention ratio.
    let plan = quantforge_quality::DataSplitPlan::chronological(
        &decision_dataset,
        validation_fraction,
        sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let result = quantforge_tick::evaluate_strategy_m1(
        &elite.strategy,
        &decision_dataset,
        &m1.dataset,
        &broker,
        &quantforge_tick::JudgeConfig {
            initial_balance: scout.initial_balance,
            costs: scout.costs.clone(),
            allow_execution_gaps: false,
            indicator_engine: scout.indicator_engine,
            entry_window: scout.entry_window,
        },
    )
    .map_err(|error| format!("M1 full-run replay failed: {error}"))?;

    let is_end = plan.development.end_timestamp_ms_exclusive;
    let oos1_end = plan.validation.end_timestamp_ms_exclusive;
    let oos2_end = plan.sealed_final.end_timestamp_ms_exclusive;

    let is_trades: Vec<_> = result
        .trades
        .iter()
        .filter(|trade| trade.entry_timestamp_ms < is_end)
        .collect();
    let oos1_trades: Vec<_> = result
        .trades
        .iter()
        .filter(|trade| trade.entry_timestamp_ms >= is_end && trade.entry_timestamp_ms < oos1_end)
        .collect();
    let oos2_trades: Vec<_> = result
        .trades
        .iter()
        .filter(|trade| {
            trade.entry_timestamp_ms >= oos1_end && trade.entry_timestamp_ms < oos2_end
        })
        .collect();

    let is_expectancy = mean_expectancy(&is_trades);
    let oos1_expectancy = mean_expectancy(&oos1_trades);
    let oos2_expectancy = mean_expectancy(&oos2_trades);
    let oos1_ratio = (is_expectancy > 0.0 && oos1_expectancy.is_finite())
        .then_some(oos1_expectancy / is_expectancy);

    let points = downsample_equity(&result.equity, 480, is_end, oos1_end);
    let trades: Vec<TradeRowView> = result
        .trades
        .iter()
        .take(2_000)
        .map(|trade| TradeRowView {
            side: format!("{:?}", trade.side).to_ascii_lowercase(),
            entry_timestamp_ms: trade.entry_timestamp_ms,
            exit_timestamp_ms: trade.exit_timestamp_ms,
            entry_price: trade.entry_price,
            exit_price: trade.exit_price,
            net_profit: trade.net_profit,
            exit_reason: format!("{:?}", trade.exit_reason)
                .replace('_', " ")
                .to_ascii_lowercase(),
        })
        .collect();

    Ok(PartitionEquityView {
        fingerprint: elite.structural_fingerprint.as_str().into(),
        strategy_id: elite.strategy.id.clone(),
        execution_engine: result.engine.clone(),
        initial_balance: scout.initial_balance,
        points,
        is_end_timestamp_ms: is_end,
        oos1_end_timestamp_ms: oos1_end,
        oos2_end_timestamp_ms: oos2_end,
        is_bars: plan.development.bar_count,
        oos1_bars: plan.validation.bar_count,
        oos2_bars: plan.sealed_final.bar_count,
        is_expectancy,
        oos1_expectancy,
        oos1_expectancy_ratio: oos1_ratio,
        oos2_expectancy,
        is_return_percent: segment_return(&result.equity, scout.initial_balance, None, Some(is_end)),
        oos1_return_percent: segment_return(
            &result.equity,
            scout.initial_balance,
            Some(is_end),
            Some(oos1_end),
        ),
        oos2_return_percent: segment_return(
            &result.equity,
            scout.initial_balance,
            Some(oos1_end),
            Some(oos2_end),
        ),
        is_trades: is_trades.len(),
        oos1_trades: oos1_trades.len(),
        oos2_trades: oos2_trades.len(),
        full_run_trades: result.metrics.trade_count,
        full_run_return_percent: result.metrics.return_percent,
        full_run_net_profit: result.metrics.net_profit,
        full_run_max_drawdown: result.metrics.max_drawdown,
        full_run_max_drawdown_percent: result.metrics.max_drawdown_percent,
        full_run_profit_factor: result.metrics.profit_factor,
        full_run_win_rate: result.metrics.win_rate,
        full_run_sharpe_ratio: result.metrics.sharpe_ratio,
        full_run_recovery_factor: finite_recovery_factor(&result.metrics),
        trades,
    })
}

fn mean_expectancy(trades: &[&quantforge_eval::Trade]) -> f64 {
    if trades.is_empty() {
        return 0.0;
    }
    trades.iter().map(|trade| trade.net_profit).sum::<f64>() / trades.len() as f64
}

fn segment_return(
    equity: &[quantforge_eval::EquityPoint],
    initial_balance: f64,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> f64 {
    let start_equity = start_ms
        .and_then(|boundary| {
            equity
                .iter()
                .rev()
                .find(|point| point.timestamp_ms < boundary)
                .map(|point| point.equity)
        })
        .unwrap_or(initial_balance);
    let end_equity = end_ms
        .and_then(|boundary| {
            equity
                .iter()
                .rev()
                .find(|point| point.timestamp_ms < boundary)
                .map(|point| point.equity)
        })
        .or_else(|| equity.last().map(|point| point.equity))
        .unwrap_or(start_equity);
    if start_equity.abs() < 1e-12 {
        return 0.0;
    }
    ((end_equity - start_equity) / start_equity) * 100.0
}

fn downsample_equity(
    equity: &[quantforge_eval::EquityPoint],
    target: usize,
    is_end: i64,
    oos1_end: i64,
) -> Vec<PartitionEquityPoint> {
    if equity.is_empty() {
        return Vec::new();
    }
    if equity.len() <= target {
        return equity
            .iter()
            .map(|point| PartitionEquityPoint {
                timestamp_ms: point.timestamp_ms,
                equity: point.equity,
            })
            .collect();
    }
    let mut keep = std::collections::BTreeSet::new();
    keep.insert(0);
    keep.insert(equity.len() - 1);
    if let Some(index) = equity
        .iter()
        .position(|point| point.timestamp_ms >= is_end)
        .map(|index| index.saturating_sub(1))
    {
        keep.insert(index);
    }
    if let Some(index) = equity
        .iter()
        .position(|point| point.timestamp_ms >= oos1_end)
        .map(|index| index.saturating_sub(1))
    {
        keep.insert(index);
    }
    let step = ((equity.len() - 1) as f64 / (target.saturating_sub(1) as f64)).max(1.0);
    let mut cursor = 0.0;
    while (cursor as usize) < equity.len() {
        keep.insert(cursor as usize);
        cursor += step;
    }
    keep.into_iter()
        .filter_map(|index| equity.get(index))
        .map(|point| PartitionEquityPoint {
            timestamp_ms: point.timestamp_ms,
            equity: point.equity,
        })
        .collect()
}

#[tauri::command]
pub fn export_elite_strategy(
    fingerprint: String,
    path: String,
    state: State<'_, DesktopState>,
) -> Result<String, String> {
    let loaded = state
        .loaded
        .read()
        .map_err(|_| DesktopError::StateUnavailable.to_string())?;
    let loaded = loaded
        .as_ref()
        .ok_or_else(|| DesktopError::NoDatabank.to_string())?;
    let elite = loaded
        .bank
        .elites
        .iter()
        .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
        .ok_or_else(|| DesktopError::MissingElite(fingerprint).to_string())?;
    quantforge_storage::write_json_new(&path, &elite.strategy)
        .map_err(|error| error.to_string())?;
    Ok(Path::new(&path)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&path).to_path_buf())
        .display()
        .to_string())
}

#[tauri::command]
pub fn export_elite_strategies(
    fingerprints: Vec<String>,
    directory: String,
    state: State<'_, DesktopState>,
) -> Result<BatchExportView, String> {
    export_elite_strategies_to(&fingerprints, Path::new(&directory), &state)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn export_elite_eas(
    request: BatchEaExportRequest,
    state: State<'_, DesktopState>,
) -> Result<BatchEaExportView, String> {
    export_elite_eas_to(&request, Path::new(&request.directory), &state)
        .map_err(|error| error.to_string())
}

/// Stage an elite strategy IR into a Vault `candidates/` folder.
/// This is not Certified admission — full certify_to_vault still requires the evidence chain.
#[tauri::command]
pub fn promote_elite_to_vault(
    fingerprint: String,
    vault_directory: String,
    state: State<'_, DesktopState>,
) -> Result<String, String> {
    let loaded = state
        .loaded
        .read()
        .map_err(|_| DesktopError::StateUnavailable.to_string())?;
    let loaded = loaded
        .as_ref()
        .ok_or_else(|| DesktopError::NoDatabank.to_string())?;
    if loaded.legacy_read_only {
        return Err(
            "Schema-v5 databanks are read-only in QuantForge v6. Results retests are allowed, but staging/certification requires a fresh v6 Discover archive."
                .into(),
        );
    }
    let elite = loaded
        .bank
        .elites
        .iter()
        .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
        .ok_or_else(|| DesktopError::MissingElite(fingerprint.clone()).to_string())?;
    let root = PathBuf::from(&vault_directory);
    let candidates = root.join("candidates");
    fs::create_dir_all(&candidates).map_err(|error| error.to_string())?;
    let safe_name = fingerprint
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    let path = candidates.join(format!("{safe_name}.ir.json"));
    if path.exists() {
        return Err(format!(
            "candidate already staged at {}; remove it before promoting again",
            path.display()
        ));
    }
    quantforge_storage::write_json_new(&path, &elite.strategy).map_err(|error| error.to_string())?;
    Ok(path
        .canonicalize()
        .unwrap_or(path)
        .display()
        .to_string())
}

fn export_elite_eas_to(
    request: &BatchEaExportRequest,
    directory: &Path,
    state: &DesktopState,
) -> Result<BatchEaExportView, DesktopError> {
    if request.fingerprints.is_empty() {
        return Err(DesktopError::InvalidExport(
            "select at least one elite".into(),
        ));
    }
    if !directory.is_dir() {
        return Err(DesktopError::InvalidExport(format!(
            "{} is not an existing directory",
            directory.display()
        )));
    }
    if directory
        .read_dir()
        .map_err(DesktopError::Io)?
        .next()
        .transpose()
        .map_err(DesktopError::Io)?
        .is_some()
    {
        return Err(DesktopError::InvalidExport(format!(
            "{} is not empty; choose an empty folder",
            directory.display()
        )));
    }
    let unique: std::collections::BTreeSet<_> = request.fingerprints.iter().collect();
    if unique.len() != request.fingerprints.len() {
        return Err(DesktopError::InvalidExport(
            "the selection contains duplicate fingerprints".into(),
        ));
    }
    if request.base_magic == 0 {
        return Err(DesktopError::InvalidExport(
            "base magic must be greater than zero".into(),
        ));
    }
    let final_magic = request
        .base_magic
        .checked_add(request.fingerprints.len().saturating_sub(1) as u64)
        .ok_or_else(|| DesktopError::InvalidExport("base magic is too large".into()))?;
    if final_magic == 0 {
        return Err(DesktopError::InvalidExport(
            "base magic range may not include zero".into(),
        ));
    }

    let loaded = state
        .loaded
        .read()
        .map_err(|_| DesktopError::StateUnavailable)?;
    let loaded = loaded.as_ref().ok_or(DesktopError::NoDatabank)?;
    let broker = load_bound_broker(&loaded.broker, None).map_err(DesktopError::InvalidExport)?;
    let costs = &loaded.bank.config.scout.costs;
    let mut exports = Vec::with_capacity(request.fingerprints.len());
    let mut used_names = std::collections::BTreeSet::new();
    for (offset, fingerprint) in request.fingerprints.iter().enumerate() {
        let elite = loaded
            .bank
            .elites
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            .ok_or_else(|| DesktopError::MissingElite(fingerprint.clone()))?;
        let unique_number = request.base_magic + offset as u64;
        // Name each expert after its own generation and candidate index so a
        // folder of exports is identifiable in the MT5 navigator.
        let preferred = quantforge_export_mql5::suggested_expert_name(
            &broker.symbol,
            &elite.strategy.id,
            unique_number,
        );
        let expert_name = if used_names.insert(preferred.clone()) {
            preferred
        } else {
            let fallback = format!("{preferred}_{unique_number}");
            used_names.insert(fallback.clone());
            fallback
        };
        let config = Mql5ExportConfig {
            expert_name: expert_name.clone(),
            expert_directory: "QuantForge".into(),
            timeframe: request.timeframe.clone(),
            magic: unique_number,
            deviation_points: 10,
            max_spread_points: costs.max_spread_points,
            estimated_slippage_points_per_side: costs.adverse_slippage_points_per_side,
            commission_per_lot_round_turn: costs.commission_per_lot_round_turn,
            allow_live_trading_default: false,
            export_style: quantforge_export_mql5::ExportStyle::Sqx,
            entry_window_start_hour: loaded.bank.config.scout.entry_window.start_hour,
            entry_window_end_hour: loaded.bank.config.scout.entry_window.end_hour,
            tester: TesterConfig {
                deposit: loaded.bank.config.scout.initial_balance,
                // Model 1 is M1 OHLC, which is the resolution the M1 judge
                // already uses. Real ticks (model 4) multiply tester runtime by
                // orders of magnitude without changing a bar-close decision, so
                // it belongs to the final parity run rather than batch review.
                model: 1,
                ..TesterConfig::default()
            },
        };
        let bundle = generate_bundle(&elite.strategy, &broker, &config)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        let strategy_path = directory.join(format!("{expert_name}.strategy.ir.json"));
        let source_path = directory.join(format!("{expert_name}.mq5"));
        let settings_path = directory.join(format!("{expert_name}.set"));
        let tester_path = directory.join(format!("{expert_name}.tester.ini"));
        let evidence_path = directory.join(format!("{expert_name}.evidence.json"));
        exports.push((
            elite,
            expert_name,
            config.magic,
            bundle,
            strategy_path,
            source_path,
            settings_path,
            tester_path,
            evidence_path,
        ));
    }
    let index_path = directory.join("quantforge-ea-batch.json");
    if index_path.exists() {
        return Err(DesktopError::InvalidExport(format!(
            "{} already exists; choose an empty folder",
            index_path.display()
        )));
    }

    for (elite, _, _, bundle, strategy, source, settings, tester, evidence) in &exports {
        quantforge_storage::write_json_new(strategy, &elite.strategy)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        write_text_new(source, &bundle.source)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        for support in &bundle.support_files {
            let path = directory.join(&support.relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
            }
            write_text_new(&path, &support.contents)
                .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        }
        write_text_new(settings, &bundle.set_file)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        write_text_new(tester, &bundle.tester_ini)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        quantforge_storage::write_json_new(evidence, &bundle.evidence)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    }
    let index = serde_json::json!({
        "schema_version": 1,
        "kind": "quantforge-mql5-ea-batch",
        "purpose": "research export; generated experts default to live trading disabled",
        "grammar_version": loaded.bank.grammar_version,
        "data_hash": loaded.bank.data_hash,
        "execution_data_hash": loaded.bank.execution_data_hash,
        "broker_spec_hash": loaded.bank.broker_spec_hash,
        "timeframe": request.timeframe,
        "tester_model": "1 minute OHLC",
        "strategies": exports.iter().map(|(elite, expert_name, magic, _, strategy, source, settings, tester, evidence)| serde_json::json!({
            "fingerprint": elite.structural_fingerprint,
            "strategy_id": elite.strategy.id,
            "entry_conditions": elite.niche.entry_conditions,
            "exit_conditions": elite.descriptor.exit_conditions,
            "grade": "illuminated",
            "magic": magic,
            "expert_name": expert_name,
            "strategy_ir": canonical_display(strategy),
            "source": canonical_display(source),
            "settings": canonical_display(settings),
            "tester": canonical_display(tester),
            "evidence": canonical_display(evidence),
        })).collect::<Vec<_>>(),
    });
    quantforge_storage::write_json_new(&index_path, &index)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;

    Ok(BatchEaExportView {
        directory: canonical_display(directory),
        index_path: canonical_display(&index_path),
        expert_paths: exports
            .iter()
            .map(|(_, _, _, _, _, source, _, _, _)| canonical_display(source))
            .collect(),
        settings_paths: exports
            .iter()
            .map(|(_, _, _, _, _, _, settings, _, _)| canonical_display(settings))
            .collect(),
        tester_paths: exports
            .iter()
            .map(|(_, _, _, _, _, _, _, tester, _)| canonical_display(tester))
            .collect(),
        evidence_paths: exports
            .iter()
            .map(|(_, _, _, _, _, _, _, _, evidence)| canonical_display(evidence))
            .collect(),
    })
}

fn export_elite_strategies_to(
    fingerprints: &[String],
    directory: &Path,
    state: &DesktopState,
) -> Result<BatchExportView, DesktopError> {
    if fingerprints.is_empty() {
        return Err(DesktopError::InvalidExport(
            "select at least one elite".into(),
        ));
    }
    if !directory.is_dir() {
        return Err(DesktopError::InvalidExport(format!(
            "{} is not an existing directory",
            directory.display()
        )));
    }
    let unique: std::collections::BTreeSet<_> = fingerprints.iter().collect();
    if unique.len() != fingerprints.len() {
        return Err(DesktopError::InvalidExport(
            "the selection contains duplicate fingerprints".into(),
        ));
    }

    let loaded = state
        .loaded
        .read()
        .map_err(|_| DesktopError::StateUnavailable)?;
    let bank = &loaded.as_ref().ok_or(DesktopError::NoDatabank)?.bank;
    let mut exports = Vec::with_capacity(fingerprints.len());
    for fingerprint in fingerprints {
        let elite = bank
            .elites
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            .ok_or_else(|| DesktopError::MissingElite(fingerprint.clone()))?;
        let prefix = &fingerprint[..fingerprint.len().min(12)];
        let file_name = format!(
            "{}.{}.strategy.ir.json",
            safe_file_stem(&elite.strategy.id),
            prefix
        );
        exports.push((elite, directory.join(file_name)));
    }
    let index_path = directory.join("quantforge-strategy-batch.json");
    if let Some(existing) = exports
        .iter()
        .map(|(_, path)| path)
        .chain(std::iter::once(&index_path))
        .find(|path| path.exists())
    {
        return Err(DesktopError::InvalidExport(format!(
            "{} already exists; choose an empty folder",
            existing.display()
        )));
    }

    for (elite, path) in &exports {
        quantforge_storage::write_json_new(path, &elite.strategy)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    }
    let index = serde_json::json!({
        "schema_version": 1,
        "grammar_version": bank.grammar_version,
        "data_hash": bank.data_hash,
        "execution_data_hash": bank.execution_data_hash,
        "broker_spec_hash": bank.broker_spec_hash,
        "strategies": exports.iter().map(|(elite, path)| serde_json::json!({
            "fingerprint": elite.structural_fingerprint,
            "strategy_id": elite.strategy.id,
            "entry_conditions": elite.niche.entry_conditions,
            "exit_conditions": elite.descriptor.exit_conditions,
            "return_percent": elite.metrics.return_percent,
            "recovery_factor": finite_recovery_factor(&elite.metrics),
            "profit_factor": elite.metrics.profit_factor,
            "sharpe_ratio": effective_sharpe(elite),
            "maximum_drawdown_percent": elite.metrics.max_drawdown_percent,
            "trades": elite.metrics.trade_count,
            "path": path.canonicalize().unwrap_or_else(|_| path.clone()),
        })).collect::<Vec<_>>(),
    });
    quantforge_storage::write_json_new(&index_path, &index)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;

    Ok(BatchExportView {
        directory: canonical_display(directory),
        index_path: canonical_display(&index_path),
        strategy_paths: exports
            .iter()
            .map(|(_, path)| canonical_display(path))
            .collect(),
    })
}

fn get_elite_from_state(
    fingerprint: &str,
    state: &DesktopState,
) -> Result<EliteDetail, DesktopError> {
    let loaded = state
        .loaded
        .read()
        .map_err(|_| DesktopError::StateUnavailable)?;
    let loaded = loaded.as_ref().ok_or(DesktopError::NoDatabank)?;
    let elite = loaded
        .bank
        .elites
        .iter()
        .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
        .ok_or_else(|| DesktopError::MissingElite(fingerprint.into()))?;
    elite_detail(elite).map_err(DesktopError::Json)
}

pub(crate) fn verify_artifact(artifact: &EvolveArtifact) -> Result<(), DesktopError> {
    artifact
        .manifest
        .validate()
        .map_err(|error| DesktopError::InvalidArtifact(error.to_string()))?;
    artifact
        .databank
        .validate_integrity()
        .map_err(|error| DesktopError::InvalidArtifact(error.to_string()))?;
    if artifact.manifest.command != "evolve"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&artifact.databank.data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&artifact.databank.broker_spec_hash)
        || artifact.manifest.recipe.grammar_version.as_deref()
            != Some(quantforge_discover::GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || artifact.coverage != artifact.databank.coverage()
        || (artifact.qd_score - artifact.databank.qd_score()).abs() > 1.0e-9
        || artifact.manifest.recipe.config.get("discover_config")
            != Some(&serde_json::to_value(&artifact.databank.config).map_err(DesktopError::Json)?)
    {
        return Err(DesktopError::InvalidArtifact(
            "manifest, quality, coverage or QD score does not match the persisted archive".into(),
        ));
    }
    Ok(())
}

fn verify_legacy_artifact(artifact: &EvolveArtifact) -> Result<(), DesktopError> {
    artifact
        .manifest
        .validate()
        .map_err(|error| DesktopError::InvalidArtifact(error.to_string()))?;
    let bank = &artifact.databank;
    if artifact.manifest.command != "evolve"
        || bank.schema_version != LEGACY_DATABANK_SCHEMA_VERSION
        || bank.grammar_version != LEGACY_GRAMMAR_VERSION
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&bank.data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&bank.broker_spec_hash)
        || artifact.manifest.recipe.grammar_version.as_deref() != Some(LEGACY_GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || artifact.coverage != bank.coverage()
        || (artifact.qd_score - bank.qd_score()).abs() > 1.0e-9
        || bank.evaluation_count == 0
        || (bank.elites.is_empty() && bank.accepted_pool.is_empty())
    {
        return Err(DesktopError::InvalidArtifact(
            "legacy manifest, quality, coverage or QD score does not match the persisted archive"
                .into(),
        ));
    }
    verify_legacy_entries(
        &bank.elites,
        &bank.config,
        bank.completed_generations,
    )?;
    verify_legacy_entries(
        &bank.accepted_pool,
        &bank.config,
        bank.completed_generations,
    )
}

fn verify_legacy_entries(
    entries: &[Elite],
    config: &DiscoverConfig,
    completed_generations: u64,
) -> Result<(), DesktopError> {
    for elite in entries {
        let fingerprint = elite
            .strategy
            .structural_fingerprint(FloatPolicy::default())
            .map_err(|error| DesktopError::InvalidArtifact(error.to_string()))?;
        let effective_profit_factor = elite.metrics.profit_factor.unwrap_or({
            if elite.metrics.net_profit > 0.0 && elite.metrics.winning_trades > 0 {
                f64::MAX
            } else {
                0.0
            }
        });
        let fixed_risk = matches!(
            elite.strategy.risk,
            RiskPolicy::FixedCurrency { amount }
                if (amount - quantforge_discover::FIXED_RISK_PER_TRADE).abs() <= 1.0e-9
        );
        let recovery = elite.metrics.recovery_factor();
        if fingerprint != elite.structural_fingerprint
            || strategy_entry_condition_count(&elite.strategy)
                != elite.descriptor.entry_conditions
            || strategy_exit_condition_count(&elite.strategy)
                != elite.descriptor.exit_conditions
            || niche_from_descriptor(&elite.descriptor) != elite.niche
            || elite.strategy.manage.flatten_end_of_day != config.flatten_at_22
            || elite.strategy.manage.max_one_entry_per_day != config.max_one_entry_per_day
            || !fixed_risk
            || elite.metrics.trade_count < config.deposit_gates.minimum_trades
            || elite.metrics.return_percent <= config.deposit_gates.minimum_return_percent
            || effective_profit_factor < config.deposit_gates.minimum_profit_factor
            || recovery < config.deposit_gates.minimum_recovery_factor
            || elite.metrics.max_drawdown_percent > config.deposit_gates.maximum_drawdown_percent
            || elite.discovered_generation > completed_generations
            || !elite.evidence.total.is_finite()
            || !elite.novelty.is_finite()
        {
            return Err(DesktopError::InvalidArtifact(
                "a legacy elite is structurally invalid or no longer passes its stored gates"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn strategy_entry_condition_count(strategy: &StrategyIr) -> usize {
    strategy
        .entry
        .long
        .as_ref()
        .or(strategy.entry.short.as_ref())
        .map(|expression| match expression {
            BoolExpr::And { children } => children.len(),
            _ => 1,
        })
        .unwrap_or(0)
}

fn strategy_exit_condition_count(strategy: &StrategyIr) -> usize {
    strategy
        .exit_long
        .as_ref()
        .or(strategy.exit_short.as_ref())
        .or(strategy.exit.as_ref())
        .map(|expression| match expression {
            BoolExpr::Or { children } => children.len(),
            _ => 1,
        })
        .unwrap_or(0)
}

fn niche_from_descriptor(
    descriptor: &quantforge_discover::BehaviorDescriptor,
) -> NicheKey {
    NicheKey {
        entry_conditions: descriptor.entry_conditions,
        trade_frequency: three_level_bucket(descriptor.trades_per_1000_bars, 5.0, 20.0),
        hold_time: three_level_bucket(descriptor.average_bars_held, 4.0, 24.0),
        drawdown: three_level_bucket(descriptor.drawdown_percent, 5.0, 15.0),
        win_rate: three_level_bucket(descriptor.win_rate_percent, 35.0, 55.0),
        long_short_skew: if descriptor.long_short_skew < -0.25 {
            LongShortSkewBucket::ShortHeavy
        } else if descriptor.long_short_skew > 0.25 {
            LongShortSkewBucket::LongHeavy
        } else {
            LongShortSkewBucket::Balanced
        },
    }
}

fn three_level_bucket(value: f64, first: f64, second: f64) -> ThreeLevelBucket {
    if value < first {
        ThreeLevelBucket::Low
    } else if value < second {
        ThreeLevelBucket::Medium
    } else {
        ThreeLevelBucket::High
    }
}

fn workspace_view(
    artifact: &EvolveArtifact,
    source_path: &Path,
    artifact_hash: &ContentHash,
) -> DatabankWorkspace {
    let bank = &artifact.databank;
    let telemetry = &bank.telemetry;
    let total_rejections = telemetry.rejected_gate
        + telemetry.rejected_deposit_gate
        + telemetry.rejected_clone
        + telemetry.rejected_correlated
        + telemetry.rejected_niche_not_improved
        + telemetry.rejected_precision
        + telemetry.rejected_ambiguous
        + telemetry.rejected_oos1
        + telemetry.rejected_m1_fidelity
        + telemetry.rejected_walk_forward
        + telemetry.rejected_monte_carlo
        + telemetry.rejected_param_neighborhood
        + telemetry.rejected_evaluation;
    DatabankWorkspace {
        source_path: source_path.display().to_string(),
        data_path: artifact.source.clone(),
        metadata_path: companion_metadata_path(&artifact.source),
        m1_data_path: manifest_path(artifact, "m1_source"),
        m1_metadata_path: manifest_path(artifact, "m1_source")
            .and_then(|path| companion_metadata_path(&path)),
        broker_path: artifact.broker.clone(),
        commission_per_lot_round_turn: bank.config.scout.costs.commission_per_lot_round_turn,
        slippage_points_per_side: bank.config.scout.costs.adverse_slippage_points_per_side,
        initial_balance: bank.config.scout.initial_balance,
        artifact_hash: artifact_hash.as_str().into(),
        run_id: artifact.manifest.run_id.clone(),
        created_at: artifact.manifest.created_at.to_rfc3339(),
        data_hash: bank.data_hash.as_str().into(),
        broker_spec_hash: bank.broker_spec_hash.as_str().into(),
        grammar_version: bank.grammar_version.clone(),
        legacy_read_only: bank.schema_version == LEGACY_DATABANK_SCHEMA_VERSION,
        quality_grade: format!("{:?}", artifact.data_quality.grade).to_ascii_lowercase(),
        quality_score: artifact.data_quality.score,
        coverage: bank.coverage(),
        total_niches: if bank.schema_version == LEGACY_DATABANK_SCHEMA_VERSION {
            LEGACY_TOTAL_NICHES
        } else {
            TOTAL_NICHES
        },
        qd_score: bank.qd_score(),
        completed_generations: bank.completed_generations,
        selection_bias: selection_bias(bank.evaluation_count),
        rejections: RejectionTelemetry {
            gate: telemetry.rejected_gate,
            deposit_gate: telemetry.rejected_deposit_gate,
            clone: telemetry.rejected_clone,
            correlated: telemetry.rejected_correlated,
            niche_not_improved: telemetry.rejected_niche_not_improved,
            precision: telemetry.rejected_precision,
            ambiguous: telemetry.rejected_ambiguous,
            oos1: telemetry.rejected_oos1,
            evaluation: telemetry.rejected_evaluation,
            total: total_rejections,
        },
        research_grade: artifact_is_research_grade(artifact),
        require_m1_precision: bank.config.require_m1_precision,
        m1_fidelity_verified: artifact_m1_fidelity_verified(artifact),
        simple_exits: bank.config.simple_exits,
        allow_break_even: bank.config.allow_break_even,
        allow_trailing_stops: bank.config.allow_trailing_stops,
        allow_partial_exits: bank.config.allow_partial_exits,
        allow_market_entries: bank.config.allow_market_entries,
        allow_stop_entries: bank.config.allow_stop_entries,
        allow_limit_entries: bank.config.allow_limit_entries,
        allow_stop_limit_entries: bank.config.allow_stop_limit_entries,
        max_one_entry_per_day: bank.config.max_one_entry_per_day,
        validation_fraction: manifest_fraction(artifact, "validation_fraction", 0.2),
        sealed_fraction: manifest_fraction(artifact, "sealed_fraction", 0.2),
        condition_groups: coverage_condition_groups(bank),
        elites: bank.elites.iter().map(elite_row).collect(),
    }
}

pub(crate) fn artifact_is_research_grade(artifact: &EvolveArtifact) -> bool {
    if artifact_m1_fidelity_verified(artifact) {
        return false;
    }
    if matches!(
        artifact.manifest.recipe.config.get("research_grade"),
        Some(Value::Bool(false))
    ) {
        return false;
    }
    !artifact.databank.config.require_m1_precision
}

pub(crate) fn artifact_m1_fidelity_verified(artifact: &EvolveArtifact) -> bool {
    matches!(
        artifact.manifest.recipe.config.get("m1_fidelity_verified"),
        Some(Value::Bool(true))
    ) || artifact.databank.config.require_m1_precision
}

fn manifest_path(artifact: &EvolveArtifact, key: &str) -> Option<String> {
    artifact
        .manifest
        .recipe
        .config
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn manifest_fraction(artifact: &EvolveArtifact, key: &str, fallback: f64) -> f64 {
    artifact
        .manifest
        .recipe
        .config
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..1.0).contains(value))
        .unwrap_or(fallback)
}

fn companion_metadata_path(data_path: &str) -> Option<String> {
    let candidate = Path::new(data_path).with_extension("metadata.csv");
    candidate.is_file().then(|| canonical_display(&candidate))
}

fn canonical_display(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .display()
        .to_string()
}

fn safe_file_stem(value: &str) -> String {
    let safe: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "strategy".into()
    } else {
        safe
    }
}

fn selection_bias(evaluation_count: u64) -> SelectionBiasView {
    let (level, message) = if evaluation_count > 10_000 {
        (
            "high",
            "More than 10,000 candidate results were compared, so the winner is highly exposed to multiple-testing luck. Require the full Challenge, sealed-final and external-parity chain.",
        )
    } else if evaluation_count > SELECTION_BIAS_WARNING_THRESHOLD {
        (
            "elevated",
            "More than 1,500 candidate results were compared, increasing the chance that a winner benefited from luck. Promotion must retain deflated metrics and robustness evidence.",
        )
    } else {
        (
            "recorded",
            "Evaluation count is retained even below the warning threshold; it must follow the candidate into later evidence.",
        )
    };
    SelectionBiasView {
        evaluation_count,
        level,
        message: message.into(),
    }
}

fn elite_row(elite: &Elite) -> EliteRow {
    EliteRow {
        fingerprint: elite.structural_fingerprint.as_str().into(),
        strategy_id: elite.strategy.id.clone(),
        entry_conditions: elite.niche.entry_conditions,
        exit_conditions: elite.descriptor.exit_conditions,
        evidence: elite.evidence.total,
        novelty: elite.novelty,
        trades: elite.metrics.trade_count,
        return_percent: elite.metrics.return_percent,
        drawdown_percent: elite.metrics.max_drawdown_percent,
        recovery_factor: finite_recovery_factor(&elite.metrics),
        profit_factor: elite.metrics.profit_factor,
        sharpe_ratio: effective_sharpe(elite),
        is_expectancy: elite.is_expectancy,
        oos1_expectancy: elite.oos1_expectancy,
        oos1_expectancy_ratio: elite.oos1_expectancy_ratio,
        complexity: elite.complexity,
        generation: elite.discovered_generation,
        grade: "illuminated",
        parity: "unknown",
        equity_signature: elite.equity_signature.clone(),
    }
}

fn finite_recovery_factor(metrics: &quantforge_eval::BacktestMetrics) -> Option<f64> {
    let value = metrics.recovery_factor();
    value.is_finite().then_some(value)
}

fn effective_sharpe(elite: &Elite) -> Option<f64> {
    elite
        .metrics
        .sharpe_ratio
        .filter(|value| value.is_finite())
        .or_else(|| signature_sharpe(&elite.equity_signature))
}

fn signature_sharpe(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (values.len() - 1) as f64;
    let deviation = variance.sqrt();
    (deviation > 1.0e-12)
        .then_some(mean / deviation * (values.len() as f64).sqrt())
        .filter(|value| value.is_finite())
}

fn elite_detail(elite: &Elite) -> Result<EliteDetail, serde_json::Error> {
    Ok(EliteDetail {
        fingerprint: elite.structural_fingerprint.as_str().into(),
        strategy_id: elite.strategy.id.clone(),
        thesis: elite.strategy.meta.thesis_hint.clone(),
        entry_conditions: elite.niche.entry_conditions,
        exit_conditions: elite.descriptor.exit_conditions,
        niche: niche_label(&elite.niche),
        grade: "illuminated",
        parity: "unknown",
        evidence: serde_json::to_value(&elite.evidence)?,
        descriptor: serde_json::to_value(&elite.descriptor)?,
        metrics: serde_json::to_value(&elite.metrics)?,
        strategy_ir: serde_json::to_value(&elite.strategy)?,
        equity_signature: elite.equity_signature.clone(),
        robustness: elite_robustness(elite)?,
    })
}

fn elite_robustness(elite: &Elite) -> Result<Option<EliteRobustnessView>, serde_json::Error> {
    let evidence = elite
        .robustness
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    if elite.gate_results.is_empty() {
        return Ok(evidence.map(|evidence| EliteRobustnessView {
            monte_carlo: None,
            walk_forward: None,
            param_permutation: None,
            summary: None,
            evidence: Some(evidence),
        }));
    }
    let find = |names: &[&str]| {
        elite.gate_results.iter().find_map(|gate| {
            names
                .iter()
                .any(|name| gate.name.eq_ignore_ascii_case(name))
                .then(|| {
                    format!(
                        "{} — {}",
                        if gate.passed { "passed" } else { "failed" },
                        gate.detail
                    )
                })
        })
    };
    let monte_carlo = find(&["monte_carlo", "mc"]);
    let walk_forward = find(&["walk_forward", "wfo"]);
    let param_permutation = find(&["param_neighborhood", "param_permutation", "neighborhood"]);
    let summary = find(&["m1_robustness", "robustness"]);
    if monte_carlo.is_none()
        && walk_forward.is_none()
        && param_permutation.is_none()
        && summary.is_none()
        && evidence.is_none()
    {
        return Ok(None);
    }
    Ok(Some(EliteRobustnessView {
        monte_carlo,
        walk_forward,
        param_permutation,
        summary,
        evidence,
    }))
}

fn coverage_condition_groups(bank: &Databank) -> Vec<ConditionCoverage> {
    let elites: BTreeMap<_, _> = bank
        .elites
        .iter()
        .map(|elite| (elite.structural_fingerprint.as_str(), elite))
        .collect();
    let minimum = bank
        .elites
        .iter()
        .map(|elite| elite.evidence.total)
        .fold(f64::INFINITY, f64::min);
    let maximum = bank
        .elites
        .iter()
        .map(|elite| elite.evidence.total)
        .fold(f64::NEG_INFINITY, f64::max);
    ENTRY_CONDITION_COUNTS
        .into_iter()
        .map(|entry_conditions| {
        let mut cells = Vec::with_capacity(3usize.pow(5));
        for win_rate in three_levels() {
            for skew in skew_levels() {
                for trade_frequency in three_levels() {
                    for hold_time in three_levels() {
                        for drawdown in three_levels() {
                            let niche = NicheKey {
                                entry_conditions,
                                trade_frequency,
                                hold_time,
                                drawdown,
                                win_rate,
                                long_short_skew: skew,
                            };
                            let label = niche_label(&niche);
                            let elite = bank
                                .coverage_map
                                .get(&label)
                                .and_then(|fingerprint| elites.get(fingerprint.as_str()).copied());
                            cells.push(CoverageCell {
                                index: cells.len(),
                                niche: label,
                                occupied: elite.is_some(),
                                fingerprint: elite
                                    .map(|value| value.structural_fingerprint.as_str().to_owned()),
                                intensity: elite
                                    .map(|value| {
                                        evidence_intensity(value.evidence.total, minimum, maximum)
                                    })
                                    .unwrap_or(0.0),
                            });
                        }
                    }
                }
            }
        }
        ConditionCoverage {
            entry_conditions,
            label: format!("{entry_conditions} entry conditions"),
            occupied: cells.iter().filter(|cell| cell.occupied).count(),
            total: cells.len(),
            cells,
        }
    })
    .collect()
}

fn evidence_intensity(value: f64, minimum: f64, maximum: f64) -> f64 {
    if (maximum - minimum).abs() <= f64::EPSILON {
        1.0
    } else {
        (0.2 + 0.8 * ((value - minimum) / (maximum - minimum))).clamp(0.2, 1.0)
    }
}

fn three_levels() -> [ThreeLevelBucket; 3] {
    [
        ThreeLevelBucket::Low,
        ThreeLevelBucket::Medium,
        ThreeLevelBucket::High,
    ]
}

fn skew_levels() -> [LongShortSkewBucket; 3] {
    [
        LongShortSkewBucket::ShortHeavy,
        LongShortSkewBucket::Balanced,
        LongShortSkewBucket::LongHeavy,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_surface_contains_every_niche_once() {
        let bank = Databank {
            schema_version: quantforge_discover::DATABANK_SCHEMA_VERSION,
            grammar_version: quantforge_discover::GRAMMAR_VERSION.into(),
            data_hash: ContentHash::sha256("data"),
            execution_data_hash: ContentHash::sha256("m1-data"),
            broker_spec_hash: ContentHash::sha256("broker"),
            config: Default::default(),
            completed_generations: 0,
            evaluation_count: 0,
            elites: Vec::new(),
            coverage_map: BTreeMap::new(),
            accepted_pool: Vec::new(),
            accepted_coverage_map: BTreeMap::new(),
            telemetry: Default::default(),
        };
        let groups = coverage_condition_groups(&bank);
        assert_eq!(groups.len(), ENTRY_CONDITION_COUNTS.len());
        assert_eq!(
            groups.iter().map(|group| group.total).sum::<usize>(),
            TOTAL_NICHES
        );
        assert!(groups.iter().all(|group| group.occupied == 0));
    }

    #[test]
    fn selection_bias_warning_is_unavoidable_and_thresholded() {
        assert_eq!(selection_bias(1_500).level, "recorded");
        assert_eq!(selection_bias(1_501).level, "elevated");
        assert_eq!(selection_bias(10_001).level, "high");
        assert_eq!(selection_bias(10_001).evaluation_count, 10_001);
    }

    #[test]
    fn robustness_depth_preserves_standard_and_only_raises_deep_compute() {
        let config = DiscoverConfig {
            robustness_folds: 3,
            robustness_monte_carlo_trials: 250,
            robustness_neighborhood_samples: 8,
            ..Default::default()
        };
        assert_eq!(
            robustness_depth(&config, ResultsRobustnessMode::Standard),
            (3, 250, 8)
        );
        assert_eq!(
            robustness_depth(&config, ResultsRobustnessMode::Deep),
            (5, 1_000, 20)
        );
    }

    #[test]
    fn results_robustness_recovers_only_the_hash_bound_is_partition() {
        let bars: Vec<_> = (0..20)
            .map(|index| quantforge_data::Bar {
                timestamp_ms: index * 3_600_000,
                open: 1.0,
                high: 1.1,
                low: 0.9,
                close: 1.0,
                tick_volume: 60,
                real_volume: 0,
                spread_points: Some(10),
            })
            .collect();
        let full = BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: b'\t' as char,
            source_timezone: "Etc/UTC".into(),
            bars,
        };
        let split = DataSplitPlan::chronological(&full, 0.2, 0.2).expect("split");
        let expected = bar_content_hash(&full.bars[..split.development.bar_count]);
        let recovered =
            databank_decision_partition(&full, &expected, 0.2, 0.2).expect("matching IS");
        assert_eq!(recovered.data_hash, expected);
        assert_eq!(recovered.bars.len(), split.development.bar_count);
        assert!(
            databank_decision_partition(&full, &ContentHash::sha256("other"), 0.2, 0.2)
                .is_err()
        );
    }

    #[test]
    fn state_refuses_elite_lookup_before_a_databank_is_loaded() {
        let state = DesktopState::default();
        assert!(matches!(
            get_elite_from_state("missing", &state),
            Err(DesktopError::NoDatabank)
        ));
    }

    #[test]
    fn batch_export_file_stems_are_portable_and_non_empty() {
        assert_eq!(safe_file_stem("trend/AUDUSD:one"), "trend_AUDUSD_one");
        assert_eq!(safe_file_stem(""), "strategy");
    }

    #[test]
    fn companion_metadata_uses_the_exporter_naming_convention() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let data = directory.path().join("AUDUSD_H1.tsv");
        let metadata = directory.path().join("AUDUSD_H1.metadata.csv");
        fs::write(&metadata, "key,value\n").expect("metadata fixture");
        assert_eq!(
            companion_metadata_path(data.to_str().expect("UTF-8 path")),
            Some(canonical_display(&metadata))
        );
    }

    #[test]
    fn archived_signature_provides_a_sharpe_fallback() {
        assert!(signature_sharpe(&[1.0, 2.0, 1.5]).is_some_and(|value| value > 0.0));
        assert_eq!(signature_sharpe(&[1.0]), None);
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityDemoRequest {
    databank_path: String,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    /// Where to write the filtered promotion-grade bank (only passers).
    output_path: String,
    /// SQX-style net/return retention vs H1 (default 0.80).
    return_retention: Option<f64>,
    /// SQX-style trade-count retention (default 0.80).
    trade_retention: Option<f64>,
    /// Max M1 DD as multiple of H1 DD (default 1.30).
    drawdown_expansion: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityEliteResult {
    fingerprint: String,
    strategy_id: String,
    passed: bool,
    h1_return_percent: f64,
    m1_return_percent: f64,
    return_retention: f64,
    h1_trades: usize,
    m1_trades: usize,
    trade_retention: f64,
    h1_drawdown_percent: f64,
    m1_drawdown_percent: f64,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FidelityDemoView {
    evaluated: usize,
    passed: usize,
    failed: usize,
    output_path: Option<String>,
    results: Vec<FidelityEliteResult>,
}

/// SQX RetestWithHigherPrecision analogue: retest H1-scout elites on M1 and keep survivors.
#[tauri::command]
pub async fn run_fidelity_demo(request: FidelityDemoRequest) -> Result<FidelityDemoView, String> {
    tauri::async_runtime::spawn_blocking(move || run_fidelity_demo_sync(&request))
        .await
        .map_err(|error| format!("fidelity demo task failed: {error}"))?
}

fn run_fidelity_demo_sync(request: &FidelityDemoRequest) -> Result<FidelityDemoView, String> {
    use crate::data_lab::{display_path, load_bound_broker, load_data_source, build_decision_from_m1};
    use quantforge_eval::evaluate_strategy;
    use quantforge_storage::{RunManifest, RunRecipe, write_json_new, write_json_versioned};
    use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};
    use serde_json::json;
    use std::collections::BTreeMap;

    let return_retention = request.return_retention.unwrap_or(0.90);
    let trade_retention = request.trade_retention.unwrap_or(0.80);
    let drawdown_expansion = request.drawdown_expansion.unwrap_or(1.30);

    let bytes = fs::read(&request.databank_path)
        .map_err(|error| format!("cannot read databank: {error}"))?;
    let artifact: EvolveArtifact = serde_json::from_slice(&bytes)
        .map_err(|error| format!("databank JSON is invalid: {error}"))?;
    verify_artifact(&artifact).map_err(|error| error.to_string())?;

    let h1_metadata = companion_metadata_path(&artifact.source);
    let h1 = load_data_source(
        &artifact.source,
        h1_metadata.as_deref(),
        if h1_metadata.is_some() {
            None
        } else {
            Some("Etc/UTC")
        },
    )?;
    let m1_metadata = request
        .m1_metadata_path
        .clone()
        .or_else(|| companion_metadata_path(&request.m1_data_path));
    let m1 = load_data_source(
        &request.m1_data_path,
        m1_metadata.as_deref(),
        if m1_metadata.is_some() {
            None
        } else {
            request.m1_source_timezone.as_deref().or(Some("Etc/UTC"))
        },
    )?;
    let broker = load_bound_broker(&artifact.broker, h1.metadata.as_ref())?;
    load_bound_broker(&artifact.broker, m1.metadata.as_ref())?;

    // SQX-style: decision OHLC is synthesized from M1 so aggregates always match.
    let decision = build_decision_from_m1(&m1.dataset, Some(&h1.dataset))?;

    let scout = &artifact.databank.config.scout;
    let judge = JudgeConfig {
        initial_balance: scout.initial_balance,
        costs: scout.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: scout.indicator_engine,
        entry_window: scout.entry_window,
    };

    let mut results = Vec::new();
    let mut keepers = Vec::new();
    for elite in &artifact.databank.elites {
        let h1_result = evaluate_strategy(&elite.strategy, &decision, &broker, scout)
            .map_err(|error| error.to_string());
        let m1_result = evaluate_strategy_m1(
            &elite.strategy,
            &decision,
            &m1.dataset,
            &broker,
            &judge,
        )
        .map_err(|error| error.to_string());

        let (passed, reason, row) = match (h1_result, m1_result) {
            (Ok(h1_eval), Ok(m1_eval)) => {
                let h1_ret = h1_eval.metrics.return_percent;
                let m1_ret = m1_eval.metrics.return_percent;
                let ret_ratio = if h1_ret > 0.0 {
                    m1_ret / h1_ret
                } else if m1_ret >= h1_ret {
                    1.0
                } else {
                    0.0
                };
                let h1_trades = h1_eval.metrics.trade_count;
                let m1_trades = m1_eval.metrics.trade_count;
                let trade_ratio = if h1_trades > 0 {
                    m1_trades as f64 / h1_trades as f64
                } else {
                    1.0
                };
                let h1_dd = h1_eval.metrics.max_drawdown_percent;
                let m1_dd = m1_eval.metrics.max_drawdown_percent;
                let dd_ok = m1_dd <= h1_dd * drawdown_expansion + 1.0e-9;
                let passed = ret_ratio >= return_retention && trade_ratio >= trade_retention && dd_ok;
                let reason = if passed {
                    "passed SQX-style M1 fidelity band".into()
                } else if !dd_ok {
                    format!("M1 DD {m1_dd:.2}% exceeds {drawdown_expansion:.2}× H1 DD")
                } else if ret_ratio < return_retention {
                    format!("return retention {ret_ratio:.2} < {return_retention:.2}")
                } else {
                    format!("trade retention {trade_ratio:.2} < {trade_retention:.2}")
                };
                (
                    passed,
                    reason,
                    FidelityEliteResult {
                        fingerprint: elite.structural_fingerprint.as_str().into(),
                        strategy_id: elite.strategy.id.clone(),
                        passed,
                        h1_return_percent: h1_ret,
                        m1_return_percent: m1_ret,
                        return_retention: ret_ratio,
                        h1_trades,
                        m1_trades,
                        trade_retention: trade_ratio,
                        h1_drawdown_percent: h1_dd,
                        m1_drawdown_percent: m1_dd,
                        reason: String::new(),
                    },
                )
            }
            (Err(error), _) | (_, Err(error)) => (
                false,
                error.clone(),
                FidelityEliteResult {
                    fingerprint: elite.structural_fingerprint.as_str().into(),
                    strategy_id: elite.strategy.id.clone(),
                    passed: false,
                    h1_return_percent: 0.0,
                    m1_return_percent: 0.0,
                    return_retention: 0.0,
                    h1_trades: 0,
                    m1_trades: 0,
                    trade_retention: 0.0,
                    h1_drawdown_percent: 0.0,
                    m1_drawdown_percent: 0.0,
                    reason: error,
                },
            ),
        };
        let mut row = row;
        row.reason = reason;
        if passed {
            keepers.push(elite.clone());
        }
        results.push(row);
    }

    let passed = results.iter().filter(|row| row.passed).count();
    let failed = results.len() - passed;
    let mut output_path = None;

    if !keepers.is_empty() {
        let mut bank = artifact.databank.clone();
        bank.elites = keepers;
        bank.coverage_map = BTreeMap::new();
        for elite in &bank.elites {
            bank.coverage_map.insert(
                niche_label(&elite.niche),
                elite.structural_fingerprint.clone(),
            );
        }
        bank.config.require_m1_precision = true;
        bank.validate_integrity()
            .map_err(|error| error.to_string())?;

        let mut config = artifact.manifest.recipe.config.clone();
        config.insert("research_grade".into(), json!(false));
        config.insert("m1_fidelity_verified".into(), json!(true));
        config.insert("require_m1_precision".into(), json!(true));
        config.insert(
            "fidelity_source_databank".into(),
            json!(display_path(Path::new(&request.databank_path))),
        );
        config.insert(
            "discover_config".into(),
            serde_json::to_value(&bank.config).map_err(|error| error.to_string())?,
        );

        let manifest = RunManifest::new(
            "evolve",
            RunRecipe {
                data_hash: Some(bank.data_hash.clone()),
                broker_spec_hash: Some(bank.broker_spec_hash.clone()),
                grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
                seed: Some(bank.config.seed),
                config,
                override_flags: Vec::new(),
            },
        )
        .map_err(|error| error.to_string())?;

        let filtered = EvolveArtifact {
            manifest,
            source: artifact.source.clone(),
            broker: artifact.broker.clone(),
            metadata_hash: artifact.metadata_hash.clone(),
            data_quality: artifact.data_quality.clone(),
            coverage: bank.coverage(),
            qd_score: bank.qd_score(),
            databank: bank,
        };

        if Path::new(&request.output_path).exists() {
            write_json_versioned(&request.output_path, &filtered)
                .map_err(|error| error.to_string())?;
        } else {
            write_json_new(&request.output_path, &filtered).map_err(|error| error.to_string())?;
        }
        output_path = Some(display_path(Path::new(&request.output_path)));
    }

    Ok(FidelityDemoView {
        evaluated: results.len(),
        passed,
        failed,
        output_path,
        results,
    })
}
