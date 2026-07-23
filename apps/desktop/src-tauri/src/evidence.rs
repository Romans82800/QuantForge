use crate::data_lab::{display_path, load_bound_broker};
use crate::workflow::{
    ChallengeArtifact, EvolveArtifact, IncubationFinalArtifact, IndicatorParityArtifact,
    JudgeArtifact, ParityArtifact, SealedFinalArtifact, SplitPlanArtifact, ValidationArtifact,
    VaultArtifactReference, binding, content_hash, manifest, read_json, read_json_hashed,
    verify_split, write_json_new,
};
use quantforge_data::QualityGrade;
use quantforge_ir::StrategyIr;
use quantforge_parity::{ParityRun, compare_runs};
use quantforge_quality::{
    BoundGateEvidence, CERTIFICATION_SCHEMA_VERSION, CHALLENGE_PROTOCOL, CertificationEvidence,
    CertificationPolicy, DataGateEvidence, EVIDENCE_PROTOCOL_VERSION, EXTERNAL_PARITY_PROTOCOL,
    ExternalEngine, ExternalParityEvidence, ILLUMINATION_PROTOCOL, INCUBATION_PROTOCOL,
    INDICATOR_PARITY_PROTOCOL, JUDGE_PROTOCOL, SEALED_FINAL_PROTOCOL, SealedFinalEvidence,
    VALIDATION_PROTOCOL, ValidationAttestation, evaluate_certification,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const EVIDENCE_BUNDLE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssembleEvidenceRequest {
    strategy_path: String,
    broker_path: String,
    split_plan_path: String,
    databank_path: String,
    challenge_path: String,
    judge_path: String,
    parity_path: String,
    indicator_parity_path: String,
    sealed_final_path: String,
    incubation_path: Option<String>,
    output_directory: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceView {
    output_directory: String,
    validation_path: String,
    evidence_path: String,
    bundle_path: String,
    gate_count: usize,
    evaluations_touched: u64,
    certification_ready: bool,
    incubation_included: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EvidenceBundle {
    schema_version: u16,
    manifest: quantforge_storage::RunManifest,
    evidence_source: String,
    evidence_hash: quantforge_core::ContentHash,
    artifacts: Vec<VaultArtifactReference>,
}

#[tauri::command]
pub async fn assemble_evidence(request: AssembleEvidenceRequest) -> Result<EvidenceView, String> {
    tauri::async_runtime::spawn_blocking(move || assemble_evidence_sync(&request))
        .await
        .map_err(|error| format!("evidence assembly task failed: {error}"))?
}

fn assemble_evidence_sync(request: &AssembleEvidenceRequest) -> Result<EvidenceView, String> {
    let out = PathBuf::from(&request.output_directory);
    if out.exists() {
        return Err(format!(
            "evidence directory already exists: {}",
            out.display()
        ));
    }
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
    let broker = load_bound_broker(&request.broker_path, None)?;
    let candidate = binding(&strategy, &broker)?;
    let split: SplitPlanArtifact = read_json(&request.split_plan_path)?;
    verify_split(&split)?;
    let split_hash = split
        .plan
        .content_hash()
        .map_err(|error| error.to_string())?;
    let (databank, databank_hash) = read_json_hashed::<EvolveArtifact>(&request.databank_path)?;
    let (challenge, challenge_hash) =
        read_json_hashed::<ChallengeArtifact>(&request.challenge_path)?;
    let (judge, judge_hash) = read_json_hashed::<JudgeArtifact>(&request.judge_path)?;
    let (parity, parity_hash) = read_json_hashed::<ParityArtifact>(&request.parity_path)?;
    let (indicator, indicator_hash) =
        read_json_hashed::<IndicatorParityArtifact>(&request.indicator_parity_path)?;
    let (sealed, sealed_hash) =
        read_json_hashed::<SealedFinalArtifact>(&request.sealed_final_path)?;
    let incubation = request
        .incubation_path
        .as_ref()
        .map(read_json_hashed::<IncubationFinalArtifact>)
        .transpose()?;

    verify_challenge(&challenge, &candidate, &split, &split_hash)?;
    verify_databank(&databank, &strategy, &candidate, &split, &challenge)?;
    verify_judge(&judge, &candidate, &split, &challenge)?;
    verify_parity(&parity, &candidate, &broker, &split, &challenge)?;
    verify_indicator(&indicator, &broker.symbol)?;
    verify_sealed(
        &sealed,
        &candidate,
        &split,
        &split_hash,
        &challenge,
        &challenge_hash,
    )?;
    if let Some((artifact, _)) = &incubation {
        verify_incubation(artifact, &candidate, &split, &split_hash)?;
    }

    fs::create_dir_all(&out)
        .map_err(|error| format!("cannot create evidence directory: {error}"))?;
    let validation_path = out.join("validation-attestation.json");
    let evidence_path = out.join("certification-evidence.json");
    let bundle_path = out.join("certification-bundle.json");
    let attestation =
        ValidationAttestation::from_challenge(&challenge.report, challenge_hash.clone())
            .map_err(|error| error.to_string())?;
    if !attestation.passed {
        return Err("Challenge validation baseline did not pass".into());
    }
    let attestation_hash =
        quantforge_core::stable_json_hash(&attestation).map_err(|error| error.to_string())?;
    let validation_manifest = manifest(
        "validation-attestation",
        Some(attestation.validation_data_hash.clone()),
        Some(attestation.binding.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            (
                "challenge".into(),
                json!(display_path(Path::new(&request.challenge_path))),
            ),
            (
                "source_challenge_artifact_hash".into(),
                json!(&challenge_hash),
            ),
            ("attestation_hash".into(), json!(&attestation_hash)),
            ("split_plan_hash".into(), json!(&split_hash)),
            (
                "strategy_fingerprint".into(),
                json!(&candidate.strategy_fingerprint),
            ),
            ("protocol".into(), json!(VALIDATION_PROTOCOL)),
            ("passed".into(), json!(attestation.passed)),
        ]),
    )?;
    write_json_new(
        &validation_path,
        &ValidationArtifact {
            manifest: validation_manifest,
            challenge_source: display_path(Path::new(&request.challenge_path)),
            attestation,
        },
    )?;
    let validation_hash = content_hash(&validation_path)?;
    let evidence = CertificationEvidence {
        schema_version: CERTIFICATION_SCHEMA_VERSION,
        protocol_version: EVIDENCE_PROTOCOL_VERSION.into(),
        candidate: candidate.clone(),
        split_plan_hash: split_hash.clone(),
        validation: DataGateEvidence {
            gate: passing_gate(&candidate, validation_hash.clone(), VALIDATION_PROTOCOL),
            data_hash: split.plan.validation.data_hash.clone(),
        },
        illumination: passing_gate(&candidate, databank_hash.clone(), ILLUMINATION_PROTOCOL),
        challenge: passing_gate(&candidate, challenge_hash.clone(), CHALLENGE_PROTOCOL),
        judge: passing_gate(&candidate, judge_hash.clone(), JUDGE_PROTOCOL),
        external_parity: ExternalParityEvidence {
            gate: passing_gate(&candidate, parity_hash.clone(), EXTERNAL_PARITY_PROTOCOL),
            engine: ExternalEngine::Mt5StrategyTester,
            protective_orders_present: parity.report.protective_orders_present,
        },
        indicator_parity: passing_gate(
            &candidate,
            indicator_hash.clone(),
            INDICATOR_PARITY_PROTOCOL,
        ),
        sealed_final: SealedFinalEvidence {
            gate: passing_gate(&candidate, sealed_hash.clone(), SEALED_FINAL_PROTOCOL),
            split_plan_hash: sealed.report.split_plan_hash.clone(),
            sealed_data_hash: sealed.report.sealed_data_hash.clone(),
            shortlisted_before_open: sealed.report.shortlisted_before_open,
            used_in_selection_score: sealed.report.used_in_selection_score,
        },
        incubation: incubation
            .as_ref()
            .map(|(_, hash)| passing_gate(&candidate, hash.clone(), INCUBATION_PROTOCOL)),
        evaluations_touched: databank.databank.evaluation_count,
        research_override_flags: Vec::new(),
    };
    let policy = CertificationPolicy {
        require_incubation: incubation.is_some(),
        ..CertificationPolicy::default()
    };
    let decision = evaluate_certification(&evidence, &split.plan, &policy)
        .map_err(|error| error.to_string())?;
    if !decision.passed {
        return Err(format!(
            "assembled evidence failed certification: {:?}",
            decision.blockers
        ));
    }
    write_json_new(&evidence_path, &evidence)?;
    let evidence_hash = content_hash(&evidence_path)?;
    let mut artifacts = vec![
        reference("validation", &validation_path, validation_hash),
        reference("illumination", &request.databank_path, databank_hash),
        reference("challenge", &request.challenge_path, challenge_hash),
        reference("judge", &request.judge_path, judge_hash),
        reference("external_parity", &request.parity_path, parity_hash),
        reference(
            "indicator_parity",
            &request.indicator_parity_path,
            indicator_hash,
        ),
        reference("sealed_final", &request.sealed_final_path, sealed_hash),
    ];
    if let (Some(path), Some((_, hash))) = (&request.incubation_path, &incubation) {
        artifacts.push(reference("incubation", path, hash.clone()));
    }
    let gate_count = artifacts.len();
    let bundle_manifest = manifest(
        "assemble-evidence",
        Some(split.plan.full_data_hash.clone()),
        Some(candidate.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("evidence_hash".into(), json!(&evidence_hash)),
            (
                "artifacts".into(),
                serde_json::to_value(&artifacts).map_err(|error| error.to_string())?,
            ),
            ("split_plan_hash".into(), json!(&split_hash)),
            (
                "strategy_fingerprint".into(),
                json!(&candidate.strategy_fingerprint),
            ),
        ]),
    )?;
    write_json_new(
        &bundle_path,
        &EvidenceBundle {
            schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION,
            manifest: bundle_manifest,
            evidence_source: display_path(&evidence_path),
            evidence_hash,
            artifacts,
        },
    )?;
    Ok(EvidenceView {
        output_directory: display_path(&out),
        validation_path: display_path(&validation_path),
        evidence_path: display_path(&evidence_path),
        bundle_path: display_path(&bundle_path),
        gate_count,
        evaluations_touched: databank.databank.evaluation_count,
        certification_ready: true,
        incubation_included: incubation.is_some(),
    })
}

fn verify_challenge(
    artifact: &ChallengeArtifact,
    candidate: &quantforge_quality::EvidenceBinding,
    split: &SplitPlanArtifact,
    split_hash: &quantforge_core::ContentHash,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .report
        .validate_integrity()
        .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "challenge"
        || artifact.report.binding != *candidate
        || artifact.report.split_plan_hash != *split_hash
        || artifact.report.validation_data_hash != split.plan.validation.data_hash
        || !artifact.report.passed
        || !artifact.report.blockers.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || !artifact.manifest.recipe.override_flags.is_empty()
    {
        return Err("Challenge artifact is failed, mismatched or overridden".into());
    }
    Ok(())
}

fn verify_databank(
    artifact: &EvolveArtifact,
    strategy: &StrategyIr,
    candidate: &quantforge_quality::EvidenceBinding,
    split: &SplitPlanArtifact,
    challenge: &ChallengeArtifact,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .databank
        .validate_integrity()
        .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "evolve"
        || artifact.databank.data_hash != split.plan.development.data_hash
        || artifact.databank.broker_spec_hash != candidate.broker_spec_hash
        || artifact.databank.evaluation_count != challenge.report.config.evaluations_touched
        || artifact.databank.config.scout != challenge.report.config.scout
        || artifact.data_quality.grade == QualityGrade::Fail
        || !artifact.manifest.recipe.override_flags.is_empty()
        || !artifact.databank.elites.iter().any(|elite| {
            elite.structural_fingerprint == candidate.strategy_fingerprint
                && elite.strategy == *strategy
        })
    {
        return Err("databank is not an exact development-only source for this candidate".into());
    }
    Ok(())
}

fn verify_judge(
    artifact: &JudgeArtifact,
    candidate: &quantforge_quality::EvidenceBinding,
    split: &SplitPlanArtifact,
    challenge: &ChallengeArtifact,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    let decision_hash: quantforge_core::ContentHash = serde_json::from_value(
        artifact
            .manifest
            .recipe
            .config
            .get("decision_data_hash")
            .ok_or_else(|| "Judge manifest lacks decision hash".to_owned())?
            .clone(),
    )
    .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "judge"
        || artifact.strategy_fingerprint != candidate.strategy_fingerprint
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&candidate.broker_spec_hash)
        || decision_hash != split.plan.validation.data_hash
        || artifact.decision_data_quality.grade == QualityGrade::Fail
        || artifact.m1_data_quality.grade == QualityGrade::Fail
        || artifact.result.engine != quantforge_tick::ENGINE_TIER
        || artifact.result.telemetry.m1_gap_events != 0
        || !challenge
            .report
            .metrics_pass_baseline(&artifact.result.metrics)
        || !artifact.manifest.recipe.override_flags.is_empty()
    {
        return Err("M1 Judge artifact is failed, mismatched or overridden".into());
    }
    Ok(())
}

