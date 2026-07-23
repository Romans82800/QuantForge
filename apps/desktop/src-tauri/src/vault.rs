use crate::data_lab::{display_path, load_bound_broker};
use crate::workflow::{
    SplitPlanArtifact, VaultArtifactReference, VaultPayload, binding, content_hash, manifest,
    read_json, recipe_path, verify_split,
};
use quantforge_core::ContentHash;
use quantforge_ir::StrategyIr;
use quantforge_quality::{
    CertificationEvidence, CertificationPolicy, StrategyGrade, evaluate_certification,
};
use quantforge_storage::{CertifiedVaultEntry, VAULT_SCHEMA_VERSION, admit_certified};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultRequest {
    vault_directory: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CertifyRequest {
    strategy_path: String,
    broker_path: String,
    split_plan_path: String,
    evidence_path: String,
    artifact_paths: Vec<String>,
    vault_directory: String,
    require_incubation: bool,
    selection_bias_warning_threshold: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultView {
    vault_directory: String,
    certified_entries: Vec<VaultEntryView>,
    rejected_files: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultEntryView {
    path: String,
    entry_id: String,
    strategy_fingerprint: String,
    admitted_at: String,
    grade: String,
    evidence_hash: String,
    warnings: usize,
    incubation_required: bool,
}

#[tauri::command]
pub async fn inspect_vault(request: VaultRequest) -> Result<VaultView, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_vault_sync(&request))
        .await
        .map_err(|error| format!("Vault inspection failed: {error}"))?
}

#[tauri::command]
pub async fn certify_to_vault(request: CertifyRequest) -> Result<VaultView, String> {
    tauri::async_runtime::spawn_blocking(move || certify_to_vault_sync(&request))
        .await
        .map_err(|error| format!("Vault certification failed: {error}"))?
}

fn inspect_vault_sync(request: &VaultRequest) -> Result<VaultView, String> {
    let root = PathBuf::from(&request.vault_directory);
    if !root.exists() {
        return Ok(VaultView {
            vault_directory: display_path(&root),
            certified_entries: Vec::new(),
            rejected_files: 0,
        });
    }
    let mut files = Vec::new();
    collect_json_files(&root, &mut files)?;
    let mut certified_entries = Vec::new();
    let mut rejected_files = 0;
    for path in files {
        let Ok(entry) = read_json::<CertifiedVaultEntry<VaultPayload>>(&path) else {
            rejected_files += 1;
            continue;
        };
        match verify_entry(&entry) {
            Ok(policy) => certified_entries.push(VaultEntryView {
                path: display_path(&path),
                entry_id: entry.entry_id.as_str().into(),
                strategy_fingerprint: entry.strategy_fingerprint.as_str().into(),
                admitted_at: entry.admitted_at.to_rfc3339(),
                grade: "certified".into(),
                evidence_hash: entry.certification.evidence_hash.as_str().into(),
                warnings: entry.certification.warnings.len(),
                incubation_required: policy.require_incubation,
            }),
            Err(_) => rejected_files += 1,
        }
    }
    certified_entries.sort_by(|left, right| right.admitted_at.cmp(&left.admitted_at));
    Ok(VaultView {
        vault_directory: display_path(&root),
        certified_entries,
        rejected_files,
    })
}

fn certify_to_vault_sync(request: &CertifyRequest) -> Result<VaultView, String> {
    if request.artifact_paths.is_empty() {
        return Err(
            "certification requires every gate artifact referenced by the evidence file".into(),
        );
    }
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
    let broker = load_bound_broker(&request.broker_path, None)?;
    let split: SplitPlanArtifact = read_json(&request.split_plan_path)?;
    verify_split(&split)?;
    let evidence: CertificationEvidence = read_json(&request.evidence_path)?;
    let actual_binding = binding(&strategy, &broker)?;
    if evidence.candidate != actual_binding {
        return Err("strategy or broker does not match the certification evidence".into());
    }
    let policy = CertificationPolicy {
        require_incubation: request.require_incubation,
        selection_bias_warning_threshold: request.selection_bias_warning_threshold,
    };
    let decision = evaluate_certification(&evidence, &split.plan, &policy)
        .map_err(|error| error.to_string())?;
    if !decision.passed {
        return Err(format!("certification denied: {:?}", decision.blockers));
    }
    let artifact_paths: Vec<_> = request.artifact_paths.iter().map(PathBuf::from).collect();
    let artifacts = bind_gate_artifacts(&evidence, &artifact_paths)?;
    let run_manifest = manifest(
        "certify",
        Some(split.plan.full_data_hash.clone()),
        Some(actual_binding.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("strategy".into(), recipe_path(&request.strategy_path)),
            ("broker".into(), recipe_path(&request.broker_path)),
            ("split_plan".into(), recipe_path(&request.split_plan_path)),
            ("evidence".into(), recipe_path(&request.evidence_path)),
            ("vault".into(), recipe_path(&request.vault_directory)),
            (
                "policy".into(),
                serde_json::to_value(&policy).map_err(|error| error.to_string())?,
            ),
            (
                "strategy_fingerprint".into(),
                json!(&actual_binding.strategy_fingerprint),
            ),
            (
                "split_plan_hash".into(),
                json!(
                    split
                        .plan
                        .content_hash()
                        .map_err(|error| error.to_string())?
                ),
            ),
            ("evidence_hash".into(), json!(&decision.evidence_hash)),
        ]),
    )?;
    let payload = VaultPayload {
        manifest: run_manifest,
        strategy_source: display_path(Path::new(&request.strategy_path)),
        strategy_source_hash: content_hash(&request.strategy_path)?,
        strategy,
        broker_source: display_path(Path::new(&request.broker_path)),
        broker_source_hash: content_hash(&request.broker_path)?,
        broker,
        split_plan_source: display_path(Path::new(&request.split_plan_path)),
        split_plan_source_hash: content_hash(&request.split_plan_path)?,
        split_plan: split.plan,
        evidence_source: display_path(Path::new(&request.evidence_path)),
        evidence_source_hash: content_hash(&request.evidence_path)?,
        evidence: evidence.clone(),
        artifacts,
    };
    let split_plan = payload.split_plan.clone();
    admit_certified(
        &request.vault_directory,
        &evidence,
        &split_plan,
        &policy,
        payload,
    )
    .map_err(|error| error.to_string())?;
    inspect_vault_sync(&VaultRequest {
        vault_directory: request.vault_directory.clone(),
    })
}

