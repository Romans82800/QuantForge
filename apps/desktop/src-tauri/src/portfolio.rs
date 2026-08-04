use crate::data_lab::{display_path, load_bound_broker};
use crate::workflow::{
    EvolveArtifact, ensure_new, manifest, read_json_hashed, recipe_path, write_json_new,
};
use quantforge_data::QualityGrade;
use quantforge_discover::Elite;
use quantforge_portfolio::{
    PortfolioCandidate, PortfolioConfig, PortfolioObjective, PortfolioReport, pack_portfolio,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRequest {
    databank_path: String,
    broker_path: String,
    output_path: String,
    objective: String,
    maximum_pairwise_correlation: f64,
    maximum_weight_per_strategy: f64,
    maximum_symbol_exposure: f64,
    maximum_cohort_exposure: f64,
    maximum_strategies: usize,
    minimum_return_percent: f64,
    cvar_tail_fraction: f64,
    stress_trials: usize,
    stress_block_length: usize,
    seed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioView {
    output_path: String,
    portfolio_id: String,
    source_candidates: usize,
    selected_strategies: usize,
    expected_return_percent: f64,
    maximum_drawdown_percent: f64,
    maximum_pairwise_correlation: f64,
    p05_return_percent: f64,
    cvar_return_percent: f64,
    p95_drawdown_percent: f64,
    allocations: Vec<AllocationView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AllocationView {
    fingerprint: String,
    cohort: String,
    symbol: String,
    weight: f64,
    return_percent: f64,
    drawdown_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PortfolioArtifact {
    manifest: quantforge_storage::RunManifest,
    databank_source: String,
    databank_source_hash: quantforge_core::ContentHash,
    broker_source: String,
    report: PortfolioReport,
}

#[tauri::command]
pub async fn build_portfolio(request: PortfolioRequest) -> Result<PortfolioView, String> {
    tauri::async_runtime::spawn_blocking(move || build_portfolio_sync(&request))
        .await
        .map_err(|error| format!("portfolio task failed: {error}"))?
}

fn build_portfolio_sync(request: &PortfolioRequest) -> Result<PortfolioView, String> {
    let out = ensure_new(&request.output_path, "portfolio artifact")?;
    let (artifact, databank_source_hash) =
        read_json_hashed::<EvolveArtifact>(&request.databank_path)?;
    let broker = load_bound_broker(&request.broker_path, None)?;
    let broker_spec_hash = broker.content_hash().map_err(|error| error.to_string())?;
    verify_databank(&artifact, &broker_spec_hash)?;

    let candidates: Vec<_> = artifact
        .databank
        .elites
        .iter()
        .map(|elite| PortfolioCandidate {
            strategy_fingerprint: elite.structural_fingerprint.clone(),
            symbol: broker.symbol.clone(),
            cohort: behavior_cohort(elite),
            initial_balance: artifact.databank.config.scout.initial_balance,
            return_percent: elite.metrics.return_percent,
            maximum_drawdown_percent: elite.metrics.max_drawdown_percent,
            equity_signature: elite.equity_signature.clone(),
        })
        .collect();
    let objective = match request.objective.as_str() {
        "risk_adjusted_return" => PortfolioObjective::RiskAdjustedReturn,
        "cvar" => PortfolioObjective::Cvar,
        "minimize_drawdown" => PortfolioObjective::MinimizeDrawdown,
        value => return Err(format!("unknown portfolio objective {value}")),
    };
    let config = PortfolioConfig {
        objective,
        maximum_pairwise_correlation: request.maximum_pairwise_correlation,
        maximum_weight_per_strategy: request.maximum_weight_per_strategy,
        maximum_symbol_exposure: request.maximum_symbol_exposure,
        maximum_cohort_exposure: request.maximum_cohort_exposure,
        maximum_strategies: request.maximum_strategies,
        minimum_return_percent: request.minimum_return_percent,
        cvar_tail_fraction: request.cvar_tail_fraction,
        stress_trials: request.stress_trials,
        stress_block_length: request.stress_block_length,
        seed: request.seed,
    };
    let report = pack_portfolio(
        &candidates,
        artifact.databank.data_hash.clone(),
        broker_spec_hash.clone(),
        config,
    )
    .map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "portfolio",
        Some(artifact.databank.data_hash.clone()),
        Some(broker_spec_hash),
        Some(artifact.databank.grammar_version.clone()),
        Some(report.config.seed),
        BTreeMap::from([
            ("databank".into(), recipe_path(&request.databank_path)),
            ("databank_source_hash".into(), json!(&databank_source_hash)),
            ("broker".into(), recipe_path(&request.broker_path)),
            ("protocol".into(), json!(&report.protocol_version)),
            ("portfolio_id".into(), json!(&report.portfolio_id)),
            (
                "portfolio_config".into(),
                serde_json::to_value(&report.config).map_err(|error| error.to_string())?,
            ),
            (
                "selected_fingerprints".into(),
                json!(
                    report
                        .selected
                        .iter()
                        .map(|value| &value.strategy_fingerprint)
                        .collect::<Vec<_>>()
                ),
            ),
        ]),
    )?;
    let output = PortfolioArtifact {
        manifest: run_manifest,
        databank_source: display_path(Path::new(&request.databank_path)),
        databank_source_hash,
        broker_source: display_path(Path::new(&request.broker_path)),
        report,
    };
    write_json_new(&out, &output)?;
    Ok(view(&output, &out))
}

fn verify_databank(
    artifact: &EvolveArtifact,
    broker_spec_hash: &quantforge_core::ContentHash,
) -> Result<(), String> {
    artifact
        .manifest
        .validate()
        .map_err(|error| error.to_string())?;
    artifact
        .databank
        .validate_integrity()
        .map_err(|error| error.to_string())?;
    let bank = &artifact.databank;
    if artifact.manifest.command != "evolve"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&bank.data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&bank.broker_spec_hash)
        || artifact.manifest.recipe.grammar_version.as_deref()
            != Some(quantforge_discover::GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || &bank.broker_spec_hash != broker_spec_hash
        || artifact.coverage != bank.coverage()
        || (artifact.qd_score - bank.qd_score()).abs() > 1.0e-9
        || artifact.manifest.recipe.config.get("discover_config")
            != Some(&serde_json::to_value(&bank.config).map_err(|error| error.to_string())?)
    {
        return Err("portfolio requires an intact, promotion-grade databank".into());
    }
    // Final parity gate: Discover stacks Selected-TF elites; Portfolio requires
    // an explicit M1 fidelity pass (Results → M1 fidelity), not a preference flag.
    let fidelity_verified = matches!(
        artifact.manifest.recipe.config.get("m1_fidelity_verified"),
        Some(serde_json::Value::Bool(true))
    );
    if !fidelity_verified {
        return Err(
            "databank is research-grade (Selected-TF Discover only). Run Results → M1 fidelity gate before Portfolio."
                .into(),
        );
    }
    Ok(())
}

/// Diversification group for the exposure cap. Entry-condition count plus the
/// trade-frequency and hold-time buckets is a better correlation proxy than the
/// old family label, which said nothing about how a strategy actually behaved.
fn behavior_cohort(elite: &Elite) -> String {
    format!(
        "e{}/{:?}/{:?}",
        elite.niche.entry_conditions, elite.niche.trade_frequency, elite.niche.hold_time
    )
    .to_ascii_lowercase()
}

fn view(artifact: &PortfolioArtifact, path: &Path) -> PortfolioView {
    PortfolioView {
        output_path: display_path(path),
        portfolio_id: artifact.report.portfolio_id.as_str().into(),
        source_candidates: artifact.report.source_candidate_count,
        selected_strategies: artifact.report.selected.len(),
        expected_return_percent: artifact.report.expected_return_percent,
        maximum_drawdown_percent: artifact.report.path_maximum_drawdown_percent,
        maximum_pairwise_correlation: artifact.report.maximum_observed_pairwise_correlation,
        p05_return_percent: artifact.report.stress.p05_return_percent,
        cvar_return_percent: artifact.report.stress.cvar_return_percent,
        p95_drawdown_percent: artifact.report.stress.p95_maximum_drawdown_percent,
        allocations: artifact
            .report
            .selected
            .iter()
            .map(|allocation| AllocationView {
                fingerprint: allocation.strategy_fingerprint.as_str().into(),
                cohort: allocation.cohort.clone(),
                symbol: allocation.symbol.clone(),
                weight: allocation.weight,
                return_percent: allocation.source_return_percent,
                drawdown_percent: allocation.source_maximum_drawdown_percent,
            })
            .collect(),
    }
}