fn verify_parity(
    artifact: &ParityArtifact,
    candidate: &quantforge_quality::EvidenceBinding,
    broker: &quantforge_broker::SymbolSpecification,
    split: &SplitPlanArtifact,
    challenge: &ChallengeArtifact,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .mt5_metadata
        .validate_evidence(&artifact.evidence)
        .map_err(|error| error.to_string())?;
    let recomputed = compare_runs(
        &artifact.reference,
        &artifact.external,
        &artifact.evidence,
        artifact.report.tolerances.clone(),
    )
    .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "parity"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&split.plan.validation.data_hash)
        || artifact.evidence.strategy_fingerprint != candidate.strategy_fingerprint
        || artifact.evidence.broker_spec_hash != candidate.broker_spec_hash
        || artifact.evidence.symbol != broker.symbol
        || artifact.reference != ParityRun::from_scout(&challenge.report.baseline)
        || artifact.external.engine != "mt5-strategy-tester"
        || artifact.report != recomputed
        || !artifact.report.passed
        || !artifact.report.protective_orders_present
        || artifact.evidence.live_trading_default
        || !artifact.manifest.recipe.override_flags.is_empty()
    {
        return Err("external parity artifact is failed, mismatched or unsafe".into());
    }
    Ok(())
}

fn verify_indicator(artifact: &IndicatorParityArtifact, symbol: &str) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    let required: BTreeSet<_> = [
        "sma",
        "ema",
        "wma",
        "rsi",
        "atr",
        "donchian_high",
        "donchian_low",
        "highest_close",
        "lowest_close",
        "standard_deviation",
        "zscore",
        "percentile_in_range",
        "rate_of_change",
    ]
    .into_iter()
    .collect();
    let actual: BTreeSet<_> = artifact
        .report
        .indicators
        .keys()
        .map(String::as_str)
        .collect();
    if artifact.manifest.command != "indicator-parity"
        || !artifact.report.passed
        || artifact.report.metadata.symbol != symbol
        || actual != required
        || artifact
            .report
            .indicators
            .values()
            .any(|value| !value.passed || value.mismatch_count != 0)
        || !artifact.manifest.recipe.override_flags.is_empty()
    {
        return Err("indicator parity artifact is failed, incomplete or mismatched".into());
    }
    Ok(())
}