pub(crate) fn verify_entry(
    entry: &CertifiedVaultEntry<VaultPayload>,
) -> Result<CertificationPolicy, String> {
    entry
        .payload
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    entry
        .payload
        .split_plan
        .validate()
        .map_err(|error| error.to_string())?;
    let actual_binding = binding(&entry.payload.strategy, &entry.payload.broker)?;
    let policy: CertificationPolicy = serde_json::from_value(
        entry
            .payload
            .manifest
            .recipe
            .config
            .get("policy")
            .ok_or_else(|| "Vault manifest is missing its certification policy".to_owned())?
            .clone(),
    )
    .map_err(|error| error.to_string())?;
    let decision =
        evaluate_certification(&entry.payload.evidence, &entry.payload.split_plan, &policy)
            .map_err(|error| error.to_string())?;
    #[derive(Serialize)]
    struct Identity<'a> {
        schema_version: u16,
        strategy_fingerprint: &'a ContentHash,
        evidence_hash: &'a ContentHash,
    }
    let expected_id = quantforge_core::stable_json_hash(&Identity {
        schema_version: VAULT_SCHEMA_VERSION,
        strategy_fingerprint: &decision.candidate.strategy_fingerprint,
        evidence_hash: &decision.evidence_hash,
    })
    .map_err(|error| error.to_string())?;
    if entry.schema_version != VAULT_SCHEMA_VERSION
        || entry.payload.manifest.command != "certify"
        || !entry.payload.manifest.recipe.override_flags.is_empty()
        || entry.payload.manifest.recipe.data_hash.as_ref()
            != Some(&entry.payload.split_plan.full_data_hash)
        || entry.payload.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&actual_binding.broker_spec_hash)
        || entry.payload.evidence.candidate != actual_binding
        || entry.payload.evidence.split_plan_hash
            != entry
                .payload
                .split_plan
                .content_hash()
                .map_err(|error| error.to_string())?
        || entry.strategy_fingerprint != actual_binding.strategy_fingerprint
        || entry.payload_hash
            != quantforge_core::stable_json_hash(&entry.payload)
                .map_err(|error| error.to_string())?
        || entry.entry_id != expected_id
        || entry.certification != decision
        || !entry.certification.passed
        || entry.certification.resulting_grade != StrategyGrade::Certified
    {
        return Err("Vault entry is not an intact Certified artifact".into());
    }
    for reference in &entry.payload.artifacts {
        if content_hash(&reference.path)? != reference.content_hash {
            return Err(format!(
                "{} gate artifact changed after admission",
                reference.gate
            ));
        }
    }
    Ok(policy)
}

pub(crate) fn bind_gate_artifacts(
    evidence: &CertificationEvidence,
    paths: &[PathBuf],
) -> Result<Vec<VaultArtifactReference>, String> {
    let mut available = BTreeMap::<ContentHash, &Path>::new();
    for path in paths {
        let hash = content_hash(path)?;
        if let Some(previous) = available.insert(hash.clone(), path) {
            return Err(format!(
                "duplicate artifact supplied by {} and {} ({hash})",
                previous.display(),
                path.display()
            ));
        }
    }
    let mut required = vec![
        ("validation", &evidence.validation.gate.artifact_hash),
        ("illumination", &evidence.illumination.artifact_hash),
        ("challenge", &evidence.challenge.artifact_hash),
        ("judge", &evidence.judge.artifact_hash),
        (
            "external_parity",
            &evidence.external_parity.gate.artifact_hash,
        ),
        ("indicator_parity", &evidence.indicator_parity.artifact_hash),
        ("sealed_final", &evidence.sealed_final.gate.artifact_hash),
    ];
    if let Some(incubation) = &evidence.incubation {
        required.push(("incubation", &incubation.artifact_hash));
    }
    let mut used = BTreeSet::new();
    let mut references = Vec::with_capacity(required.len());
    for (gate, hash) in required {
        let path = available.get(hash).ok_or_else(|| {
            format!("the {gate} gate references {hash}, but that artifact was not supplied")
        })?;
        used.insert(hash.clone());
        references.push(VaultArtifactReference {
            gate: gate.into(),
            path: display_path(path),
            content_hash: hash.clone(),
        });
    }
    if let Some(unused) = available.iter().find(|(hash, _)| !used.contains(*hash)) {
        return Err(format!(
            "supplied artifact {} is not referenced by a certification gate",
            unused.1.display()
        ));
    }
    Ok(references)
}

fn collect_json_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(root)
        .map_err(|error| format!("cannot read Vault directory {}: {error}", root.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, output)?;
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".certified.json"))
        {
            output.push(path);
        }
    }
    Ok(())
}
