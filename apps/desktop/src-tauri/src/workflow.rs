use quantforge_broker::SymbolSpecification;
use quantforge_core::ContentHash;
use quantforge_data::{DataQualityReport, QualityGrade};
use quantforge_discover::Databank;
use quantforge_export_mql5::ExportEvidenceCard;
use quantforge_ir::StrategyIr;
use quantforge_parity::{DiffReport, Mt5TesterMetadata, ParityRun};
use quantforge_quality::{
    CertificationEvidence, ChallengeReport, DataSplitPlan, EvidenceBinding, IncubationReport,
    IncubationStart, SealedFinalReport, ValidationAttestation,
};
use quantforge_storage::{RunManifest, RunRecipe};
use quantforge_tick::JudgeResult;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SplitPlanArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) source: String,
    pub(crate) metadata_hash: Option<ContentHash>,
    pub(crate) data_quality: DataQualityReport,
    pub(crate) validation_fraction: f64,
    pub(crate) sealed_fraction: f64,
    pub(crate) plan: DataSplitPlan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ChallengeArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) source: String,
    pub(crate) metadata_hash: Option<ContentHash>,
    pub(crate) data_quality: DataQualityReport,
    pub(crate) strategy_source: String,
    pub(crate) broker_source: String,
    pub(crate) split_plan_source: String,
    pub(crate) report: ChallengeReport,
}

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ScoutArtifactInput {
    pub(crate) manifest: RunManifest,
    pub(crate) strategy_fingerprint: ContentHash,
    pub(crate) result: quantforge_eval::ScoutResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct JudgeArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) strategy_fingerprint: ContentHash,
    pub(crate) decision_source: String,
    pub(crate) m1_source: String,
    pub(crate) strategy: String,
    pub(crate) broker: String,
    pub(crate) decision_metadata_hash: Option<ContentHash>,
    pub(crate) m1_metadata_hash: Option<ContentHash>,
    pub(crate) decision_data_quality: DataQualityReport,
    pub(crate) m1_data_quality: DataQualityReport,
    pub(crate) result: JudgeResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ParityArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) evidence: ExportEvidenceCard,
    pub(crate) reference: ParityRun,
    pub(crate) external: ParityRun,
    pub(crate) mt5_metadata: Mt5TesterMetadata,
    pub(crate) report: DiffReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IndicatorParityArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) report: quantforge_parity::IndicatorParityReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ValidationArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) challenge_source: String,
    pub(crate) attestation: ValidationAttestation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SealedFinalArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) source: String,
    pub(crate) metadata_hash: Option<ContentHash>,
    pub(crate) data_quality: DataQualityReport,
    pub(crate) strategy_source: String,
    pub(crate) broker_source: String,
    pub(crate) split_plan_source: String,
    pub(crate) challenge_source: String,
    pub(crate) report: SealedFinalReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IncubationStartArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) strategy_source: String,
    pub(crate) broker_source: String,
    pub(crate) split_plan_source: String,
    pub(crate) start: IncubationStart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IncubationObservationArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) start_source: String,
    pub(crate) start_artifact_hash: ContentHash,
    pub(crate) observation: quantforge_quality::IncubationObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IncubationFinalArtifact {
    pub(crate) manifest: RunManifest,
    pub(crate) start_source: String,
    pub(crate) observation_sources: Vec<String>,
    pub(crate) start: IncubationStart,
    pub(crate) observations: Vec<quantforge_quality::IncubationObservation>,
    pub(crate) report: IncubationReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct VaultArtifactReference {
    pub(crate) gate: String,
    pub(crate) path: String,
    pub(crate) content_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct VaultPayload {
    pub(crate) manifest: RunManifest,
    pub(crate) strategy_source: String,
    pub(crate) strategy_source_hash: ContentHash,
    pub(crate) strategy: StrategyIr,
    pub(crate) broker_source: String,
    pub(crate) broker_source_hash: ContentHash,
    pub(crate) broker: SymbolSpecification,
    pub(crate) split_plan_source: String,
    pub(crate) split_plan_source_hash: ContentHash,
    pub(crate) split_plan: DataSplitPlan,
    pub(crate) evidence_source: String,
    pub(crate) evidence_source_hash: ContentHash,
    pub(crate) evidence: CertificationEvidence,
    pub(crate) artifacts: Vec<VaultArtifactReference>,
}

pub(crate) fn read_json<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T, String> {
    let path = path.as_ref();
    serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))
}

pub(crate) fn read_json_hashed<T: DeserializeOwned>(
    path: impl AsRef<Path>,
) -> Result<(T, ContentHash), String> {
    let path = path.as_ref();
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not valid JSON: {error}", path.display()))?;
    Ok((value, ContentHash::sha256(&bytes)))
}

pub(crate) fn content_hash(path: impl AsRef<Path>) -> Result<ContentHash, String> {
    let path = path.as_ref();
    fs::read(path)
        .map(|bytes| ContentHash::sha256(&bytes))
        .map_err(|error| format!("cannot hash {}: {error}", path.display()))
}

pub(crate) fn write_json_new(path: impl AsRef<Path>, value: &impl Serialize) -> Result<(), String> {
    quantforge_storage::write_json_new(path, value).map_err(|error| error.to_string())
}

pub(crate) fn write_text_new(path: impl AsRef<Path>, value: &str) -> Result<(), String> {
    quantforge_storage::write_text_new(path, value).map_err(|error| error.to_string())
}

pub(crate) fn manifest(
    command: &str,
    data_hash: Option<ContentHash>,
    broker_spec_hash: Option<ContentHash>,
    grammar_version: Option<String>,
    seed: Option<u64>,
    config: BTreeMap<String, Value>,
) -> Result<RunManifest, String> {
    RunManifest::new(
        command,
        RunRecipe {
            data_hash,
            broker_spec_hash,
            grammar_version,
            seed,
            config,
            override_flags: Vec::new(),
        },
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn verify_split(artifact: &SplitPlanArtifact) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .plan
        .validate()
        .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "split-plan"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&artifact.plan.full_data_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
    {
        return Err("split plan is not an intact promotion-grade artifact".into());
    }
    Ok(())
}

pub(crate) fn binding(
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
) -> Result<EvidenceBinding, String> {
    Ok(EvidenceBinding {
        strategy_fingerprint: strategy
            .structural_fingerprint(quantforge_core::FloatPolicy::default())
            .map_err(|error| error.to_string())?,
        broker_spec_hash: broker.content_hash().map_err(|error| error.to_string())?,
    })
}

pub(crate) fn recipe_path(path: impl AsRef<Path>) -> Value {
    json!(crate::data_lab::display_path(path.as_ref()))
}

pub(crate) fn ensure_new(path: impl AsRef<Path>, label: &str) -> Result<PathBuf, String> {
    let path = path.as_ref().to_path_buf();
    if path.exists() {
        return Err(format!(
            "{label} already exists and will not be replaced: {}",
            path.display()
        ));
    }
    Ok(path)
}