fn verify_sealed(
    artifact: &SealedFinalArtifact,
    candidate: &quantforge_quality::EvidenceBinding,
    split: &SplitPlanArtifact,
    split_hash: &quantforge_core::ContentHash,
    challenge: &ChallengeArtifact,
    challenge_hash: &quantforge_core::ContentHash,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .report
        .validate_integrity(&challenge.report)
        .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "sealed-final"
        || artifact.report.binding != *candidate
        || artifact.report.split_plan_hash != *split_hash
        || artifact.report.challenge_artifact_hash != *challenge_hash
        || artifact.report.sealed_data_hash != split.plan.sealed_final.data_hash
        || !artifact.report.passed
        || !artifact.report.blockers.is_empty()
        || !artifact.report.shortlisted_before_open
        || artifact.report.used_in_selection_score
        || artifact.data_quality.grade == QualityGrade::Fail
        || !artifact.manifest.recipe.override_flags.is_empty()
    {
        return Err("sealed-final artifact is failed, reused or mismatched".into());
    }
    Ok(())
}

fn verify_incubation(
    artifact: &IncubationFinalArtifact,
    candidate: &quantforge_quality::EvidenceBinding,
    split: &SplitPlanArtifact,
    split_hash: &quantforge_core::ContentHash,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .report
        .validate_integrity(&artifact.start, &artifact.observations)
        .map_err(|error| error.to_string())?;
    if artifact.manifest.command != "incubation-final"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&split.plan.full_data_hash)
        || artifact.start.binding != *candidate
        || artifact.start.split_plan_hash != *split_hash
        || !artifact.report.passed
        || !artifact.report.blockers.is_empty()
    {
        return Err("incubation artifact is failed or mismatched".into());
    }
    Ok(())
}

fn passing_gate(
    binding: &quantforge_quality::EvidenceBinding,
    artifact_hash: quantforge_core::ContentHash,
    protocol: &str,
) -> BoundGateEvidence {
    BoundGateEvidence {
        binding: binding.clone(),
        artifact_hash,
        protocol_version: protocol.into(),
        passed: true,
        override_flags: Vec::new(),
    }
}
fn reference(
    gate: &str,
    path: impl AsRef<Path>,
    hash: quantforge_core::ContentHash,
) -> VaultArtifactReference {
    VaultArtifactReference {
        gate: gate.into(),
        path: display_path(path.as_ref()),
        content_hash: hash,
    }
}
