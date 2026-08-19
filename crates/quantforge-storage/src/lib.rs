//! Versioned artifact writes and immutable run manifests.

use chrono::{DateTime, SecondsFormat, Utc};
use quantforge_core::{
    ContentHash, HashError, MANIFEST_SCHEMA_VERSION, PRODUCT_NAME, stable_json_hash,
};
use quantforge_quality::{
    CertificationEvidence, CertificationPolicy, DataSplitPlan, EvidenceError, PromotionDecision,
    SealedFinalReport, StrategyGrade, evaluate_certification,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use tempfile::NamedTempFile;
use thiserror::Error;

pub const VAULT_SCHEMA_VERSION: u16 = 1;
pub const SEALED_ACCESS_SCHEMA_VERSION: u16 = 1;
pub const SEALED_BUDGET_SCHEMA_VERSION: u16 = 1;
/// Lifetime cap on sealed (OOS2) opens per split plan.
pub const DEFAULT_SEALED_ACCESS_BUDGET: u64 = 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecipe {
    pub data_hash: Option<ContentHash>,
    pub broker_spec_hash: Option<ContentHash>,
    pub grammar_version: Option<String>,
    pub seed: Option<u64>,
    pub config: BTreeMap<String, Value>,
    pub override_flags: Vec<String>,
}

impl RunRecipe {
    pub fn empty() -> Self {
        Self {
            data_hash: None,
            broker_spec_hash: None,
            grammar_version: None,
            seed: None,
            config: BTreeMap::new(),
            override_flags: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    pub schema_version: u16,
    pub run_id: String,
    pub created_at: DateTime<Utc>,
    pub product: String,
    pub product_version: String,
    pub command: String,
    pub recipe_hash: ContentHash,
    pub recipe: RunRecipe,
}

impl RunManifest {
    pub fn new(command: impl Into<String>, recipe: RunRecipe) -> Result<Self, StorageError> {
        let recipe_hash = stable_json_hash(&recipe)?;
        let created_at = Utc::now();
        let timestamp = created_at.format("%Y%m%dT%H%M%S%.6fZ");
        let run_id = format!("{timestamp}-{}", &recipe_hash.as_str()[..12]);

        Ok(Self {
            schema_version: MANIFEST_SCHEMA_VERSION,
            run_id,
            created_at,
            product: PRODUCT_NAME.into(),
            product_version: env!("CARGO_PKG_VERSION").into(),
            command: command.into(),
            recipe_hash,
            recipe,
        })
    }

    pub fn write_immutable(&self, path: impl AsRef<Path>) -> Result<(), StorageError> {
        write_json_new(path, self)
    }

    pub fn validate(&self) -> Result<(), StorageError> {
        if stable_json_hash(&self.recipe)? != self.recipe_hash {
            return Err(StorageError::ManifestIntegrity);
        }
        Ok(())
    }
}

/// A Certified-only, immutable Vault record. The payload is application-owned,
/// while the admission decision is always recomputed by this crate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CertifiedVaultEntry<T> {
    pub schema_version: u16,
    pub entry_id: ContentHash,
    pub admitted_at: DateTime<Utc>,
    pub strategy_fingerprint: ContentHash,
    pub payload_hash: ContentHash,
    pub certification: PromotionDecision,
    pub payload: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultAdmission {
    pub path: PathBuf,
    pub entry_id: ContentHash,
    pub decision: PromotionDecision,
}

#[derive(Debug, Serialize)]
struct VaultIdentity<'a> {
    schema_version: u16,
    strategy_fingerprint: &'a ContentHash,
    evidence_hash: &'a ContentHash,
}

/// Re-evaluates the complete certification gate and writes one immutable Vault
/// entry. A denied candidate never reaches the filesystem.
pub fn admit_certified<T>(
    vault_root: impl AsRef<Path>,
    evidence: &CertificationEvidence,
    split_plan: &DataSplitPlan,
    policy: &CertificationPolicy,
    payload: T,
) -> Result<VaultAdmission, StorageError>
where
    T: Serialize,
{
    let decision = evaluate_certification(evidence, split_plan, policy)?;
    if !decision.passed || decision.resulting_grade != StrategyGrade::Certified {
        return Err(StorageError::CertificationDenied(format!(
            "{:?}",
            decision.blockers
        )));
    }

    let payload_hash = stable_json_hash(&payload)?;
    let entry_id = stable_json_hash(&VaultIdentity {
        schema_version: VAULT_SCHEMA_VERSION,
        strategy_fingerprint: &decision.candidate.strategy_fingerprint,
        evidence_hash: &decision.evidence_hash,
    })?;
    let path = vault_root
        .as_ref()
        .join(decision.candidate.strategy_fingerprint.as_str())
        .join(format!("{}.certified.json", entry_id.as_str()));
    let entry = CertifiedVaultEntry {
        schema_version: VAULT_SCHEMA_VERSION,
        entry_id: entry_id.clone(),
        admitted_at: Utc::now(),
        strategy_fingerprint: decision.candidate.strategy_fingerprint.clone(),
        payload_hash,
        certification: decision.clone(),
        payload,
    };
    write_json_new(&path, &entry)?;
    Ok(VaultAdmission {
        path,
        entry_id,
        decision,
    })
}

/// Deterministic location of a strategy's single sealed-final attempt for one
/// split plan. Passing and failing attempts intentionally share this key.
pub fn sealed_final_path(
    root: impl AsRef<Path>,
    strategy_fingerprint: &ContentHash,
    split_plan_hash: &ContentHash,
) -> PathBuf {
    root.as_ref()
        .join(strategy_fingerprint.as_str())
        .join(format!("{}.sealed-final.json", split_plan_hash.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedAccessClaim {
    pub schema_version: u16,
    pub claimed_at: DateTime<Utc>,
    pub strategy_fingerprint: ContentHash,
    pub split_plan_hash: ContentHash,
    pub challenge_artifact_hash: ContentHash,
}

pub fn sealed_access_path(
    root: impl AsRef<Path>,
    strategy_fingerprint: &ContentHash,
    split_plan_hash: &ContentHash,
) -> PathBuf {
    root.as_ref()
        .join(strategy_fingerprint.as_str())
        .join(format!("{}.sealed-open.json", split_plan_hash.as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedBudgetEntry {
    pub strategy_fingerprint: ContentHash,
    pub challenge_artifact_hash: ContentHash,
    pub claimed_at: DateTime<Utc>,
}

/// Lifetime sealed-access budget for one split plan (default 20 opens).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedAccessBudgetLedger {
    pub schema_version: u16,
    pub split_plan_hash: ContentHash,
    pub lifetime_limit: u64,
    pub claims: Vec<SealedBudgetEntry>,
}

pub fn sealed_budget_path(root: impl AsRef<Path>, split_plan_hash: &ContentHash) -> PathBuf {
    root.as_ref()
        .join(".quantforge-budget")
        .join(format!("{}.sealed-budget.json", split_plan_hash.as_str()))
}

pub fn read_sealed_budget(
    root: impl AsRef<Path>,
    split_plan_hash: &ContentHash,
) -> Result<SealedAccessBudgetLedger, StorageError> {
    let path = sealed_budget_path(&root, split_plan_hash);
    if !path.is_file() {
        // Read the pre-v1 location so existing sealed roots remain usable, but
        // always write new ledgers into the private metadata directory. The
        // sealed root itself must contain candidate directories only.
        let legacy_path = root
            .as_ref()
            .join(format!("{}.sealed-budget.json", split_plan_hash.as_str()));
        if legacy_path.is_file() {
            let bytes = fs::read(legacy_path)?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
        return Ok(SealedAccessBudgetLedger {
            schema_version: SEALED_BUDGET_SCHEMA_VERSION,
            split_plan_hash: split_plan_hash.clone(),
            lifetime_limit: DEFAULT_SEALED_ACCESS_BUDGET,
            claims: Vec::new(),
        });
    }
    let bytes = fs::read(&path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Claims the one allowed access before any sealed market bars are loaded. If
/// evaluation crashes afterward, the durable claim still prevents a retry.
///
/// Also enforces the lifetime sealed-access budget for the split plan
/// ([`DEFAULT_SEALED_ACCESS_BUDGET`] opens).
pub fn claim_sealed_access_once(
    root: impl AsRef<Path>,
    strategy_fingerprint: &ContentHash,
    split_plan_hash: &ContentHash,
    challenge_artifact_hash: &ContentHash,
) -> Result<PathBuf, StorageError> {
    claim_sealed_access_once_with_budget(
        root,
        strategy_fingerprint,
        split_plan_hash,
        challenge_artifact_hash,
        DEFAULT_SEALED_ACCESS_BUDGET,
    )
}

pub fn claim_sealed_access_once_with_budget(
    root: impl AsRef<Path>,
    strategy_fingerprint: &ContentHash,
    split_plan_hash: &ContentHash,
    challenge_artifact_hash: &ContentHash,
    lifetime_limit: u64,
) -> Result<PathBuf, StorageError> {
    let root = root.as_ref();
    let mut ledger = read_sealed_budget(root, split_plan_hash)?;
    ledger.lifetime_limit = lifetime_limit;
    if ledger.claims.len() as u64 >= lifetime_limit {
        return Err(StorageError::SealedBudgetExhausted {
            used: ledger.claims.len() as u64,
            limit: lifetime_limit,
        });
    }

    let path = sealed_access_path(root, strategy_fingerprint, split_plan_hash);
    let claim = SealedAccessClaim {
        schema_version: SEALED_ACCESS_SCHEMA_VERSION,
        claimed_at: Utc::now(),
        strategy_fingerprint: strategy_fingerprint.clone(),
        split_plan_hash: split_plan_hash.clone(),
        challenge_artifact_hash: challenge_artifact_hash.clone(),
    };
    write_json_new(&path, &claim)?;

    ledger.claims.push(SealedBudgetEntry {
        strategy_fingerprint: strategy_fingerprint.clone(),
        challenge_artifact_hash: challenge_artifact_hash.clone(),
        claimed_at: claim.claimed_at,
    });
    // Budget ledger is versioned in place after the durable per-strategy claim.
    let budget_path = sealed_budget_path(root, split_plan_hash);
    if budget_path.is_file() {
        write_json_versioned(&budget_path, &ledger)?;
    } else {
        write_json_new(&budget_path, &ledger)?;
    }
    Ok(path)
}

/// Writes the one allowed sealed-final attempt. A failed report is retained and
/// blocks retries exactly like a passing report.
pub fn write_sealed_final_once<T: Serialize>(
    root: impl AsRef<Path>,
    report: &SealedFinalReport,
    artifact: &T,
) -> Result<PathBuf, StorageError> {
    if !report.shortlisted_before_open || report.used_in_selection_score {
        return Err(StorageError::InvalidSealedAttempt);
    }
    let access_path = sealed_access_path(
        &root,
        &report.binding.strategy_fingerprint,
        &report.split_plan_hash,
    );
    if !access_path.is_file() {
        return Err(StorageError::MissingSealedAccessClaim);
    }
    let path = sealed_final_path(
        root,
        &report.binding.strategy_fingerprint,
        &report.split_plan_hash,
    );
    write_json_new(&path, artifact)?;
    Ok(path)
}

/// Writes a new JSON artifact and refuses to replace an existing path.
pub fn write_json_new<T: Serialize>(path: impl AsRef<Path>, value: &T) -> Result<(), StorageError> {
    let path = path.as_ref();
    let temp = serialized_temp_file(path, value)?;
    temp.persist_noclobber(path)
        .map_err(|error| StorageError::Io(error.error))?;
    Ok(())
}

/// Atomically writes a new UTF-8 text artifact and refuses to replace an
/// existing path.
pub fn write_text_new(path: impl AsRef<Path>, value: &str) -> Result<(), StorageError> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temp = NamedTempFile::new_in(parent)?;
    temp.write_all(value.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path)
        .map_err(|error| StorageError::Io(error.error))?;
    Ok(())
}

/// Materializes a complete directory in a sibling temporary location and then
/// renames it into place. Relative paths are restricted to normal components,
/// and an existing destination is never intentionally replaced.
pub fn write_directory_new(
    destination: impl AsRef<Path>,
    files: &BTreeMap<PathBuf, Vec<u8>>,
) -> Result<(), StorageError> {
    let destination = destination.as_ref();
    if files.is_empty() {
        return Err(StorageError::InvalidDirectoryArtifact(
            "at least one file is required".into(),
        ));
    }
    if destination.exists() {
        return Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("destination already exists: {}", destination.display()),
        )));
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let temporary = tempfile::TempDir::new_in(parent)?;
    for (relative, bytes) in files {
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(StorageError::InvalidDirectoryArtifact(format!(
                "unsafe relative path {}",
                relative.display()
            )));
        }
        let path = temporary.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if destination.exists() {
        return Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!(
                "destination appeared during write: {}",
                destination.display()
            ),
        )));
    }
    fs::rename(temporary.path(), destination)?;
    let _persisted_path = temporary.keep();
    Ok(())
}

/// Atomically replaces a JSON artifact after preserving the prior version.
pub fn write_json_versioned<T: Serialize>(
    path: impl AsRef<Path>,
    value: &T,
) -> Result<Option<PathBuf>, StorageError> {
    let path = path.as_ref();
    let temp = serialized_temp_file(path, value)?;
    let backup = if path.exists() {
        let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Micros, true);
        let safe_stamp = stamp.replace([':', '-'], "");
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("artifact");
        let backup = path.with_file_name(format!("{file_name}.bak.{safe_stamp}"));
        fs::copy(path, &backup)?;
        Some(backup)
    } else {
        None
    };

    temp.persist(path)
        .map_err(|error| StorageError::Io(error.error))?;
    Ok(backup)
}

/// Atomically replaces a mutable JSON checkpoint without retaining historical
/// copies. Immutable artifacts must continue to use [`write_json_new`].
///
/// This is intentionally separate from [`write_json_versioned`]: long-running
/// workers update their recovery checkpoint repeatedly, and preserving a full
/// timestamped copy on every update causes unbounded disk growth.
pub fn write_json_replacing<T: Serialize>(
    path: impl AsRef<Path>,
    value: &T,
) -> Result<(), StorageError> {
    let path = path.as_ref();
    let temp = serialized_temp_file(path, value)?;
    temp.persist(path)
        .map_err(|error| StorageError::Io(error.error))?;
    Ok(())
}

fn serialized_temp_file<T: Serialize>(
    destination: &Path,
    value: &T,
) -> Result<NamedTempFile, StorageError> {
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let mut temp = NamedTempFile::new_in(parent)?;
    // Live recovery checkpoints are rewritten often and can hold hundreds of
    // Holding names. Pretty JSON made the first battery pass look paused for
    // minutes while the whole archive was re-serialized.
    serde_json::to_writer(&mut temp, value)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    Ok(temp)
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    #[error("Vault admission denied: {0}")]
    CertificationDenied(String),
    #[error("run manifest recipe hash does not match its content")]
    ManifestIntegrity,
    #[error("sealed-final attempt does not satisfy access invariants")]
    InvalidSealedAttempt,
    #[error("sealed-final report has no durable pre-open access claim")]
    MissingSealedAccessClaim,
    #[error("sealed-access budget exhausted ({used}/{limit} lifetime opens)")]
    SealedBudgetExhausted { used: u64, limit: u64 },
    #[error("invalid directory artifact: {0}")]
    InvalidDirectoryArtifact(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_quality::{
        BoundGateEvidence, CERTIFICATION_SCHEMA_VERSION, CHALLENGE_PROTOCOL, CertificationEvidence,
        DataGateEvidence, DataSegment, EVIDENCE_PROTOCOL_VERSION, EXTERNAL_PARITY_PROTOCOL,
        ExternalEngine, ExternalParityEvidence, ILLUMINATION_PROTOCOL, INDICATOR_PARITY_PROTOCOL,
        JUDGE_PROTOCOL, SEALED_FINAL_PROTOCOL, SealedFinalEvidence, VALIDATION_PROTOCOL,
    };

    fn test_plan() -> DataSplitPlan {
        DataSplitPlan {
            schema_version: 1,
            full_data_hash: ContentHash::sha256("full"),
            bar_count: 60,
            development: DataSegment {
                start_timestamp_ms: 0,
                end_timestamp_ms_exclusive: 20,
                bar_count: 20,
                data_hash: ContentHash::sha256("development"),
            },
            development_cpcv: None,
            validation: DataSegment {
                start_timestamp_ms: 20,
                end_timestamp_ms_exclusive: 40,
                bar_count: 20,
                data_hash: ContentHash::sha256("validation data"),
            },
            sealed_final: DataSegment {
                start_timestamp_ms: 40,
                end_timestamp_ms_exclusive: 60,
                bar_count: 20,
                data_hash: ContentHash::sha256("sealed data"),
            },
        }
    }

    fn gate(
        binding: &quantforge_quality::EvidenceBinding,
        artifact: &str,
        protocol: &str,
    ) -> BoundGateEvidence {
        BoundGateEvidence {
            binding: binding.clone(),
            artifact_hash: ContentHash::sha256(artifact),
            protocol_version: protocol.into(),
            passed: true,
            override_flags: Vec::new(),
        }
    }

    fn passing_evidence(plan: &DataSplitPlan) -> CertificationEvidence {
        let candidate = quantforge_quality::EvidenceBinding {
            strategy_fingerprint: ContentHash::sha256("strategy"),
            broker_spec_hash: ContentHash::sha256("broker"),
        };
        let split_plan_hash = plan.content_hash().unwrap();
        CertificationEvidence {
            schema_version: CERTIFICATION_SCHEMA_VERSION,
            protocol_version: EVIDENCE_PROTOCOL_VERSION.into(),
            candidate: candidate.clone(),
            split_plan_hash: split_plan_hash.clone(),
            validation: DataGateEvidence {
                gate: gate(&candidate, "validation", VALIDATION_PROTOCOL),
                data_hash: plan.validation.data_hash.clone(),
            },
            illumination: gate(&candidate, "illumination", ILLUMINATION_PROTOCOL),
            challenge: gate(&candidate, "challenge", CHALLENGE_PROTOCOL),
            judge: gate(&candidate, "judge", JUDGE_PROTOCOL),
            external_parity: ExternalParityEvidence {
                gate: gate(&candidate, "external", EXTERNAL_PARITY_PROTOCOL),
                engine: ExternalEngine::Mt5StrategyTester,
                protective_orders_present: true,
            },
            indicator_parity: gate(&candidate, "indicator", INDICATOR_PARITY_PROTOCOL),
            sealed_final: SealedFinalEvidence {
                gate: gate(&candidate, "sealed", SEALED_FINAL_PROTOCOL),
                split_plan_hash,
                sealed_data_hash: plan.sealed_final.data_hash.clone(),
                shortlisted_before_open: true,
                used_in_selection_score: false,
            },
            incubation: None,
            evaluations_touched: 100,
            research_override_flags: Vec::new(),
        }
    }

    #[test]
    fn immutable_manifest_refuses_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("manifest.json");
        let manifest = RunManifest::new("test", RunRecipe::empty()).unwrap();

        manifest.write_immutable(&path).unwrap();
        assert!(manifest.write_immutable(&path).is_err());
    }

    #[test]
    fn versioned_write_preserves_previous_value() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bank.json");

        assert!(write_json_versioned(&path, &1).unwrap().is_none());
        let backup = write_json_versioned(&path, &2).unwrap().unwrap();

        assert_eq!(fs::read_to_string(path).unwrap().trim(), "2");
        assert_eq!(fs::read_to_string(backup).unwrap().trim(), "1");
    }

    #[test]
    fn replacing_write_keeps_only_the_live_checkpoint() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bank.json");

        write_json_replacing(&path, &1).unwrap();
        write_json_replacing(&path, &2).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap().trim(), "2");
        let files = fs::read_dir(directory.path()).unwrap().count();
        assert_eq!(files, 1, "mutable replacement must not retain backups");
    }

    #[test]
    fn text_write_refuses_to_replace_existing_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("strategy.mq5");
        write_text_new(&path, "first").unwrap();
        assert!(write_text_new(&path, "second").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "first");
    }

    #[test]
    fn directory_write_materializes_complete_pack_and_refuses_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("deployment");
        let files = BTreeMap::from([
            (PathBuf::from("Expert.mq5"), b"source".to_vec()),
            (PathBuf::from("meta/risk.json"), b"{}\n".to_vec()),
        ]);

        write_directory_new(&path, &files).unwrap();
        assert_eq!(fs::read(path.join("Expert.mq5")).unwrap(), b"source");
        assert_eq!(fs::read(path.join("meta/risk.json")).unwrap(), b"{}\n");
        assert!(write_directory_new(&path, &files).is_err());
    }

    #[test]
    fn directory_write_rejects_parent_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let files = BTreeMap::from([(PathBuf::from("../escape"), b"no".to_vec())]);
        let result = write_directory_new(directory.path().join("deployment"), &files);
        assert!(matches!(
            result,
            Err(StorageError::InvalidDirectoryArtifact(_))
        ));
        assert!(!directory.path().join("escape").exists());
    }

    #[test]
    fn manifest_integrity_detects_recipe_tampering() {
        let mut manifest = RunManifest::new("test", RunRecipe::empty()).unwrap();
        manifest
            .recipe
            .override_flags
            .push("edited_after_creation".into());

        assert!(matches!(
            manifest.validate(),
            Err(StorageError::ManifestIntegrity)
        ));
    }

    #[test]
    fn vault_admits_certified_once_and_refuses_duplicate_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let plan = test_plan();
        let evidence = passing_evidence(&plan);

        let first = admit_certified(
            directory.path(),
            &evidence,
            &plan,
            &CertificationPolicy::default(),
            serde_json::json!({"artifact": "first"}),
        )
        .unwrap();
        assert!(first.path.is_file());
        let duplicate = admit_certified(
            directory.path(),
            &evidence,
            &plan,
            &CertificationPolicy::default(),
            serde_json::json!({"artifact": "changed payload"}),
        );
        assert!(duplicate.is_err());
    }

    #[test]
    fn denied_candidate_never_creates_a_vault_directory() {
        let directory = tempfile::tempdir().unwrap();
        let plan = test_plan();
        let mut evidence = passing_evidence(&plan);
        let candidate_directory = directory
            .path()
            .join(evidence.candidate.strategy_fingerprint.as_str());
        evidence.external_parity.engine = ExternalEngine::InternalJudge;

        let result = admit_certified(
            directory.path(),
            &evidence,
            &plan,
            &CertificationPolicy::default(),
            serde_json::json!({"artifact": "denied"}),
        );

        assert!(matches!(result, Err(StorageError::CertificationDenied(_))));
        assert!(!candidate_directory.exists());
    }

    #[test]
    fn sealed_access_claim_is_durable_and_no_clobber() {
        let directory = tempfile::tempdir().unwrap();
        let strategy = ContentHash::sha256("sealed strategy");
        let split = ContentHash::sha256("sealed split");
        let challenge = ContentHash::sha256("challenge artifact");

        let path =
            claim_sealed_access_once(directory.path(), &strategy, &split, &challenge).unwrap();
        assert!(path.is_file());
        assert!(claim_sealed_access_once(directory.path(), &strategy, &split, &challenge).is_err());
    }

    #[test]
    fn sealed_access_budget_refuses_the_21st_open() {
        let directory = tempfile::tempdir().unwrap();
        let split = ContentHash::sha256("budget split");
        for index in 0..DEFAULT_SEALED_ACCESS_BUDGET {
            let strategy = ContentHash::sha256(format!("strategy-{index}"));
            let challenge = ContentHash::sha256(format!("challenge-{index}"));
            claim_sealed_access_once(directory.path(), &strategy, &split, &challenge).unwrap();
        }
        let over = claim_sealed_access_once(
            directory.path(),
            &ContentHash::sha256("strategy-over"),
            &split,
            &ContentHash::sha256("challenge-over"),
        );
        assert!(matches!(
            over,
            Err(StorageError::SealedBudgetExhausted {
                used: DEFAULT_SEALED_ACCESS_BUDGET,
                limit: DEFAULT_SEALED_ACCESS_BUDGET
            })
        ));
    }
}
