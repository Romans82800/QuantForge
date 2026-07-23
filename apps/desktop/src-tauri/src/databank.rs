use quantforge_core::ContentHash;
use quantforge_data::{DataQualityReport, QualityGrade};
use quantforge_discover::{
    Databank, Elite, FamilyStyle, LongShortSkewBucket, NicheKey, ThreeLevelBucket, niche_label,
};
use quantforge_storage::RunManifest;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tauri::State;
use thiserror::Error;

const TOTAL_NICHES: usize = 4 * 3usize.pow(5);
const SELECTION_BIAS_WARNING_THRESHOLD: u64 = 1_500;

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
    quality_grade: String,
    quality_score: u8,
    coverage: usize,
    total_niches: usize,
    qd_score: f64,
    completed_generations: u64,
    selection_bias: SelectionBiasView,
    rejections: RejectionTelemetry,
    families: Vec<FamilyCoverage>,
    elites: Vec<EliteRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchExportView {
    directory: String,
    index_path: String,
    strategy_paths: Vec<String>,
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
    clone: u64,
    correlated: u64,
    niche_not_improved: u64,
    precision: u64,
    evaluation: u64,
    total: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct FamilyCoverage {
    family: String,
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
    family: String,
    evidence: f64,
    novelty: f64,
    trades: usize,
    return_percent: f64,
    drawdown_percent: f64,
    profit_factor: Option<f64>,
    complexity: usize,
    generation: u64,
    grade: &'static str,
    parity: &'static str,
    equity_signature: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EliteDetail {
    fingerprint: String,
    strategy_id: String,
    thesis: String,
    family: String,
    niche: String,
    grade: &'static str,
    parity: &'static str,
    evidence: Value,
    descriptor: Value,
    metrics: Value,
    strategy_ir: Value,
    equity_signature: Vec<f64>,
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
    let artifact: EvolveArtifact = serde_json::from_slice(&bytes)?;
    verify_artifact(&artifact)?;
    let source_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let workspace = workspace_view(&artifact, &source_path, &artifact_hash);
    *state
        .loaded
        .write()
        .map_err(|_| DesktopError::StateUnavailable)? = Some(LoadedDatabank {
        bank: artifact.databank,
    });
    Ok(workspace)
}

#[tauri::command]
pub fn get_elite(
    fingerprint: String,
    state: State<'_, DesktopState>,
) -> Result<EliteDetail, String> {
    get_elite_from_state(&fingerprint, &state).map_err(|error| error.to_string())
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
    let elite = loaded
        .as_ref()
        .ok_or_else(|| DesktopError::NoDatabank.to_string())?
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
            "family": family_name(elite.niche.family),
            "return_percent": elite.metrics.return_percent,
            "profit_factor": elite.metrics.profit_factor,
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

fn workspace_view(
    artifact: &EvolveArtifact,
    source_path: &Path,
    artifact_hash: &ContentHash,
) -> DatabankWorkspace {
    let bank = &artifact.databank;
    let telemetry = &bank.telemetry;
    let total_rejections = telemetry.rejected_gate
        + telemetry.rejected_clone
        + telemetry.rejected_correlated
        + telemetry.rejected_niche_not_improved
        + telemetry.rejected_precision
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
        quality_grade: format!("{:?}", artifact.data_quality.grade).to_ascii_lowercase(),
        quality_score: artifact.data_quality.score,
        coverage: bank.coverage(),
        total_niches: TOTAL_NICHES,
        qd_score: bank.qd_score(),
        completed_generations: bank.completed_generations,
        selection_bias: selection_bias(bank.evaluation_count),
        rejections: RejectionTelemetry {
            gate: telemetry.rejected_gate,
            clone: telemetry.rejected_clone,
            correlated: telemetry.rejected_correlated,
            niche_not_improved: telemetry.rejected_niche_not_improved,
            precision: telemetry.rejected_precision,
            evaluation: telemetry.rejected_evaluation,
            total: total_rejections,
        },
        families: coverage_families(bank),
        elites: bank.elites.iter().map(elite_row).collect(),
    }
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
            "A large search space was touched. Treat raw backtest rankings as heavily selected and require the full Challenge, sealed-final and external-parity chain.",
        )
    } else if evaluation_count > SELECTION_BIAS_WARNING_THRESHOLD {
        (
            "elevated",
            "The search exceeded the default selection-bias warning threshold. Promotion must retain deflated metrics and robustness evidence.",
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
        family: family_name(elite.niche.family).into(),
        evidence: elite.evidence.total,
        novelty: elite.novelty,
        trades: elite.metrics.trade_count,
        return_percent: elite.metrics.return_percent,
        drawdown_percent: elite.metrics.max_drawdown_percent,
        profit_factor: elite.metrics.profit_factor,
        complexity: elite.complexity,
        generation: elite.discovered_generation,
        grade: "illuminated",
        parity: "unknown",
        equity_signature: elite.equity_signature.clone(),
    }
}

fn elite_detail(elite: &Elite) -> Result<EliteDetail, serde_json::Error> {
    Ok(EliteDetail {
        fingerprint: elite.structural_fingerprint.as_str().into(),
        strategy_id: elite.strategy.id.clone(),
        thesis: elite.strategy.meta.thesis_hint.clone(),
        family: family_name(elite.niche.family).into(),
        niche: niche_label(&elite.niche),
        grade: "illuminated",
        parity: "unknown",
        evidence: serde_json::to_value(&elite.evidence)?,
        descriptor: serde_json::to_value(&elite.descriptor)?,
        metrics: serde_json::to_value(&elite.metrics)?,
        strategy_ir: serde_json::to_value(&elite.strategy)?,
        equity_signature: elite.equity_signature.clone(),
    })
}

fn coverage_families(bank: &Databank) -> Vec<FamilyCoverage> {
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
    [
        FamilyStyle::Trend,
        FamilyStyle::Momentum,
        FamilyStyle::Breakout,
        FamilyStyle::MeanReversion,
    ]
    .into_iter()
    .map(|family| {
        let mut cells = Vec::with_capacity(3usize.pow(5));
        for win_rate in three_levels() {
            for skew in skew_levels() {
                for trade_frequency in three_levels() {
                    for hold_time in three_levels() {
                        for drawdown in three_levels() {
                            let niche = NicheKey {
                                family,
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
        FamilyCoverage {
            family: family_name(family).into(),
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

fn family_name(family: FamilyStyle) -> &'static str {
    match family {
        FamilyStyle::Trend => "trend",
        FamilyStyle::Momentum => "momentum",
        FamilyStyle::Breakout => "breakout",
        FamilyStyle::MeanReversion => "mean reversion",
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
            telemetry: Default::default(),
        };
        let families = coverage_families(&bank);
        assert_eq!(families.len(), 4);
        assert_eq!(
            families.iter().map(|family| family.total).sum::<usize>(),
            TOTAL_NICHES
        );
        assert!(families.iter().all(|family| family.occupied == 0));
    }

    #[test]
    fn selection_bias_warning_is_unavoidable_and_thresholded() {
        assert_eq!(selection_bias(1_500).level, "recorded");
        assert_eq!(selection_bias(1_501).level, "elevated");
        assert_eq!(selection_bias(10_001).level, "high");
        assert_eq!(selection_bias(10_001).evaluation_count, 10_001);
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
}
