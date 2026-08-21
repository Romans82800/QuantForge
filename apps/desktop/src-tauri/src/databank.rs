use crate::data_lab::{
    build_decision_from_m1, build_decision_from_m1_quotes, load_bound_broker, load_quote_sidecar,
    trim_market_history_to_year,
};
use quantforge_broker::{BrokerClock, SymbolSpecification};
use quantforge_core::{ContentHash, FloatPolicy};
use quantforge_data::{BarDataset, DataQualityReport, QualityGrade, bar_content_hash};
use quantforge_discover::{
    Databank, DiscoverConfig, Elite, LongShortSkewBucket, NicheKey, RobustnessConfig,
    RobustnessEvidence, RobustnessReject, ThreeLevelBucket, niche_label,
    run_m1_predeposit_robustness,
};
use quantforge_eval::evaluate_strategy;
use quantforge_export_mql5::{Mql5ExportConfig, TesterConfig, generate_bundle};
use quantforge_ir::{BoolExpr, RiskPolicy, StrategyIr};
use quantforge_quality::DataSplitPlan;
use quantforge_storage::{RunManifest, RunRecipe, write_json_new, write_text_new};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tauri::State;
use thiserror::Error;

/// Entry-condition counts the grammar can emit; the first MAP-Elites axis.
const ENTRY_CONDITION_COUNTS: [usize; 3] = [2, 3, 4];
const TOTAL_NICHES: usize = ENTRY_CONDITION_COUNTS.len() * 3usize.pow(5);
const LEGACY_TOTAL_NICHES: usize = 10 * 3usize.pow(5);
const SELECTION_BIAS_WARNING_THRESHOLD: u64 = 1_500;
const LEGACY_DATABANK_SCHEMA_VERSION: u16 = 5;
const LEGACY_GRAMMAR_VERSION: &str = "search-families-v5-selected-tf-parity";
/// A full M1 replay retains an equity path as well as its trades.  Running an
/// arbitrarily large selection through Rayon and collecting every result at
/// once can therefore consume several gigabytes before the first CSV is
/// written.  Keep a small amount of CPU parallelism while bounding peak memory.
const TRADE_CSV_REPLAY_BATCH_SIZE: usize = 4;

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
pub(crate) struct LoadedDatabank {
    pub(crate) bank: Databank,
    pub(crate) legacy_read_only: bool,
    pub(crate) databank_path: String,
    pub(crate) source: String,
    pub(crate) broker: String,
    pub(crate) metadata_path: Option<String>,
    pub(crate) m1_source: Option<String>,
    pub(crate) m1_metadata_path: Option<String>,
    pub(crate) validation_fraction: f64,
    pub(crate) sealed_fraction: f64,
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

#[derive(Clone, Default)]
pub struct DesktopState {
    pub(crate) loaded: Arc<RwLock<Option<LoadedDatabank>>>,
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
    max_one_entry_per_day: bool,
    validation_fraction: f64,
    sealed_fraction: f64,
    condition_groups: Vec<ConditionCoverage>,
    elites: Vec<EliteRow>,
    #[serde(default)]
    holding: Vec<EliteRow>,
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

/// Batch of SQX-compatible trade ledgers. Each selected strategy is replayed
/// through the same M1 judge used by Results before its CSV is written.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTradeCsvExportView {
    directory: String,
    index_path: String,
    csv_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct TradeCsvExportSnapshot {
    elites: Vec<Elite>,
    source: String,
    broker: String,
    metadata_path: Option<String>,
    m1_source: String,
    m1_metadata_path: Option<String>,
    scout: quantforge_eval::ScoutConfig,
    validation_fraction: f64,
    sealed_fraction: f64,
    data_hash: ContentHash,
    execution_data_hash: ContentHash,
    broker_spec_hash: ContentHash,
    history_start_year: u16,
}

struct TradeCsvReplayContext {
    decision: BarDataset,
    m1: BarDataset,
    quotes: Option<quantforge_data::QuoteBarDataset>,
    broker: SymbolSpecification,
    judge: quantforge_tick::JudgeConfig,
    plan: DataSplitPlan,
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
    #[serde(default)]
    family_not_improved: u64,
    precision: u64,
    #[serde(default)]
    ambiguous: u64,
    oos1: u64,
    development_expectancy: u64,
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
    expectancy_r: f64,
    median_r: f64,
    fold_median_r: f64,
    fold_spread: f64,
    fold_count: usize,
    fold_usable: bool,
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
    oos1_expectancy: Option<f64>,
    oos1_expectancy_ratio: Option<f64>,
    fold_median_r: f64,
    fold_spread: f64,
    fold_pooled_r: f64,
    fold_count: usize,
    fold_usable: bool,
    strategy_ir: Value,
    equity_signature: Vec<f64>,
    /// Production Lane entries must not trigger the normal full-history chart,
    /// because that chart includes the sealed final partition.
    sealed_protected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    robustness: Option<EliteRobustnessView>,
}

/// Deterministic, in-memory preview of the exact QuantForge-native expert that
/// batch export would write for an elite. Keeping this generation server-side
/// means the Databank viewer and exported file cannot silently drift.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EliteMql5SourceView {
    fingerprint: String,
    expert_name: String,
    timeframe: String,
    export_style: &'static str,
    source_hash: String,
    source: String,
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

pub(crate) fn reload_workspace_from_path(
    path: &Path,
    state: &DesktopState,
) -> Result<DatabankWorkspace, String> {
    load_databank_path(path, state).map_err(|error| error.to_string())
}

fn load_databank_path(
    path: &Path,
    state: &DesktopState,
) -> Result<DatabankWorkspace, DesktopError> {
    let bytes = fs::read(path)?;
    let artifact_hash = ContentHash::sha256(&bytes);
    let (artifact, legacy_read_only) = parse_evolve_artifact(&bytes)?;
    let source_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    install_databank_artifact(
        artifact,
        legacy_read_only,
        source_path,
        artifact_hash,
        state,
    )
}

/// Installs an already-verified in-memory Discover artifact into the regular
/// Databank state. Live Discover uses this path so its table can refresh from
/// RAM without forcing a multi-megabyte recovery-checkpoint write for every
/// promoted strategy.
pub(crate) fn install_live_databank_artifact(
    artifact: EvolveArtifact,
    source_path: PathBuf,
    state: &DesktopState,
) -> Result<DatabankWorkspace, DesktopError> {
    verify_artifact(&artifact)?;
    let bytes = serde_json::to_vec(&artifact)?;
    install_databank_artifact(
        artifact,
        false,
        source_path,
        ContentHash::sha256(&bytes),
        state,
    )
}

fn install_databank_artifact(
    artifact: EvolveArtifact,
    legacy_read_only: bool,
    source_path: PathBuf,
    artifact_hash: ContentHash,
    state: &DesktopState,
) -> Result<DatabankWorkspace, DesktopError> {
    let mut workspace = workspace_view(&artifact, &source_path, &artifact_hash);
    // Archives are often copied from a Windows VPS. Preserve the signed/hash-
    // bound artifact exactly as written, but recover unavailable absolute paths
    // by filename from a sibling market-data pack on this machine. Replay code
    // verifies the recovered files against the hashes sealed in the bank before
    // it exports anything.
    let source = resolve_portable_binding(&artifact.source, &source_path);
    let broker = resolve_portable_binding(&artifact.broker, &source_path);
    let m1_source = manifest_path(&artifact, "m1_source")
        .map(|path| resolve_portable_binding(&path, &source_path));
    let metadata_path = companion_metadata_path(&source);
    let m1_metadata_path = m1_source.as_deref().and_then(companion_metadata_path);
    workspace.data_path = source.clone();
    workspace.metadata_path = metadata_path.clone();
    workspace.m1_data_path = m1_source.clone();
    workspace.m1_metadata_path = m1_metadata_path.clone();
    workspace.broker_path = broker.clone();
    let validation_fraction = manifest_fraction(&artifact, "validation_fraction", 0.2);
    let sealed_fraction = manifest_fraction(&artifact, "sealed_fraction", 0.2);
    *state
        .loaded
        .write()
        .map_err(|_| DesktopError::StateUnavailable)? = Some(LoadedDatabank {
        bank: artifact.databank,
        legacy_read_only,
        databank_path: source_path.display().to_string(),
        source,
        broker,
        metadata_path,
        m1_source,
        m1_metadata_path,
        validation_fraction,
        sealed_fraction,
    });
    Ok(workspace)
}

fn resolve_portable_binding(stored: &str, archive_path: &Path) -> String {
    if Path::new(stored).is_file() {
        return stored.to_owned();
    }
    let normalized = stored.replace('\\', "/");
    let mut parts = normalized
        .split('/')
        .filter(|part| !part.is_empty() && *part != "?")
        .collect::<Vec<_>>();
    let Some(file_name) = parts.pop() else {
        return stored.to_owned();
    };
    let parent_name = parts.last().copied();
    for ancestor in archive_path.ancestors().skip(1).take(5) {
        let direct = ancestor.join(file_name);
        if direct.is_file() {
            return canonical_display(&direct);
        }
        if let Some(parent_name) = parent_name {
            let nested = ancestor.join(parent_name).join(file_name);
            if nested.is_file() {
                return canonical_display(&nested);
            }
        }
    }
    stored.to_owned()
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
    verify_legacy_raw_coverage(raw, "accepted_pool", "accepted_coverage_map", false)
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
            DesktopError::InvalidArtifact(format!("legacy databank is missing {entries_key}"))
        })?;
    let coverage = raw
        .pointer(&format!("/databank/{coverage_key}"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            DesktopError::InvalidArtifact(format!("legacy databank is missing {coverage_key}"))
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
            legacy_niche_label(entry.get("niche").ok_or_else(|| {
                DesktopError::InvalidArtifact(format!(
                    "legacy {entries_key} contains an entry without a niche"
                ))
            })?)?
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
            .ok_or_else(|| DesktopError::InvalidArtifact(format!("legacy niche is missing {name}")))
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
                DesktopError::InvalidArtifact(format!("legacy databank is missing {entries_key}"))
            })?;
        for entry in entries {
            let strategy: StrategyIr =
                serde_json::from_value(entry.get("strategy").cloned().ok_or_else(|| {
                    DesktopError::InvalidArtifact(format!(
                        "legacy {entries_key} contains an entry without a strategy"
                    ))
                })?)?;
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

fn rename_legacy_field(object: &mut serde_json::Map<String, Value>, legacy: &str, current: &str) {
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
pub fn get_elite_mql5_source(
    fingerprint: String,
    timeframe: String,
    magic: u64,
    state: State<'_, DesktopState>,
) -> Result<EliteMql5SourceView, String> {
    get_elite_mql5_source_from_state(&fingerprint, &timeframe, magic, &state)
        .map_err(|error| error.to_string())
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
        if elite_is_sealed_protected(&elite) {
            return Err(
                "This Production Lane strategy is sealed-protected. Use the explicit one-shot Sealed Final workflow after the shortlist is frozen."
                    .into(),
            );
        }
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
            loaded.bank.config.history_start_year,
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
            history_start_year,
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
            history_start_year,
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
    tauri::async_runtime::spawn_blocking(move || run_elite_robustness_sync(&request, &snapshot))
        .await
        .map_err(|error| format!("Results robustness task failed: {error}"))?
}

pub(crate) fn infer_quote_sidecar_path(m1_path: &str) -> Option<PathBuf> {
    let path = Path::new(m1_path);
    let stem = path.file_stem()?.to_str()?;
    let mut candidates = vec![path.with_file_name(format!("{stem}.quotes.csv"))];
    for suffix in ["_H1", "_M15"] {
        if let Some(base) = stem.strip_suffix(suffix) {
            candidates.push(path.with_file_name(format!("{base}_M1.quotes.csv")));
        }
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn metadata_is_canonical_bid_ask(metadata: Option<&quantforge_data::Mt5ExportMetadata>) -> bool {
    let Some(metadata) = metadata else {
        return false;
    };
    metadata.properties.get("price_basis").is_some_and(|value| {
        value.eq_ignore_ascii_case("bid") || value.eq_ignore_ascii_case("bid_ask")
    }) && metadata
        .properties
        .get("import_kind")
        .is_some_and(|value| value.to_ascii_lowercase().contains("bid_ask"))
}

fn run_elite_robustness_sync(
    request: &ResultsRobustnessRequest,
    snapshot: &RobustnessSnapshot,
) -> Result<ResultsRobustnessView, String> {
    let mut decision_source = crate::data_lab::load_data_source(
        &snapshot.source,
        snapshot.metadata_path.as_deref(),
        None,
    )?;
    let mut m1_source = crate::data_lab::load_data_source(
        &snapshot.m1_source,
        snapshot.m1_metadata_path.as_deref(),
        None,
    )?;
    let mut quote_dataset = infer_quote_sidecar_path(&snapshot.m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1_source.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut decision_source.dataset,
        &mut m1_source.dataset,
        quote_dataset.as_mut(),
        snapshot.config.history_start_year,
    )?;
    if let Some(quotes) = quote_dataset.as_ref() {
        quotes
            .validate_against(&m1_source.dataset)
            .map_err(|error| format!("quote sidecar does not match M1 data: {error}"))?;
    } else if metadata_is_canonical_bid_ask(m1_source.metadata.as_ref()) {
        return Err(
            "canonical bid/ask M1 metadata is present but its .quotes.csv sidecar was not found"
                .into(),
        );
    }
    let pending_entry = matches!(
        snapshot.elite.strategy.entry.order,
        quantforge_ir::EntryOrderPolicy::Stop { .. }
            | quantforge_ir::EntryOrderPolicy::Limit { .. }
    ) || snapshot.config.allow_stop_entries
        || snapshot.config.allow_limit_entries;
    if pending_entry && quote_dataset.is_none() {
        return Err(
            "stop/limit strategies require a bid/ask M1 quote sidecar for Results robustness \
             (re-import ticks with qf-import-market and install_icmarkets_pack.py)"
                .into(),
        );
    }
    let broker = load_bound_broker(&snapshot.broker, decision_source.metadata.as_ref())?;
    load_bound_broker(&snapshot.broker, m1_source.metadata.as_ref())?;

    // Reconstruct the exact Selected-TF candles from M1, then recover the same
    // IS partition that Discover hashed into the databank. This prevents a
    // Results retest from silently drifting onto full history or OOS1/OOS2.
    let full_decision = match quote_dataset.as_ref() {
        Some(quotes) => build_decision_from_m1_quotes(
            &m1_source.dataset,
            Some(&decision_source.dataset),
            quotes,
            broker.point,
        )?,
        None => build_decision_from_m1(&m1_source.dataset, Some(&decision_source.dataset))?,
    };
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
        monte_carlo_block_length: snapshot.config.robustness_monte_carlo_block_length,
        monte_carlo_skip_trade_probability: snapshot
            .config
            .robustness_monte_carlo_skip_trade_probability,
        monte_carlo_minimum_p80_profit_retention: snapshot
            .config
            .robustness_monte_carlo_p80_profit_retention,
        monte_carlo_max_drawdown_ratio: snapshot.config.robustness_monte_carlo_max_drawdown_ratio,
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
        quote_dataset.as_ref(),
        &broker,
        &config,
        &selected_timeframe.metrics,
        true,
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
                    "monte_carlo_block_length".into(),
                    json!(config.monte_carlo_block_length),
                ),
                (
                    "monte_carlo_skip_trade_probability".into(),
                    json!(config.monte_carlo_skip_trade_probability),
                ),
                (
                    "monte_carlo_minimum_p80_profit_retention".into(),
                    json!(config.monte_carlo_minimum_p80_profit_retention),
                ),
                (
                    "monte_carlo_max_drawdown_ratio".into(),
                    json!(config.monte_carlo_max_drawdown_ratio),
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
    let split = DataSplitPlan::chronological(full_decision, validation_fraction, sealed_fraction)
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
        RobustnessReject::FoldStability => (
            "fold_stability",
            "Development calendar-year R failed pooled/median positivity, year concentration, or pooled-vs-median sanity.",
        ),
        RobustnessReject::Cpcv => (
            "cpcv",
            "Failed the purged combinatorial Development cross-validation requirement.",
        ),
        RobustnessReject::WalkForward => (
            "walk_forward",
            "Failed the sequential Development walk-forward stability requirement.",
        ),
        RobustnessReject::MonteCarlo => (
            "monte_carlo",
            "Failed the block-bootstrap Monte Carlo requirement (P80 net-profit retention vs baseline).",
        ),
        RobustnessReject::ParamNeighborhood => (
            "parameter_neighborhood",
            "Failed ±param survival or orig Ret/DD outside 0.85–1.25 of the neighbourhood median.",
        ),
    }
}

fn robustness_depth(config: &DiscoverConfig, mode: ResultsRobustnessMode) -> (usize, usize, usize) {
    match mode {
        ResultsRobustnessMode::Standard => (
            config.robustness_folds,
            config.robustness_monte_carlo_trials,
            config.robustness_neighborhood_samples,
        ),
        ResultsRobustnessMode::Deep => (
            config.robustness_folds.max(12),
            config.robustness_monte_carlo_trials.max(5_000),
            config.robustness_neighborhood_samples.max(400),
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
    history_start_year: u16,
) -> Result<PartitionEquityView, String> {
    let mut loaded = crate::data_lab::load_data_source(source, metadata_path, None)?;
    // Prefer full decision history. If the databank was built on an IS-only
    // slice whose path still points at full history, this is the right series.
    let broker = crate::data_lab::load_bound_broker(broker_path, loaded.metadata.as_ref())?;
    let m1_source = m1_source.ok_or_else(|| {
        "This legacy databank does not bind its M1 source; reopen it through Discover and run a new M1-verified search before using its full-run curve.".to_owned()
    })?;
    let mut m1 = crate::data_lab::load_data_source(m1_source, m1_metadata_path, None)?;
    crate::data_lab::load_bound_broker(broker_path, m1.metadata.as_ref())?;
    let mut quote_dataset = infer_quote_sidecar_path(m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut loaded.dataset,
        &mut m1.dataset,
        quote_dataset.as_mut(),
        history_start_year,
    )?;
    if let Some(quotes) = quote_dataset.as_ref() {
        quotes
            .validate_against(&m1.dataset)
            .map_err(|error| format!("quote sidecar does not match M1 data: {error}"))?;
    }
    // Match Discover/Parity Lab: decision OHLC is synthesized from M1 so aggregates
    // align with the exported EA and external MT5 backtests.
    let decision_dataset = match quote_dataset.as_ref() {
        Some(quotes) => {
            build_decision_from_m1_quotes(&m1.dataset, Some(&loaded.dataset), quotes, broker.point)?
        }
        None => build_decision_from_m1(&m1.dataset, Some(&loaded.dataset))?,
    };
    // Use the databank's sealed split, not a hardcoded 20/20. Discover gated this
    // elite on IS/OOS1 cut with these fractions; a mismatched chart invents a
    // different OOS1 window and a false retention ratio.
    let plan = quantforge_quality::DataSplitPlan::chronological(
        &decision_dataset,
        validation_fraction,
        sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let judge = quantforge_tick::JudgeConfig {
        initial_balance: scout.initial_balance,
        costs: scout.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: scout.indicator_engine,
        entry_window: scout.entry_window,
    };
    let result = match quote_dataset.as_ref() {
        Some(quotes) => quantforge_tick::evaluate_strategy_m1_with_quotes(
            &elite.strategy,
            &decision_dataset,
            &m1.dataset,
            quotes,
            &broker,
            &judge,
        ),
        None => quantforge_tick::evaluate_strategy_m1(
            &elite.strategy,
            &decision_dataset,
            &m1.dataset,
            &broker,
            &judge,
        ),
    }
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
        .filter(|trade| trade.entry_timestamp_ms >= oos1_end && trade.entry_timestamp_ms < oos2_end)
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
        is_return_percent: segment_return(
            &result.equity,
            scout.initial_balance,
            None,
            Some(is_end),
        ),
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

#[tauri::command]
pub async fn export_elite_trade_csvs(
    fingerprints: Vec<String>,
    directory: String,
    state: State<'_, DesktopState>,
) -> Result<BatchTradeCsvExportView, String> {
    let snapshot =
        trade_csv_export_snapshot(&fingerprints, &state).map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        export_elite_trade_csvs_to(snapshot, Path::new(&directory))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("trade CSV export task failed: {error}"))?
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
    quantforge_storage::write_json_new(&path, &elite.strategy)
        .map_err(|error| error.to_string())?;
    Ok(path.canonicalize().unwrap_or(path).display().to_string())
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingBatteryRequest {
    #[serde(default)]
    pub fingerprints: Vec<String>,
    /// Rank Holding by trades × R-expectancy instead of using `fingerprints`.
    #[serde(default)]
    pub ranked: bool,
    #[serde(default)]
    pub shrink_first: bool,
    #[serde(default)]
    pub max_correlation: Option<f64>,
    /// Ranked factory queue cap. `None` or `0` batteries everyone left after shrink.
    #[serde(default)]
    pub queue_limit: Option<usize>,
    /// Stop the battery once Databank reaches this many elites. `None` or `0` keeps every passer.
    #[serde(default)]
    pub target_databank: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingBatteryRejectRow {
    fingerprint: String,
    reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingBatteryView {
    promoted: usize,
    rejected: Vec<HoldingBatteryRejectRow>,
    workspace: DatabankWorkspace,
}

/// Run the deferred M1 battery + OOS1 on Holding candidates and promote passes
/// into Databank elites. Failures stay in Holding.
#[tauri::command]
pub async fn run_holding_battery(
    request: HoldingBatteryRequest,
    state: State<'_, DesktopState>,
) -> Result<HoldingBatteryView, String> {
    if request.fingerprints.is_empty() {
        return Err("select at least one Holding strategy".into());
    }
    let snapshot = {
        let loaded = state
            .loaded
            .read()
            .map_err(|_| DesktopError::StateUnavailable.to_string())?;
        let loaded = loaded
            .as_ref()
            .ok_or_else(|| DesktopError::NoDatabank.to_string())?;
        if loaded.legacy_read_only {
            return Err(
                "Schema-v5 databanks are read-only. Run a fresh Discover archive before Holding battery."
                    .into(),
            );
        }
        (
            loaded.bank.clone(),
            loaded.databank_path.clone(),
            loaded.source.clone(),
            loaded.broker.clone(),
            loaded.metadata_path.clone(),
            loaded.m1_source.clone().ok_or_else(|| {
                "This archive does not bind an M1 source; cannot run Holding battery.".to_owned()
            })?,
            loaded.m1_metadata_path.clone(),
            loaded.validation_fraction,
            loaded.sealed_fraction,
        )
    };
    let (
        mut bank,
        databank_path,
        source,
        broker_path,
        metadata_path,
        m1_source,
        m1_metadata_path,
        validation_fraction,
        sealed_fraction,
    ) = snapshot;
    let fingerprints = request.fingerprints.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        run_holding_battery_sync(
            &mut bank,
            &fingerprints,
            &source,
            metadata_path.as_deref(),
            &m1_source,
            m1_metadata_path.as_deref(),
            &broker_path,
            validation_fraction,
            sealed_fraction,
        )
        .map(|report| (bank, report))
    })
    .await
    .map_err(|error| format!("Holding battery task failed: {error}"))??;
    let (bank, report) = result;
    persist_loaded_bank(&databank_path, &bank, &state)?;
    let workspace =
        load_databank_path(Path::new(&databank_path), &state).map_err(|error| error.to_string())?;
    Ok(HoldingBatteryView {
        promoted: report.promoted,
        rejected: report.rejected,
        workspace,
    })
}

struct HoldingBatteryReport {
    promoted: usize,
    rejected: Vec<HoldingBatteryRejectRow>,
}

fn run_holding_battery_sync(
    bank: &mut Databank,
    fingerprints: &[String],
    source: &str,
    metadata_path: Option<&str>,
    m1_source: &str,
    m1_metadata_path: Option<&str>,
    broker_path: &str,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<HoldingBatteryReport, String> {
    let mut decision = crate::data_lab::load_data_source(source, metadata_path, None)?;
    let mut m1 = crate::data_lab::load_data_source(m1_source, m1_metadata_path, None)?;
    let broker = load_bound_broker(broker_path, decision.metadata.as_ref())?;
    load_bound_broker(broker_path, m1.metadata.as_ref())?;
    let mut quote_dataset = infer_quote_sidecar_path(m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut decision.dataset,
        &mut m1.dataset,
        quote_dataset.as_mut(),
        bank.config.history_start_year,
    )?;
    let plan =
        DataSplitPlan::chronological(&decision.dataset, validation_fraction, sealed_fraction)
            .map_err(|error| error.to_string())?;
    let development = slice_bars(&decision.dataset, 0, plan.development.bar_count)?;
    let oos1 = if plan.validation.bar_count == 0 {
        None
    } else {
        Some(slice_bars(
            &decision.dataset,
            plan.development.bar_count,
            plan.development.bar_count + plan.validation.bar_count,
        )?)
    };
    let m1_plan = DataSplitPlan::chronological(&m1.dataset, validation_fraction, sealed_fraction)
        .map_err(|error| error.to_string())?;
    let m1_development = slice_bars(&m1.dataset, 0, m1_plan.development.bar_count)?;
    let m1_eval = if bank.execution_data_hash == m1_development.data_hash {
        &m1_development
    } else if bank.execution_data_hash == m1.dataset.data_hash {
        &m1.dataset
    } else {
        &m1_development
    };

    let mut promoted = 0usize;
    let mut rejected = Vec::new();
    for fingerprint in fingerprints {
        let Some(target) = bank
            .holding
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            .map(|elite| elite.structural_fingerprint.clone())
        else {
            rejected.push(HoldingBatteryRejectRow {
                fingerprint: fingerprint.clone(),
                reason: "not in Holding".into(),
            });
            continue;
        };
        match quantforge_discover::run_holding_battery_and_promote(
            bank,
            &target,
            &development,
            oos1.as_ref(),
            m1_eval,
            quote_dataset.as_ref(),
            &broker,
        ) {
            Ok(_) => promoted += 1,
            Err(reason) => rejected.push(HoldingBatteryRejectRow {
                fingerprint: fingerprint.clone(),
                reason: format!("{reason:?}"),
            }),
        }
    }
    Ok(HoldingBatteryReport { promoted, rejected })
}

pub(crate) fn slice_bars(
    dataset: &quantforge_data::BarDataset,
    start: usize,
    end: usize,
) -> Result<quantforge_data::BarDataset, String> {
    if end > dataset.bars.len() || start > end {
        return Err(format!(
            "cannot slice bars [{start}..{end}) from {} bars",
            dataset.bars.len()
        ));
    }
    let bars = dataset.bars[start..end].to_vec();
    Ok(quantforge_data::BarDataset {
        data_hash: quantforge_data::bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: dataset.delimiter,
        source_timezone: dataset.source_timezone.clone(),
        bars,
    })
}

pub(crate) fn persist_bank_file(databank_path: &str, bank: &mut Databank) -> Result<(), String> {
    let path = Path::new(databank_path);
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let mut artifact: EvolveArtifact =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    persist_evolve_artifact(path, &mut artifact, bank)
}

/// Write `bank` into an already-parsed artifact. Battery must not re-read the
/// multi-MB checkpoint after every passer.
pub(crate) fn persist_evolve_artifact(
    path: &Path,
    artifact: &mut EvolveArtifact,
    bank: &mut Databank,
) -> Result<(), String> {
    std::mem::swap(&mut artifact.databank, bank);
    artifact.coverage = artifact.databank.coverage();
    artifact.qd_score = artifact.databank.qd_score();
    let written =
        quantforge_storage::write_json_replacing(path, artifact).map_err(|error| error.to_string());
    std::mem::swap(&mut artifact.databank, bank);
    written
}

pub(crate) fn persist_loaded_bank(
    databank_path: &str,
    bank: &Databank,
    state: &DesktopState,
) -> Result<(), String> {
    persist_bank_file(databank_path, &mut bank.clone())?;
    {
        let mut loaded = state
            .loaded
            .write()
            .map_err(|_| DesktopError::StateUnavailable.to_string())?;
        if let Some(current) = loaded.as_mut() {
            current.bank = bank.clone();
        }
    }
    Ok(())
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
    let mut planned = Vec::with_capacity(request.fingerprints.len());
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
            export_style: quantforge_export_mql5::ExportStyle::Quantforge,
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
        let source_path = directory.join(format!("{expert_name}.mq5"));
        if source_path.exists() {
            return Err(DesktopError::InvalidExport(format!(
                "{} already exists; rename or remove it, then export again",
                source_path.display()
            )));
        }
        planned.push((expert_name, bundle, source_path));
    }

    // Bulk strategy export is MQ5-only: no .set, tester.ini, evidence, IR, or
    // batch index. The folder may already contain other files.
    let mut expert_paths = Vec::with_capacity(planned.len());
    for (_, bundle, source) in &planned {
        write_text_new(source, &bundle.source)
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
        expert_paths.push(canonical_display(source));
    }

    Ok(BatchEaExportView {
        directory: canonical_display(directory),
        index_path: String::new(),
        expert_paths,
        settings_paths: Vec::new(),
        tester_paths: Vec::new(),
        evidence_paths: Vec::new(),
    })
}

fn trade_csv_export_snapshot(
    fingerprints: &[String],
    state: &DesktopState,
) -> Result<TradeCsvExportSnapshot, DesktopError> {
    if fingerprints.is_empty() {
        return Err(DesktopError::InvalidExport(
            "select at least one elite".into(),
        ));
    }
    let unique: BTreeSet<_> = fingerprints.iter().collect();
    if unique.len() != fingerprints.len() {
        return Err(DesktopError::InvalidExport(
            "the selection contains duplicate fingerprints".into(),
        ));
    }
    let loaded = state
        .loaded
        .read()
        .map_err(|_| DesktopError::StateUnavailable)?;
    let loaded = loaded.as_ref().ok_or(DesktopError::NoDatabank)?;
    let m1_source = loaded.m1_source.clone().ok_or_else(|| {
        DesktopError::InvalidExport(
            "this databank does not bind an M1 source; a full trade CSV requires the M1 judge replay"
                .into(),
        )
    })?;
    let mut elites = Vec::with_capacity(fingerprints.len());
    for fingerprint in fingerprints {
        elites.push(
            loaded
                .bank
                .elites
                .iter()
                .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
                .ok_or_else(|| DesktopError::MissingElite(fingerprint.clone()))?
                .clone(),
        );
    }
    Ok(TradeCsvExportSnapshot {
        elites,
        source: loaded.source.clone(),
        broker: loaded.broker.clone(),
        metadata_path: loaded.metadata_path.clone(),
        m1_source,
        m1_metadata_path: loaded.m1_metadata_path.clone(),
        scout: loaded.bank.config.scout.clone(),
        validation_fraction: loaded.validation_fraction,
        sealed_fraction: loaded.sealed_fraction,
        data_hash: loaded.bank.data_hash.clone(),
        execution_data_hash: loaded.bank.execution_data_hash.clone(),
        broker_spec_hash: loaded.bank.broker_spec_hash.clone(),
        history_start_year: loaded.bank.config.history_start_year,
    })
}

fn export_elite_trade_csvs_to(
    snapshot: TradeCsvExportSnapshot,
    directory: &Path,
) -> Result<BatchTradeCsvExportView, DesktopError> {
    if !directory.is_dir() {
        return Err(DesktopError::InvalidExport(format!(
            "{} is not an existing directory",
            directory.display()
        )));
    }
    let replay = prepare_trade_csv_replay(&snapshot)?;
    let broker = &replay.broker;
    let mut planned = Vec::with_capacity(snapshot.elites.len());
    let mut used_names = BTreeSet::new();
    for elite in &snapshot.elites {
        let fingerprint = elite.structural_fingerprint.as_str();
        let suffix = fingerprint.chars().take(8).collect::<String>();
        let preferred = format!(
            "{}_{}_{}",
            safe_file_stem(&broker.symbol),
            safe_file_stem(&elite.strategy.id),
            suffix
        );
        let stem = if used_names.insert(preferred.clone()) {
            preferred
        } else {
            format!("{preferred}_{}", planned.len() + 1)
        };
        planned.push((elite, directory.join(format!("{stem}.csv"))));
    }
    let index_path = directory.join("quantforge-strategy-csv-index.csv");
    if let Some(existing) = planned
        .iter()
        .map(|(_, path)| path)
        .chain(std::iter::once(&index_path))
        .find(|path| path.exists())
    {
        return Err(DesktopError::InvalidExport(format!(
            "{} already exists; rename or remove it, then export again",
            existing.display()
        )));
    }

    let mut rows = Vec::with_capacity(planned.len());
    let mut csv_paths = Vec::with_capacity(planned.len());
    for batch in planned.chunks(TRADE_CSV_REPLAY_BATCH_SIZE) {
        let completed = batch
            .par_iter()
            .map(|(elite, path)| {
                replay_elite_for_trade_csv(&replay, elite)
                    .map(|result| (*elite, path.clone(), result))
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (elite, path, result) in completed {
            write_sqx_style_trade_csv(
                &path,
                &elite.strategy.id,
                broker,
                snapshot.scout.initial_balance,
                &result.trades,
                replay.plan.development.end_timestamp_ms_exclusive,
                replay.plan.validation.end_timestamp_ms_exclusive,
            )?;
            rows.push((elite, path.clone(), result.metrics));
            csv_paths.push(canonical_display(&path));
            // `result.equity` is dropped here instead of being retained for the
            // entire selection.  Exporting 1,000 strategies is now bounded to
            // at most one small replay batch in memory.
        }
    }
    write_trade_csv_index(&index_path, &broker.symbol, &rows)?;

    Ok(BatchTradeCsvExportView {
        directory: canonical_display(directory),
        index_path: canonical_display(&index_path),
        csv_paths,
    })
}

fn prepare_trade_csv_replay(
    snapshot: &TradeCsvExportSnapshot,
) -> Result<TradeCsvReplayContext, DesktopError> {
    let mut decision = crate::data_lab::load_data_source(
        &snapshot.source,
        snapshot.metadata_path.as_deref(),
        None,
    )
    .map_err(DesktopError::InvalidExport)?;
    let mut m1 = crate::data_lab::load_data_source(
        &snapshot.m1_source,
        snapshot.m1_metadata_path.as_deref(),
        None,
    )
    .map_err(DesktopError::InvalidExport)?;
    let mut quote_dataset = infer_quote_sidecar_path(&snapshot.m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| {
            DesktopError::InvalidExport(format!("cannot load bid/ask quote sidecar: {error}"))
        })?;
    trim_market_history_to_year(
        &mut decision.dataset,
        &mut m1.dataset,
        quote_dataset.as_mut(),
        snapshot.history_start_year,
    )
    .map_err(DesktopError::InvalidExport)?;
    let broker = load_bound_broker(&snapshot.broker, m1.metadata.as_ref())
        .map_err(DesktopError::InvalidExport)?;
    if broker
        .content_hash()
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?
        != snapshot.broker_spec_hash
    {
        return Err(DesktopError::InvalidExport(
            "the recovered broker profile does not match this databank".into(),
        ));
    }
    if m1.dataset.data_hash != snapshot.execution_data_hash {
        return Err(DesktopError::InvalidExport(
            "the recovered M1 source does not match this databank".into(),
        ));
    }
    if let Some(quotes) = quote_dataset.as_ref() {
        quotes.validate_against(&m1.dataset).map_err(|error| {
            DesktopError::InvalidExport(format!("quote sidecar does not match M1 data: {error}"))
        })?;
    }
    let decision_dataset = match quote_dataset.as_ref() {
        Some(quotes) => build_decision_from_m1_quotes(
            &m1.dataset,
            Some(&decision.dataset),
            quotes,
            broker.point,
        ),
        None => build_decision_from_m1(&m1.dataset, Some(&decision.dataset)),
    }
    .map_err(DesktopError::InvalidExport)?;
    // The bank binds the Development partition rather than the full 60/20/20
    // history. Reconstruct that partition to prove the recovered decision grid
    // is identical before replaying the full history for CSV labels.
    databank_decision_partition(
        &decision_dataset,
        &snapshot.data_hash,
        snapshot.validation_fraction,
        snapshot.sealed_fraction,
    )
    .map_err(DesktopError::InvalidExport)?;
    let plan = DataSplitPlan::chronological(
        &decision_dataset,
        snapshot.validation_fraction,
        snapshot.sealed_fraction,
    )
    .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    let judge = quantforge_tick::JudgeConfig {
        initial_balance: snapshot.scout.initial_balance,
        costs: snapshot.scout.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: snapshot.scout.indicator_engine,
        entry_window: snapshot.scout.entry_window,
    };
    Ok(TradeCsvReplayContext {
        decision: decision_dataset,
        m1: m1.dataset,
        quotes: quote_dataset,
        broker,
        judge,
        plan,
    })
}

fn replay_elite_for_trade_csv(
    replay: &TradeCsvReplayContext,
    elite: &Elite,
) -> Result<quantforge_tick::JudgeResult, DesktopError> {
    match replay.quotes.as_ref() {
        Some(quotes) => quantforge_tick::evaluate_strategy_m1_with_quotes(
            &elite.strategy,
            &replay.decision,
            &replay.m1,
            quotes,
            &replay.broker,
            &replay.judge,
        ),
        None => quantforge_tick::evaluate_strategy_m1(
            &elite.strategy,
            &replay.decision,
            &replay.m1,
            &replay.broker,
            &replay.judge,
        ),
    }
    .map_err(|error| {
        DesktopError::InvalidExport(format!(
            "M1 trade replay failed for {}: {error}",
            elite.strategy.id
        ))
    })
}

const SQX_TRADE_HEADERS: [&str; 20] = [
    "Ticket",
    "Symbol",
    "Type",
    "Open time",
    "Open price",
    "Size",
    "Close time",
    "Close price",
    "Time in trade",
    "Profit/Loss",
    "Cummulative P/L",
    "Comm/Swap",
    "P/L in money",
    "Cummulative money P/L",
    "P/L in pips",
    "Cummulative pips P/L",
    "P/L in %",
    "Cummulative % P/L",
    "Comment",
    "Sample type",
];

fn write_sqx_style_trade_csv(
    path: &Path,
    strategy_id: &str,
    broker: &SymbolSpecification,
    initial_balance: f64,
    trades: &[quantforge_eval::Trade],
    is_end_timestamp_ms: i64,
    oos1_end_timestamp_ms: i64,
) -> Result<(), DesktopError> {
    let clock = BrokerClock::parse(&broker.timezone)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    let pip_size = if matches!(broker.digits, 3 | 5) {
        broker.point * 10.0
    } else {
        broker.point
    };
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_path(path)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    writer
        .write_record(SQX_TRADE_HEADERS)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    let mut cumulative_profit = 0.0;
    let mut cumulative_pips = 0.0;
    for (index, trade) in trades.iter().enumerate() {
        let direction = match trade.side {
            quantforge_eval::PositionSide::Long => 1.0,
            quantforge_eval::PositionSide::Short => -1.0,
        };
        let pips = if pip_size > 0.0 {
            (trade.exit_price - trade.entry_price) * direction / pip_size
        } else {
            0.0
        };
        let balance_before = initial_balance + cumulative_profit;
        cumulative_profit += trade.net_profit;
        cumulative_pips += pips;
        let trade_percent = if balance_before.abs() > 1.0e-12 {
            trade.net_profit / balance_before * 100.0
        } else {
            0.0
        };
        let cumulative_percent = if initial_balance.abs() > 1.0e-12 {
            cumulative_profit / initial_balance * 100.0
        } else {
            0.0
        };
        let comment = trade_exit_comment(trade, broker.digits, strategy_id);
        let sample = if trade.entry_timestamp_ms < is_end_timestamp_ms {
            "IS"
        } else if trade.entry_timestamp_ms < oos1_end_timestamp_ms {
            "OOS1"
        } else {
            "OOS2"
        };
        writer
            .write_record([
                ((index + 1) * 2).to_string(),
                broker.symbol.clone(),
                match trade.side {
                    quantforge_eval::PositionSide::Long => "Buy".into(),
                    quantforge_eval::PositionSide::Short => "Sell".into(),
                },
                format_sqx_timestamp(clock, trade.entry_timestamp_ms)?,
                format_price(trade.entry_price, broker.digits),
                format_decimal(trade.volume, 8),
                format_sqx_timestamp(clock, trade.exit_timestamp_ms)?,
                format_price(trade.exit_price, broker.digits),
                format_duration(trade.exit_timestamp_ms - trade.entry_timestamp_ms),
                format_decimal(trade.net_profit, 2),
                format_decimal(cumulative_profit, 2),
                format_decimal(-trade.commission + trade.swap, 2),
                format_decimal(trade.net_profit, 2),
                format_decimal(cumulative_profit, 2),
                format_decimal(pips, 2),
                format_decimal(cumulative_pips, 2),
                format_decimal(trade_percent, 2),
                format_decimal(cumulative_percent, 2),
                comment,
                sample.into(),
            ])
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    }
    writer
        .flush()
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    Ok(())
}

fn write_trade_csv_index(
    path: &Path,
    symbol: &str,
    rows: &[(&Elite, PathBuf, quantforge_eval::BacktestMetrics)],
) -> Result<(), DesktopError> {
    let mut writer = csv::WriterBuilder::new()
        .terminator(csv::Terminator::CRLF)
        .from_path(path)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    writer
        .write_record([
            "Strategy",
            "Fingerprint",
            "Symbol",
            "Trades",
            "Net profit",
            "Return %",
            "Max drawdown %",
            "Recovery factor",
            "Profit factor",
            "Sharpe ratio",
            "CSV path",
        ])
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    for (elite, csv_path, metrics) in rows {
        writer
            .write_record([
                elite.strategy.id.clone(),
                elite.structural_fingerprint.to_string(),
                symbol.to_owned(),
                metrics.trade_count.to_string(),
                format_decimal(metrics.net_profit, 2),
                format_decimal(metrics.return_percent, 4),
                format_decimal(metrics.max_drawdown_percent, 4),
                finite_recovery_factor(metrics)
                    .map(|value| format_decimal(value, 4))
                    .unwrap_or_default(),
                metrics
                    .profit_factor
                    .map(|value| format_decimal(value, 4))
                    .unwrap_or_default(),
                metrics
                    .sharpe_ratio
                    .map(|value| format_decimal(value, 4))
                    .unwrap_or_default(),
                canonical_display(csv_path),
            ])
            .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    }
    writer
        .flush()
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    Ok(())
}

fn format_sqx_timestamp(clock: BrokerClock, timestamp_ms: i64) -> Result<String, DesktopError> {
    clock
        .local_datetime(timestamp_ms)
        .map(|value| value.format("%d.%m.%Y %H:%M:%S").to_string())
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))
}

fn format_duration(milliseconds: i64) -> String {
    let total_seconds = milliseconds.max(0) / 1_000;
    let days = total_seconds / 86_400;
    let hours = total_seconds % 86_400 / 3_600;
    let minutes = total_seconds % 3_600 / 60;
    let seconds = total_seconds % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days}d"));
    }
    if hours > 0 {
        parts.push(format!("{hours}h"));
    }
    if minutes > 0 {
        parts.push(format!("{minutes}m"));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{seconds}s"));
    }
    parts.join(" ")
}

fn format_price(value: f64, digits: u8) -> String {
    format!("{value:.precision$}", precision = usize::from(digits))
}

fn format_decimal(value: f64, precision: usize) -> String {
    let formatted = format!("{value:.precision$}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn trade_exit_comment(trade: &quantforge_eval::Trade, digits: u8, strategy_id: &str) -> String {
    match trade.exit_reason {
        quantforge_eval::ExitReason::StopLoss => {
            format!("sl {}", format_price(trade.exit_price, digits))
        }
        quantforge_eval::ExitReason::TakeProfit => {
            format!("tp {}", format_price(trade.exit_price, digits))
        }
        quantforge_eval::ExitReason::Indicator => strategy_id.into(),
        quantforge_eval::ExitReason::TimeStop => "time exit".into(),
        quantforge_eval::ExitReason::EndOfDay => "end of day".into(),
        quantforge_eval::ExitReason::PartialExit => "partial exit".into(),
        quantforge_eval::ExitReason::EndOfData => "end of data".into(),
    }
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
    let elite = find_elite_in_bank(&loaded.bank, fingerprint)
        .ok_or_else(|| DesktopError::MissingElite(fingerprint.into()))?;
    elite_detail(elite).map_err(DesktopError::Json)
}

fn find_elite_in_bank<'a>(bank: &'a Databank, fingerprint: &str) -> Option<&'a Elite> {
    bank.elites
        .iter()
        .chain(bank.holding.iter())
        .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
}

fn get_elite_mql5_source_from_state(
    fingerprint: &str,
    timeframe: &str,
    magic: u64,
    state: &DesktopState,
) -> Result<EliteMql5SourceView, DesktopError> {
    if magic == 0 {
        return Err(DesktopError::InvalidExport(
            "magic must be greater than zero".into(),
        ));
    }
    let timeframe = timeframe.trim().to_ascii_uppercase();
    if !matches!(timeframe.as_str(), "M1" | "M15" | "H1") {
        return Err(DesktopError::InvalidExport(
            "source preview timeframe must be M1, M15 or H1".into(),
        ));
    }
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
    let broker = load_bound_broker(&loaded.broker, None).map_err(DesktopError::InvalidExport)?;
    let costs = &loaded.bank.config.scout.costs;
    let expert_name =
        quantforge_export_mql5::suggested_expert_name(&broker.symbol, &elite.strategy.id, magic);
    let config = Mql5ExportConfig {
        expert_name: expert_name.clone(),
        expert_directory: "QuantForge".into(),
        timeframe: timeframe.clone(),
        magic,
        deviation_points: 10,
        max_spread_points: costs.max_spread_points,
        estimated_slippage_points_per_side: costs.adverse_slippage_points_per_side,
        commission_per_lot_round_turn: costs.commission_per_lot_round_turn,
        allow_live_trading_default: false,
        export_style: quantforge_export_mql5::ExportStyle::Quantforge,
        entry_window_start_hour: loaded.bank.config.scout.entry_window.start_hour,
        entry_window_end_hour: loaded.bank.config.scout.entry_window.end_hour,
        tester: TesterConfig {
            deposit: loaded.bank.config.scout.initial_balance,
            model: 1,
            ..TesterConfig::default()
        },
    };
    let bundle = generate_bundle(&elite.strategy, &broker, &config)
        .map_err(|error| DesktopError::InvalidExport(error.to_string()))?;
    Ok(EliteMql5SourceView {
        fingerprint: fingerprint.into(),
        expert_name,
        timeframe,
        export_style: "quantforge-native-v1",
        source_hash: bundle.evidence.source_hash.to_string(),
        source: bundle.source,
    })
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
    verify_legacy_entries(&bank.elites, &bank.config, bank.completed_generations)?;
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
            || strategy_entry_condition_count(&elite.strategy) != elite.descriptor.entry_conditions
            || strategy_exit_condition_count(&elite.strategy) != elite.descriptor.exit_conditions
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

fn niche_from_descriptor(descriptor: &quantforge_discover::BehaviorDescriptor) -> NicheKey {
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
        + telemetry.rejected_family_not_improved
        + telemetry.rejected_precision
        + telemetry.rejected_ambiguous
        + telemetry.rejected_oos1
        + telemetry.rejected_development_expectancy
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
        // The databank is a diversified strategy stack, so its entry count is
        // not the same thing as behavioral coverage. Count distinct niche
        // labels for the coverage KPI and map.
        coverage: unique_niche_count(bank),
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
            family_not_improved: telemetry.rejected_family_not_improved,
            precision: telemetry.rejected_precision,
            ambiguous: telemetry.rejected_ambiguous,
            oos1: telemetry.rejected_oos1,
            development_expectancy: telemetry.rejected_development_expectancy,
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
        max_one_entry_per_day: bank.config.max_one_entry_per_day,
        validation_fraction: manifest_fraction(artifact, "validation_fraction", 0.2),
        sealed_fraction: manifest_fraction(artifact, "sealed_fraction", 0.2),
        condition_groups: coverage_condition_groups(bank),
        elites: bank.elites.iter().map(elite_row).collect(),
        holding: bank.holding.iter().map(elite_row).collect(),
    }
}

pub(crate) fn artifact_is_research_grade(artifact: &EvolveArtifact) -> bool {
    // Selected-TF Discover banks stay research-grade until the M1 fidelity
    // final gate stamps `m1_fidelity_verified`. Preference flags alone do not
    // promote a bank (SQX RetestWithHigherPrecision pattern).
    !artifact_m1_fidelity_verified(artifact)
}

pub(crate) fn artifact_m1_fidelity_verified(artifact: &EvolveArtifact) -> bool {
    matches!(
        artifact.manifest.recipe.config.get("m1_fidelity_verified"),
        Some(Value::Bool(true))
    )
}

pub(crate) fn manifest_path(artifact: &EvolveArtifact, key: &str) -> Option<String> {
    artifact
        .manifest
        .recipe
        .config
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub(crate) fn manifest_fraction(artifact: &EvolveArtifact, key: &str, fallback: f64) -> f64 {
    artifact
        .manifest
        .recipe
        .config
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0 && *value < 1.0)
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
        expectancy_r: elite.metrics.expectancy_r,
        median_r: elite.metrics.median_r,
        fold_median_r: elite.fold_r.median_fold_r,
        fold_spread: elite.fold_r.fold_spread,
        fold_count: elite.fold_r.fold_count,
        fold_usable: elite.fold_r.usable,
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
        oos1_expectancy: elite.oos1_expectancy,
        oos1_expectancy_ratio: elite.oos1_expectancy_ratio,
        fold_median_r: elite.fold_r.median_fold_r,
        fold_spread: elite.fold_r.fold_spread,
        fold_pooled_r: elite.fold_r.pooled_r,
        fold_count: elite.fold_r.fold_count,
        fold_usable: elite.fold_r.usable,
        strategy_ir: serde_json::to_value(&elite.strategy)?,
        equity_signature: elite.equity_signature.clone(),
        sealed_protected: elite_is_sealed_protected(elite),
        robustness: elite_robustness(elite)?,
    })
}

fn elite_is_sealed_protected(elite: &Elite) -> bool {
    elite
        .gate_results
        .iter()
        .any(|gate| gate.name == "production_lane_v1" && gate.passed)
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
    // Since v6 the persisted coverage_map is fingerprint-keyed because the
    // databank may retain multiple diverse structures in one behavioral niche.
    // Build the display index from Elite.niche and choose the strongest member
    // of each cell. Looking up niche labels in coverage_map leaves every cell
    // dark even though the archive contains promoted strategies.
    let mut elites_by_niche: BTreeMap<String, &Elite> = BTreeMap::new();
    for elite in &bank.elites {
        let label = niche_label(&elite.niche);
        match elites_by_niche.get(&label) {
            Some(current) if current.evidence.total >= elite.evidence.total => {}
            _ => {
                elites_by_niche.insert(label, elite);
            }
        }
    }
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
                                let elite = elites_by_niche.get(&label).copied();
                                cells.push(CoverageCell {
                                    index: cells.len(),
                                    niche: label,
                                    occupied: elite.is_some(),
                                    fingerprint: elite.map(|value| {
                                        value.structural_fingerprint.as_str().to_owned()
                                    }),
                                    intensity: elite
                                        .map(|value| {
                                            evidence_intensity(
                                                value.evidence.total,
                                                minimum,
                                                maximum,
                                            )
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

fn unique_niche_count(bank: &Databank) -> usize {
    bank.elites
        .iter()
        .map(|elite| niche_label(&elite.niche))
        .collect::<std::collections::BTreeSet<_>>()
        .len()
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

    fn sample_elite(sequence: u64, niche: NicheKey, evidence_total: f64) -> Elite {
        let strategy = quantforge_discover::generate_seed(42, sequence);
        let fingerprint = strategy
            .structural_fingerprint(quantforge_core::FloatPolicy::default())
            .expect("fingerprint");
        Elite {
            strategy,
            structural_fingerprint: fingerprint,
            descriptor: quantforge_discover::BehaviorDescriptor {
                entry_conditions: niche.entry_conditions,
                exit_conditions: 1,
                trades_per_1000_bars: 10.0,
                average_bars_held: 8.0,
                drawdown_percent: 8.0,
                win_rate_percent: 45.0,
                long_short_skew: 0.0,
            },
            niche,
            evidence: quantforge_discover::EvidenceComponents {
                return_component: evidence_total,
                profit_factor_component: 0.0,
                trade_count_bonus: 0.0,
                drawdown_penalty: 0.0,
                complexity_penalty: 0.0,
                total: evidence_total,
            },
            novelty: 0.0,
            complexity: 1,
            metrics: quantforge_eval::BacktestMetrics {
                initial_balance: 100_000.0,
                ending_balance: 101_000.0,
                net_profit: 1_000.0,
                return_percent: 1.0,
                trade_count: 10,
                winning_trades: 5,
                losing_trades: 5,
                win_rate: 50.0,
                profit_factor: Some(1.2),
                max_drawdown: 500.0,
                max_drawdown_percent: 0.5,
                sharpe_ratio: Some(1.0),
                expectancy: 100.0,
                expectancy_r: 0.0,
                median_r: 0.0,
            },
            is_expectancy: 0.0,
            oos1_expectancy: None,
            oos1_expectancy_ratio: None,
            fold_r: Default::default(),
            observed_trade_sharpe: None,
            expected_max_lucky_sharpe: None,
            deflated_trade_sharpe: None,
            multi_symbol_results: Vec::new(),
            gate_results: Vec::new(),
            robustness: None,
            equity_signature: vec![0.0, 1.0],
            discovered_generation: 1,
            island_id: 0,
        }
    }

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
            specialist_pool: Vec::new(),
            specialist_coverage_map: BTreeMap::new(),
            holding: Vec::new(),
            holding_coverage_map: BTreeMap::new(),
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
    fn coverage_surface_groups_stacked_strategies_by_actual_niche() {
        let niche = NicheKey {
            entry_conditions: 2,
            trade_frequency: ThreeLevelBucket::Medium,
            hold_time: ThreeLevelBucket::Medium,
            drawdown: ThreeLevelBucket::Medium,
            win_rate: ThreeLevelBucket::Medium,
            long_short_skew: LongShortSkewBucket::Balanced,
        };
        let weak = sample_elite(1, niche.clone(), 2.0);
        let strong = sample_elite(2, niche, 9.0);
        let strong_fingerprint = strong.structural_fingerprint.as_str().to_owned();
        let bank = Databank {
            schema_version: quantforge_discover::DATABANK_SCHEMA_VERSION,
            grammar_version: quantforge_discover::GRAMMAR_VERSION.into(),
            data_hash: ContentHash::sha256("data"),
            execution_data_hash: ContentHash::sha256("m1-data"),
            broker_spec_hash: ContentHash::sha256("broker"),
            config: Default::default(),
            completed_generations: 1,
            evaluation_count: 2,
            elites: vec![weak, strong],
            // v6 production archives are fingerprint-keyed, not niche-keyed.
            coverage_map: BTreeMap::new(),
            accepted_pool: Vec::new(),
            accepted_coverage_map: BTreeMap::new(),
            specialist_pool: Vec::new(),
            specialist_coverage_map: BTreeMap::new(),
            holding: Vec::new(),
            holding_coverage_map: BTreeMap::new(),
            telemetry: Default::default(),
        };

        assert_eq!(unique_niche_count(&bank), 1);
        let groups = coverage_condition_groups(&bank);
        let occupied: Vec<_> = groups
            .iter()
            .flat_map(|group| &group.cells)
            .filter(|cell| cell.occupied)
            .collect();
        assert_eq!(occupied.len(), 1);
        assert_eq!(
            occupied[0].fingerprint.as_deref(),
            Some(strong_fingerprint.as_str())
        );
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
            (12, 5_000, 400)
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
            databank_decision_partition(&full, &ContentHash::sha256("other"), 0.2, 0.2).is_err()
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
    fn portable_binding_recovers_a_windows_vps_path_without_editing_the_archive() {
        let root = tempfile::tempdir().expect("temporary directory");
        let pack = root.path().join("ICMarkets_EST7_2020_present");
        fs::create_dir(&pack).expect("market pack");
        let local = pack.join("BTCUSD_M1.tsv");
        fs::write(&local, "fixture").expect("local binding");
        let archive = root.path().join("runs").join("btc-databank.json");
        assert_eq!(
            resolve_portable_binding(
                r"\\?\C:\Users\Administrator\Documents\QuantForge\ICMarkets_EST7_2020_present\BTCUSD_M1.tsv",
                &archive,
            ),
            canonical_display(&local),
        );
    }

    #[test]
    fn sqx_trade_csv_matches_reference_columns_and_partition_labels() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("strategy.csv");
        let broker: SymbolSpecification = serde_json::from_str(include_str!(
            "../../../../fixtures/EURUSD_fixture_broker.json"
        ))
        .expect("broker fixture");
        let trades = vec![
            quantforge_eval::Trade {
                side: quantforge_eval::PositionSide::Long,
                entry_timestamp_ms: 1_704_067_200_000,
                exit_timestamp_ms: 1_704_070_900_000,
                entry_price: 1.10001,
                exit_price: 1.10101,
                volume: 1.25,
                initial_stop_loss: 1.09901,
                initial_take_profit: 1.10101,
                gross_profit: 125.0,
                commission: 8.75,
                swap: 0.0,
                net_profit: 116.25,
                bars_held: 1,
                exit_reason: quantforge_eval::ExitReason::TakeProfit,
                r_multiple: 0.0,
            },
            quantforge_eval::Trade {
                side: quantforge_eval::PositionSide::Short,
                entry_timestamp_ms: 1_704_074_400_000,
                exit_timestamp_ms: 1_704_078_000_000,
                entry_price: 1.10100,
                exit_price: 1.10200,
                volume: 1.0,
                initial_stop_loss: 1.10200,
                initial_take_profit: 1.09900,
                gross_profit: -100.0,
                commission: 7.0,
                swap: -1.0,
                net_profit: -108.0,
                bars_held: 1,
                exit_reason: quantforge_eval::ExitReason::StopLoss,
                r_multiple: 0.0,
            },
        ];
        write_sqx_style_trade_csv(
            &path,
            "test-strategy",
            &broker,
            100_000.0,
            &trades,
            1_704_074_400_000,
            1_704_100_000_000,
        )
        .expect("CSV export");
        let mut reader = csv::Reader::from_path(&path).expect("CSV reader");
        assert_eq!(reader.headers().expect("headers").len(), 20);
        assert_eq!(
            reader
                .headers()
                .expect("headers")
                .iter()
                .collect::<Vec<_>>(),
            SQX_TRADE_HEADERS
        );
        let rows = reader
            .records()
            .collect::<Result<Vec<_>, _>>()
            .expect("CSV rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(&rows[0][1], "EURUSD");
        assert_eq!(&rows[0][2], "Buy");
        assert_eq!(&rows[0][19], "IS");
        assert_eq!(&rows[1][2], "Sell");
        assert_eq!(&rows[1][19], "OOS1");
        assert!(rows[0][18].starts_with("tp "));
        assert!(rows[1][18].starts_with("sl "));
    }

    #[test]
    fn sqx_duration_and_decimal_formatting_are_compact() {
        assert_eq!(format_duration(18 * 3_600_000 + 45 * 60_000), "18h 45m");
        assert_eq!(format_duration(40_000), "40s");
        assert_eq!(format_decimal(7.20, 8), "7.2");
        assert_eq!(format_price(108.2, 3), "108.200");
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
    use crate::data_lab::{
        build_decision_from_m1, display_path, load_bound_broker, load_data_source,
        trim_market_history_to_year,
    };
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
    let mut h1 = load_data_source(
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
    let mut m1 = load_data_source(
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
    let mut quote_dataset = infer_quote_sidecar_path(&request.m1_data_path)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut h1.dataset,
        &mut m1.dataset,
        quote_dataset.as_mut(),
        artifact.databank.config.history_start_year,
    )?;
    if let Some(quotes) = quote_dataset.as_ref() {
        quotes
            .validate_against(&m1.dataset)
            .map_err(|error| format!("quote sidecar does not match M1 data: {error}"))?;
    }

    // Decision OHLC and spread are synthesized from the same M1 quote pack
    // used by the chronological judge.
    let decision = match quote_dataset.as_ref() {
        Some(quotes) => {
            build_decision_from_m1_quotes(&m1.dataset, Some(&h1.dataset), quotes, broker.point)?
        }
        None => build_decision_from_m1(&m1.dataset, Some(&h1.dataset))?,
    };

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
        let m1_result = match quote_dataset.as_ref() {
            Some(quotes) => quantforge_tick::evaluate_strategy_m1_with_quotes(
                &elite.strategy,
                &decision,
                &m1.dataset,
                quotes,
                &broker,
                &judge,
            ),
            None => evaluate_strategy_m1(&elite.strategy, &decision, &m1.dataset, &broker, &judge),
        }
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
                let passed =
                    ret_ratio >= return_retention && trade_ratio >= trade_retention && dd_ok;
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
