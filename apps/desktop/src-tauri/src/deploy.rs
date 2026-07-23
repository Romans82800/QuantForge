use crate::data_lab::display_path;
use crate::vault::verify_entry;
use crate::workflow::{
    IncubationFinalArtifact, ParityArtifact, VaultPayload, manifest, read_json_hashed,
};
use quantforge_broker::{FillingMode, TradeMode};
use quantforge_core::ContentHash;
use quantforge_export_mql5::{ExportEvidenceCard, generate_bundle};
use quantforge_parity::compare_runs;
use quantforge_quality::{EvidenceBinding, StrategyGrade};
use quantforge_storage::{CertifiedVaultEntry, write_directory_new};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEPLOYMENT_SCHEMA_VERSION: u16 = 1;
const DEPLOYMENT_PROTOCOL_VERSION: &str = "mt5-deployment-pack-v1";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployRequest {
    vault_entry_path: String,
    output_directory: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployView {
    output_directory: String,
    deployment_id: String,
    grade: &'static str,
    expert_name: String,
    symbol: String,
    timeframe: String,
    magic: u64,
    file_count: usize,
    live_trading_default: bool,
    certification_warnings: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeploymentBrokerLimits {
    volume_min: f64,
    volume_step: f64,
    volume_max: f64,
    stops_level_points: u32,
    freeze_level_points: u32,
    filling_modes: Vec<FillingMode>,
    trade_mode: TradeMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeploymentRiskPack {
    schema_version: u16,
    protocol_version: String,
    certified_vault_entry_id: ContentHash,
    certified_vault_entry_hash: ContentHash,
    certification_evidence_hash: ContentHash,
    incubation_artifact_hash: ContentHash,
    candidate: EvidenceBinding,
    symbol: String,
    timeframe: String,
    magic: u64,
    deviation_points: u32,
    maximum_spread_points: Option<f64>,
    estimated_slippage_points_per_side: f64,
    commission_per_lot_round_turn: f64,
    live_trading_default: bool,
    export_config: quantforge_export_mql5::Mql5ExportConfig,
    strategy_risk: quantforge_ir::RiskPolicy,
    protective_stops: quantforge_ir::ProtectiveStops,
    broker_limits: DeploymentBrokerLimits,
    certification_warnings: Vec<quantforge_quality::CertificationWarning>,
    operator_notice: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DeploymentFileRecord {
    relative_path: String,
    content_hash: ContentHash,
    byte_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeploymentManifest {
    schema_version: u16,
    protocol_version: String,
    deployment_id: ContentHash,
    grade: StrategyGrade,
    run_manifest: quantforge_storage::RunManifest,
    certified_vault_entry_source: String,
    certified_vault_entry_id: ContentHash,
    certified_vault_entry_hash: ContentHash,
    external_parity_artifact_hash: ContentHash,
    incubation_artifact_hash: ContentHash,
    candidate: EvidenceBinding,
    live_trading_default: bool,
    files: Vec<DeploymentFileRecord>,
}

#[derive(Serialize)]
struct DeploymentIdentity<'a> {
    protocol_version: &'a str,
    certified_vault_entry_id: &'a ContentHash,
    certified_vault_entry_hash: &'a ContentHash,
    external_parity_artifact_hash: &'a ContentHash,
    incubation_artifact_hash: &'a ContentHash,
    candidate: &'a EvidenceBinding,
    files: &'a [DeploymentFileRecord],
}

#[tauri::command]
pub async fn build_deployment_pack(request: DeployRequest) -> Result<DeployView, String> {
    tauri::async_runtime::spawn_blocking(move || build_deployment_pack_sync(&request))
        .await
        .map_err(|error| format!("deployment task failed: {error}"))?
}

fn build_deployment_pack_sync(request: &DeployRequest) -> Result<DeployView, String> {
    let out = PathBuf::from(&request.output_directory);
    if out.exists() {
        return Err(format!(
            "deployment pack already exists and will not be replaced: {}",
            out.display()
        ));
    }
    let (entry, vault_entry_hash) =
        read_json_hashed::<CertifiedVaultEntry<VaultPayload>>(&request.vault_entry_path)?;
    let policy = verify_entry(&entry)?;
    if !policy.require_incubation {
        return Err("deployment requires a Vault entry certified with mandatory incubation".into());
    }
    let binding = entry.payload.evidence.candidate.clone();
    let references: BTreeMap<_, _> = entry
        .payload
        .artifacts
        .iter()
        .map(|reference| (reference.gate.as_str(), reference))
        .collect();
    let incubation_reference = references
        .get("incubation")
        .ok_or_else(|| "Certified Vault entry has no incubation artifact".to_owned())?;
    let (incubation, incubation_hash) =
        read_json_hashed::<IncubationFinalArtifact>(&incubation_reference.path)?;
    if incubation_hash != incubation_reference.content_hash
        || Some(&incubation_hash)
            != entry
                .payload
                .evidence
                .incubation
                .as_ref()
                .map(|gate| &gate.artifact_hash)
    {
        return Err("incubation artifact does not match the Certified evidence".into());
    }
    incubation
        .report
        .validate_integrity(&incubation.start, &incubation.observations)
        .map_err(|error| error.to_string())?;
    let split_hash = entry
        .payload
        .split_plan
        .content_hash()
        .map_err(|error| error.to_string())?;
    if incubation.manifest.command != "incubation-final"
        || incubation.manifest.recipe.data_hash.as_ref()
            != Some(&entry.payload.split_plan.full_data_hash)
        || incubation.start.binding != binding
        || incubation.start.split_plan_hash != split_hash
        || !incubation.report.passed
        || !incubation.report.blockers.is_empty()
    {
        return Err("incubation artifact is failed, mismatched or internally unbound".into());
    }

    let parity_reference = references
        .get("external_parity")
        .ok_or_else(|| "Certified Vault entry has no external parity artifact".to_owned())?;
    let (parity, parity_hash) = read_json_hashed::<ParityArtifact>(&parity_reference.path)?;
    if parity_hash != parity_reference.content_hash
        || parity_hash != entry.payload.evidence.external_parity.gate.artifact_hash
    {
        return Err("external parity artifact does not match the Certified evidence".into());
    }
    verify_parity(
        &parity,
        &binding,
        &entry.payload.broker,
        &entry.payload.split_plan,
    )?;
    let generated = generate_bundle(
        &entry.payload.strategy,
        &entry.payload.broker,
        &parity.evidence.config,
    )
    .map_err(|error| error.to_string())?;
    if generated.evidence != parity.evidence
        || ContentHash::sha256(generated.source.as_bytes()) != parity.evidence.source_hash
    {
        return Err("regenerated EA does not match the source that passed external parity".into());
    }

    let risk_pack = DeploymentRiskPack {
        schema_version: DEPLOYMENT_SCHEMA_VERSION,
        protocol_version: DEPLOYMENT_PROTOCOL_VERSION.into(),
        certified_vault_entry_id: entry.entry_id.clone(),
        certified_vault_entry_hash: vault_entry_hash.clone(),
        certification_evidence_hash: entry.certification.evidence_hash.clone(),
        incubation_artifact_hash: incubation_hash.clone(),
        candidate: binding.clone(),
        symbol: entry.payload.broker.symbol.clone(),
        timeframe: generated.evidence.timeframe.clone(),
        magic: generated.evidence.config.magic,
        deviation_points: generated.evidence.config.deviation_points,
        maximum_spread_points: generated.evidence.config.max_spread_points,
        estimated_slippage_points_per_side: generated
            .evidence
            .config
            .estimated_slippage_points_per_side,
        commission_per_lot_round_turn: generated
            .evidence
            .config
            .commission_per_lot_round_turn,
        live_trading_default: false,
        export_config: generated.evidence.config.clone(),
        strategy_risk: entry.payload.strategy.risk.clone(),
        protective_stops: entry.payload.strategy.stops.clone(),
        broker_limits: DeploymentBrokerLimits {
            volume_min: entry.payload.broker.volume_min,
            volume_step: entry.payload.broker.volume_step,
            volume_max: entry.payload.broker.volume_max,
            stops_level_points: entry.payload.broker.stops_level_points,
            freeze_level_points: entry.payload.broker.freeze_level_points,
            filling_modes: entry.payload.broker.filling_modes.clone(),
            trade_mode: entry.payload.broker.trade_mode,
        },
        certification_warnings: entry.certification.warnings.clone(),
        operator_notice: "Certification and paper incubation are not a profitability guarantee. AllowLiveTrading remains false until an operator completes independent review.".into(),
    };
    let expert_name = generated.evidence.expert_name.clone();
    let mut files = BTreeMap::<PathBuf, Vec<u8>>::from([
        (
            PathBuf::from(format!("{expert_name}.mq5")),
            generated.source.into_bytes(),
        ),
        (
            PathBuf::from(format!("{expert_name}.set")),
            generated.set_file.into_bytes(),
        ),
        (
            PathBuf::from(format!("{expert_name}.tester.ini")),
            generated.tester_ini.into_bytes(),
        ),
        (
            PathBuf::from("strategy.ir.json"),
            pretty_json_bytes(&entry.payload.strategy)?,
        ),
        (
            PathBuf::from("broker-spec.json"),
            pretty_json_bytes(&entry.payload.broker)?,
        ),
        (
            PathBuf::from("export-evidence.json"),
            pretty_json_bytes(&generated.evidence)?,
        ),
        (
            PathBuf::from("risk-pack.json"),
            pretty_json_bytes(&risk_pack)?,
        ),
        (
            PathBuf::from("CHANGELOG.md"),
            deployment_changelog(&entry, &parity_hash, &generated.evidence).into_bytes(),
        ),
    ]);
    let file_records: Vec<_> = files
        .iter()
        .map(|(path, bytes)| DeploymentFileRecord {
            relative_path: path.to_string_lossy().into_owned(),
            content_hash: ContentHash::sha256(bytes),
            byte_count: bytes.len(),
        })
        .collect();
    let deployment_id = quantforge_core::stable_json_hash(&DeploymentIdentity {
        protocol_version: DEPLOYMENT_PROTOCOL_VERSION,
        certified_vault_entry_id: &entry.entry_id,
        certified_vault_entry_hash: &vault_entry_hash,
        external_parity_artifact_hash: &parity_hash,
        incubation_artifact_hash: &incubation_hash,
        candidate: &binding,
        files: &file_records,
    })
    .map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "deploy",
        Some(entry.payload.split_plan.full_data_hash.clone()),
        Some(binding.broker_spec_hash.clone()),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            (
                "vault_entry".into(),
                json!(display_path(Path::new(&request.vault_entry_path))),
            ),
            ("vault_entry_hash".into(), json!(&vault_entry_hash)),
            ("vault_entry_id".into(), json!(&entry.entry_id)),
            ("external_parity_hash".into(), json!(&parity_hash)),
            ("incubation_hash".into(), json!(&incubation_hash)),
            ("deployment_id".into(), json!(&deployment_id)),
            ("export_config".into(), json!(&generated.evidence.config)),
            ("files".into(), json!(&file_records)),
            ("live_trading_default".into(), json!(false)),
        ]),
    )?;
    let deployment_manifest = DeploymentManifest {
        schema_version: DEPLOYMENT_SCHEMA_VERSION,
        protocol_version: DEPLOYMENT_PROTOCOL_VERSION.into(),
        deployment_id: deployment_id.clone(),
        grade: StrategyGrade::Deployed,
        run_manifest,
        certified_vault_entry_source: display_path(Path::new(&request.vault_entry_path)),
        certified_vault_entry_id: entry.entry_id,
        certified_vault_entry_hash: vault_entry_hash,
        external_parity_artifact_hash: parity_hash,
        incubation_artifact_hash: incubation_hash,
        candidate: binding,
        live_trading_default: false,
        files: file_records,
    };
    files.insert(
        PathBuf::from("deployment-manifest.json"),
        pretty_json_bytes(&deployment_manifest)?,
    );
    write_directory_new(&out, &files).map_err(|error| error.to_string())?;
    Ok(DeployView {
        output_directory: display_path(&out),
        deployment_id: deployment_id.as_str().into(),
        grade: "deployed",
        expert_name,
        symbol: entry.payload.broker.symbol,
        timeframe: generated.evidence.timeframe,
        magic: generated.evidence.config.magic,
        file_count: files.len(),
        live_trading_default: false,
        certification_warnings: entry.certification.warnings.len(),
    })
}

fn verify_parity(
    artifact: &ParityArtifact,
    binding: &EvidenceBinding,
    broker: &quantforge_broker::SymbolSpecification,
    split_plan: &quantforge_quality::DataSplitPlan,
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
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&split_plan.validation.data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&binding.broker_spec_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.evidence.strategy_fingerprint != binding.strategy_fingerprint
        || artifact.evidence.broker_spec_hash != binding.broker_spec_hash
        || artifact.evidence.symbol != broker.symbol
        || artifact.evidence.live_trading_default
        || artifact.evidence.config.allow_live_trading_default
        || !artifact.evidence.mandatory_stop_loss
        || !artifact.evidence.mandatory_take_profit
        || artifact.external.engine != "mt5-strategy-tester"
        || artifact.report != recomputed
        || !artifact.report.passed
        || !artifact.report.protective_orders_present
    {
        return Err("Certified external parity artifact is unsafe or inconsistent".into());
    }
    Ok(())
}

fn deployment_changelog(
    entry: &CertifiedVaultEntry<VaultPayload>,
    parity_hash: &ContentHash,
    evidence: &ExportEvidenceCard,
) -> String {
    format!(
        "# QuantForge Deployment Changelog\n\n## Initial certified build\n\n- Vault entry: `{}`\n- Strategy fingerprint: `{}`\n- Broker specification: `{}`\n- External MT5 parity artifact: `{}`\n- Parity-passed EA source: `{}`\n- Expert: `{}`\n- Symbol/timeframe: `{}` / `{}`\n- Magic: `{}`\n- Paper incubation: `passed and reverified`\n- Live trading default: `false`\n\nThis pack reproduces the exact source and settings that passed external parity. Certification and incubation are not guarantees of future performance.\n",
        entry.entry_id,
        evidence.strategy_fingerprint,
        evidence.broker_spec_hash,
        parity_hash,
        evidence.source_hash,
        evidence.expert_name,
        evidence.symbol,
        evidence.timeframe,
        evidence.config.magic,
    )
}

fn pretty_json_bytes(value: &impl Serialize) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    Ok(bytes)
}
