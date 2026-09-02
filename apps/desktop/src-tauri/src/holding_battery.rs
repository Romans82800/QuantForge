//! Discover-style job runner for on-demand Holding → Databank battery.

use crate::data_lab::{
    apply_history_start_year, build_decision_from_m1, build_decision_from_m1_quotes,
    load_bound_broker, load_quote_sidecar, trim_market_history_to_year,
};
use crate::databank::{
    DesktopState, EvolveArtifact, HoldingBatteryRequest, infer_quote_sidecar_path,
    persist_bank_file, persist_loaded_bank, reload_workspace_from_path, slice_bars,
};
use quantforge_core::ContentHash;
use quantforge_data::{
    BarDataset, QuoteBarDataset, bar_content_hash, build_timeframe_from_m1,
    build_timeframe_from_m1_with_quotes, quote_bar_content_hash,
};
use quantforge_discover::{
    Databank, Elite, GateResult, ProductionLaneConfig, ProductionLaneReplay, ProductionLaneReport,
    RobustnessEvidence, apply_holding_daily_corr_shrink, daily_pnl_from_trades,
    holding_factory_score, promote_selected_holding_without_robustness, run_production_lane,
};
use quantforge_eval::{IndicatorBufferCache, evaluate_strategy_cached};
use quantforge_quality::DataSplitPlan;
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1, evaluate_strategy_m1_with_quotes};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

/// Called after a factory promotion has been durably checkpointed.  Discover
/// uses this to replace its otherwise-finished live snapshot, so its dashboard
/// cannot keep reporting the pre-battery Holding/Databank counts.
pub(crate) type FactoryCheckpoint = Arc<dyn Fn(&EvolveArtifact) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryItemView {
    fingerprint: String,
    strategy_id: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    evidence: f64,
    trades: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BatteryKillMix {
    neighborhood: usize,
    monte_carlo: usize,
    folds: usize,
    m1: usize,
    deposit: usize,
    expectancy: usize,
    oos1: usize,
    correlation: usize,
    clone: usize,
    other: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingBypassView {
    promoted: usize,
    replaced: usize,
    workspace: crate::databank::DatabankWorkspace,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryJobView {
    job_id: Option<String>,
    job_kind: &'static str,
    status: &'static str,
    phase: String,
    message: String,
    databank_path: Option<String>,
    total: usize,
    queued: usize,
    running: usize,
    completed: usize,
    passed: usize,
    rejected: usize,
    holding_remaining: usize,
    databank_elites: usize,
    batteries_per_hour: f64,
    eta_seconds: Option<f64>,
    elapsed_seconds: f64,
    stop_requested: bool,
    /// Bumps whenever a strategy finishes so the UI can reload the archive.
    revision: u64,
    items: Vec<BatteryItemView>,
    kill_mix: BatteryKillMix,
    holding_before_shrink: usize,
    holding_after_shrink: usize,
    target_databank: Option<usize>,
    audit_and_graduate: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    report_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<crate::databank::DatabankWorkspace>,
}

impl BatteryJobView {
    fn idle() -> Self {
        Self {
            job_id: None,
            job_kind: "battery",
            status: "idle",
            phase: "Ready".into(),
            message: "Select Holding strategies and start the battery.".into(),
            databank_path: None,
            total: 0,
            queued: 0,
            running: 0,
            completed: 0,
            passed: 0,
            rejected: 0,
            holding_remaining: 0,
            databank_elites: 0,
            batteries_per_hour: 0.0,
            eta_seconds: None,
            elapsed_seconds: 0.0,
            stop_requested: false,
            revision: 0,
            items: Vec::new(),
            kill_mix: BatteryKillMix::default(),
            holding_before_shrink: 0,
            holding_after_shrink: 0,
            target_databank: None,
            audit_and_graduate: false,
            report_path: None,
            workspace: None,
        }
    }
}

pub struct BatteryJobState {
    pub(crate) job: Arc<RwLock<BatteryJobView>>,
    pub(crate) stop: Arc<AtomicBool>,
}

impl Default for BatteryJobState {
    fn default() -> Self {
        Self {
            job: Arc::new(RwLock::new(BatteryJobView::idle())),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[tauri::command]
pub fn get_holding_battery_job(
    state: State<'_, BatteryJobState>,
) -> Result<BatteryJobView, String> {
    state
        .job
        .read()
        .map(|view| view.clone())
        .map_err(|_| "battery job state is unavailable".into())
}

#[tauri::command]
pub fn stop_holding_battery(state: State<'_, BatteryJobState>) -> Result<BatteryJobView, String> {
    let mut view = state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable".to_owned())?;
    if view.status != "running" {
        return Err("no active Holding battery job can be stopped".into());
    }
    state.stop.store(true, Ordering::SeqCst);
    view.stop_requested = true;
    view.phase = "Stopping after the current strategy".into();
    view.message = "Stop requested — finishing the in-flight battery, then checkpointing.".into();
    Ok(view.clone())
}

/// Explicit research override: graduate the current Holding cohort without
/// running CPCV, walk-forward, Monte Carlo or parameter-neighborhood tests.
/// Holding admission's existing basic/M1 checks have already run; this command
/// only changes where those entries are stored.
#[tauri::command]
pub fn promote_holding_without_robustness(
    desktop: State<'_, DesktopState>,
    state: State<'_, BatteryJobState>,
) -> Result<HoldingBypassView, String> {
    if state.is_busy()? {
        return Err(
            "stop or finish the Holding battery before promoting Holding without robustness".into(),
        );
    }
    let (mut bank, databank_path, count, legacy_read_only) = {
        let loaded = desktop
            .loaded
            .read()
            .map_err(|_| "desktop databank state is unavailable".to_owned())?;
        let loaded = loaded.as_ref().ok_or_else(|| {
            "no databank is loaded — open the Discover checkpoint first".to_owned()
        })?;
        (
            loaded.bank.clone(),
            loaded.databank_path.clone(),
            loaded.bank.holding.len(),
            loaded.legacy_read_only,
        )
    };
    if legacy_read_only {
        return Err(
            "Schema-v5 databanks are read-only. Run a fresh Discover archive before promoting Holding."
                .into(),
        );
    }
    if count == 0 {
        return Err("Holding is empty".into());
    }

    let result = quantforge_discover::promote_all_holding_without_robustness(&mut bank);
    persist_loaded_bank(&databank_path, &bank, &desktop)?;
    let workspace = reload_workspace_from_path(Path::new(&databank_path), &desktop)?;
    Ok(HoldingBypassView {
        promoted: result.promoted,
        replaced: result.replaced,
        workspace,
    })
}

impl BatteryJobState {
    pub(crate) fn is_busy(&self) -> Result<bool, String> {
        let current = self
            .job
            .read()
            .map_err(|_| "battery job state is unavailable")?;
        let stale_running = current.status == "running"
            && current.total > 0
            && current.completed >= current.total
            && current.running == 0;
        Ok(current.status == "running" && !stale_running)
    }
}

const DEFAULT_FACTORY_CORR: f64 = 0.5;

fn optional_positive(value: Option<usize>) -> Option<usize> {
    value.filter(|&n| n > 0)
}

#[tauri::command]
pub fn start_holding_battery_job(
    request: HoldingBatteryRequest,
    desktop: State<'_, DesktopState>,
    state: State<'_, BatteryJobState>,
) -> Result<BatteryJobView, String> {
    if state.is_busy()? {
        return Err("a Holding battery job is already running".into());
    }
    if !request.ranked && request.fingerprints.is_empty() {
        return Err("select Holding strategies or start the ranked factory".into());
    }

    let snapshot = snapshot_loaded_archive(&desktop)?;
    if snapshot.legacy_read_only {
        return Err(
            "Schema-v5 databanks are read-only. Run a fresh Discover archive before Holding battery."
                .into(),
        );
    }

    state.stop.store(false, Ordering::SeqCst);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis() as u64;
    let started = BatteryJobView {
        job_id: Some(format!("battery-{now_ms}")),
        job_kind: if request.audit_and_graduate {
            "audit_graduate"
        } else {
            "battery"
        },
        status: "running",
        phase: if request.shrink_first {
            "Preparing factory".into()
        } else if request.ranked {
            "Ranking Holding queue".into()
        } else {
            "Loading market data".into()
        },
        message: if request.audit_and_graduate {
            "Running the full battery as an audit, then graduating every Holding strategy regardless of result.".into()
        } else if request.ranked {
            "Ranking the current Holding cohort, then running the robustness battery one strategy at a time.".into()
        } else {
            format!(
                "Queued {} Holding strategies for the robustness battery.",
                request.fingerprints.len()
            )
        },
        databank_path: Some(snapshot.databank_path.clone()),
        total: 0,
        queued: 0,
        running: 0,
        completed: 0,
        passed: 0,
        rejected: 0,
        holding_remaining: snapshot.artifact.databank.holding.len(),
        databank_elites: snapshot.artifact.databank.elites.len(),
        batteries_per_hour: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0.0,
        stop_requested: false,
        revision: 0,
        items: Vec::new(),
        kill_mix: BatteryKillMix::default(),
        holding_before_shrink: snapshot.artifact.databank.holding.len(),
        holding_after_shrink: snapshot.artifact.databank.holding.len(),
        target_databank: request.target_databank,
        audit_and_graduate: request.audit_and_graduate,
        report_path: None,
        workspace: None,
    };
    *state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable")? = started.clone();

    let desktop = desktop.inner().clone();
    let job = Arc::clone(&state.job);
    let stop = Arc::clone(&state.stop);
    let job_for_err = Arc::clone(&job);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) =
            run_factory_or_battery(job, stop, snapshot, request, Some(desktop), None)
        {
            if let Ok(mut view) = job_for_err.write() {
                view.status = "failed";
                view.phase = "Stopped with an error".into();
                view.message = error;
                view.running = 0;
            }
        }
    });

    Ok(started)
}

/// Run the fixed H4 Production Lane v1 against the complete frozen Holding
/// cohort. Eligibility and ranking use Development M1 replays only; the sealed
/// partition is never passed to the selector or evaluator.
#[tauri::command]
pub fn start_production_lane_job(
    desktop: State<'_, DesktopState>,
    state: State<'_, BatteryJobState>,
) -> Result<BatteryJobView, String> {
    if state.is_busy()? {
        return Err("a Holding job is already running".into());
    }
    let snapshot = snapshot_loaded_archive(&desktop)?;
    if snapshot.legacy_read_only {
        return Err(
            "Schema-v5 databanks are read-only. Run a fresh H4 Discover archive first.".into(),
        );
    }
    let total = snapshot.artifact.databank.holding.len();
    if total == 0 {
        return Err("Holding is empty".into());
    }
    if snapshot.sealed_fraction <= 0.0 {
        return Err("Production Lane requires a non-zero sealed partition".into());
    }
    let timeframe = crate::databank::archive_decision_timeframe(
        &snapshot.artifact,
        Path::new(&snapshot.databank_path),
    );
    if timeframe != "H4" {
        return Err(format!(
            "This is a {timeframe} Holding archive. Production Lane v1 is H4-only; use Run full robustness battery for this cohort. Nothing was evaluated or promoted."
        ));
    }

    state.stop.store(false, Ordering::SeqCst);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis() as u64;
    let items = snapshot
        .artifact
        .databank
        .holding
        .iter()
        .map(|elite| BatteryItemView {
            fingerprint: elite.structural_fingerprint.to_string(),
            strategy_id: elite.strategy.id.clone(),
            status: "queued",
            reason: None,
            evidence: elite.evidence.total,
            trades: elite.metrics.trade_count,
        })
        .collect::<Vec<_>>();
    let started = BatteryJobView {
        job_id: Some(format!("production-lane-{now_ms}")),
        job_kind: "production_lane",
        status: "running",
        phase: "Verifying H4 Development data".into(),
        message: format!(
            "Frozen cohort: {total}. Reconstructing H4 from M1 and matching the archive hash before replay."
        ),
        databank_path: Some(snapshot.databank_path.clone()),
        total,
        queued: total,
        running: 0,
        completed: 0,
        passed: 0,
        rejected: 0,
        holding_remaining: total,
        databank_elites: snapshot.artifact.databank.elites.len(),
        batteries_per_hour: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0.0,
        stop_requested: false,
        revision: 0,
        items,
        kill_mix: BatteryKillMix::default(),
        holding_before_shrink: total,
        holding_after_shrink: total,
        target_databank: None,
        audit_and_graduate: false,
        report_path: None,
        workspace: None,
    };
    *state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable")? = started.clone();

    let desktop = desktop.inner().clone();
    let job = Arc::clone(&state.job);
    let stop = Arc::clone(&state.stop);
    let job_for_err = Arc::clone(&job);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_production_lane_job(job, stop, snapshot, desktop) {
            if let Ok(mut view) = job_for_err.write() {
                view.status = "failed";
                view.phase = "Production Lane stopped with an error".into();
                view.message = error;
                view.running = 0;
                view.queued = 0;
            }
        }
    });
    Ok(started)
}

/// Overnight Discover → shrink → ranked battery. Safe to call after a checkpoint write.
pub(crate) fn spawn_factory_from_archive(
    databank_path: String,
    request: HoldingBatteryRequest,
    state: &BatteryJobState,
    factory_checkpoint: Option<FactoryCheckpoint>,
) -> Result<BatteryJobView, String> {
    if state.is_busy()? {
        return Err("a Holding battery job is already running".into());
    }
    let snapshot = snapshot_archive_file(&databank_path)?;
    state.stop.store(false, Ordering::SeqCst);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis() as u64;
    let started = BatteryJobView {
        job_id: Some(format!("factory-{now_ms}")),
        job_kind: "battery",
        status: "running",
        phase: "Preparing factory".into(),
        message: "Discover finished — ranking the current Holding cohort for the battery queue."
            .into(),
        databank_path: Some(databank_path),
        total: 0,
        queued: 0,
        running: 0,
        completed: 0,
        passed: 0,
        rejected: 0,
        holding_remaining: snapshot.artifact.databank.holding.len(),
        databank_elites: snapshot.artifact.databank.elites.len(),
        batteries_per_hour: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0.0,
        stop_requested: false,
        revision: 0,
        items: Vec::new(),
        kill_mix: BatteryKillMix::default(),
        holding_before_shrink: snapshot.artifact.databank.holding.len(),
        holding_after_shrink: snapshot.artifact.databank.holding.len(),
        target_databank: request.target_databank,
        audit_and_graduate: false,
        report_path: None,
        workspace: None,
    };
    *state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable")? = started.clone();
    let job = Arc::clone(&state.job);
    let stop = Arc::clone(&state.stop);
    let job_for_err = Arc::clone(&job);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) =
            run_factory_or_battery(job, stop, snapshot, request, None, factory_checkpoint)
        {
            if let Ok(mut view) = job_for_err.write() {
                view.status = "failed";
                view.phase = "Stopped with an error".into();
                view.message = error;
                view.running = 0;
            }
        }
    });
    Ok(started)
}

struct ArchiveSnapshot {
    artifact: EvolveArtifact,
    databank_path: String,
    source: String,
    broker_path: String,
    metadata_path: Option<String>,
    m1_source: String,
    m1_metadata_path: Option<String>,
    validation_fraction: f64,
    sealed_fraction: f64,
    legacy_read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProductionLaneArtifact {
    schema_version: u16,
    source_databank_path: String,
    source_artifact_hash: ContentHash,
    decision_timeframe: &'static str,
    sealed_start_timestamp_ms: i64,
    sealed_fraction: f64,
    validation_fraction: f64,
    report: ProductionLaneReport,
}

fn snapshot_loaded_archive(desktop: &DesktopState) -> Result<ArchiveSnapshot, String> {
    let loaded = desktop
        .loaded
        .read()
        .map_err(|_| "desktop databank state is unavailable".to_owned())?;
    let loaded = loaded
        .as_ref()
        .ok_or_else(|| "no databank is loaded — open the Discover checkpoint first".to_owned())?;
    Ok(ArchiveSnapshot {
        artifact: {
            let bytes = std::fs::read(&loaded.databank_path).map_err(|error| error.to_string())?;
            let mut artifact: EvolveArtifact =
                serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            artifact.databank = loaded.bank.clone();
            artifact
        },
        databank_path: loaded.databank_path.clone(),
        source: loaded.source.clone(),
        broker_path: loaded.broker.clone(),
        metadata_path: loaded.metadata_path.clone(),
        m1_source: loaded.m1_source.clone().ok_or_else(|| {
            "This archive does not bind an M1 source; cannot run Holding battery.".to_owned()
        })?,
        m1_metadata_path: loaded.m1_metadata_path.clone(),
        validation_fraction: loaded.validation_fraction,
        sealed_fraction: loaded.sealed_fraction,
        legacy_read_only: loaded.legacy_read_only,
    })
}

fn snapshot_archive_file(databank_path: &str) -> Result<ArchiveSnapshot, String> {
    let path = Path::new(databank_path);
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let artifact: crate::databank::EvolveArtifact =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let source = artifact.source.clone();
    let broker_path = artifact.broker.clone();
    let m1_source = crate::databank::manifest_path(&artifact, "m1_source").ok_or_else(|| {
        "This archive does not bind an M1 source; cannot run the factory.".to_owned()
    })?;
    let validation_fraction =
        crate::databank::manifest_fraction(&artifact, "validation_fraction", 0.0);
    let sealed_fraction =
        crate::databank::manifest_fraction(&artifact, "sealed_fraction", 1.0 / 3.0);
    let metadata_path = Path::new(&source)
        .with_extension("metadata.csv")
        .is_file()
        .then(|| {
            Path::new(&source)
                .with_extension("metadata.csv")
                .display()
                .to_string()
        });
    let m1_metadata_path = Path::new(&m1_source)
        .with_extension("metadata.csv")
        .is_file()
        .then(|| {
            Path::new(&m1_source)
                .with_extension("metadata.csv")
                .display()
                .to_string()
        });
    Ok(ArchiveSnapshot {
        artifact,
        databank_path: databank_path.to_string(),
        source,
        broker_path,
        metadata_path,
        m1_source,
        m1_metadata_path,
        validation_fraction,
        sealed_fraction,
        legacy_read_only: false,
    })
}

fn run_production_lane_job(
    job: Arc<RwLock<BatteryJobView>>,
    stop: Arc<AtomicBool>,
    mut snapshot: ArchiveSnapshot,
    desktop: DesktopState,
) -> Result<(), String> {
    let started = Instant::now();
    let source_bytes = std::fs::read(&snapshot.databank_path).map_err(|error| error.to_string())?;
    let source_artifact_hash = ContentHash::sha256(&source_bytes);

    let mut source = crate::data_lab::load_data_source(
        &snapshot.source,
        snapshot.metadata_path.as_deref(),
        None,
    )?;
    let mut m1 = crate::data_lab::load_data_source(
        &snapshot.m1_source,
        snapshot.m1_metadata_path.as_deref(),
        None,
    )?;
    let broker = load_bound_broker(&snapshot.broker_path, source.metadata.as_ref())?;
    load_bound_broker(&snapshot.broker_path, m1.metadata.as_ref())?;
    let mut quotes = infer_quote_sidecar_path(&snapshot.m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut source.dataset,
        &mut m1.dataset,
        quotes.as_mut(),
        snapshot.artifact.databank.config.history_start_year,
    )?;

    update_phase(
        &job,
        "Verifying H4 Development data",
        "Rebuilding H4 from the bound M1 chronology; the archive Development hash must match exactly.",
    );
    let h4 = match quotes.as_ref() {
        Some(quote_dataset) => build_timeframe_from_m1_with_quotes(
            &m1.dataset,
            quote_dataset,
            broker.point,
            14_400_000,
            None,
        ),
        None => build_timeframe_from_m1(&m1.dataset, 14_400_000, None),
    }
    .map_err(|error| format!("cannot reconstruct H4 from M1: {error}"))?;
    let plan =
        DataSplitPlan::chronological(&h4, snapshot.validation_fraction, snapshot.sealed_fraction)
            .map_err(|error| error.to_string())?;
    if plan.sealed_final.bar_count == 0 {
        return Err("Production Lane requires a non-empty sealed final partition".into());
    }
    let development = slice_bars(&h4, 0, plan.development.bar_count)?;
    if development.data_hash != snapshot.artifact.databank.data_hash {
        return Err(format!(
            "This Holding archive is not bound to the reconstructed H4 Development data (archive {}, reconstructed {}). Nothing was evaluated or promoted.",
            snapshot.artifact.databank.data_hash, development.data_hash,
        ));
    }
    let development_start = development
        .bars
        .first()
        .map(|bar| bar.timestamp_ms)
        .ok_or_else(|| "H4 Development partition is empty".to_owned())?;
    let development_end = h4
        .bars
        .get(plan.development.bar_count)
        .map(|bar| bar.timestamp_ms)
        .ok_or_else(|| "H4 split has no later validation/sealed boundary".to_owned())?;
    let sealed_start_index = plan.development.bar_count + plan.validation.bar_count;
    let sealed_start = h4
        .bars
        .get(sealed_start_index)
        .map(|bar| bar.timestamp_ms)
        .ok_or_else(|| "H4 sealed partition boundary is missing".to_owned())?;
    let m1_development = slice_dataset_by_time(&m1.dataset, development_start, development_end)?;
    let quote_development = quotes.as_ref().map(|quote_dataset| {
        slice_quotes_by_time(quote_dataset, development_start, development_end)
    });

    let candidates = snapshot.artifact.databank.holding.clone();
    let judge = JudgeConfig {
        initial_balance: snapshot.artifact.databank.config.scout.initial_balance,
        costs: snapshot.artifact.databank.config.scout.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: snapshot.artifact.databank.config.scout.indicator_engine,
        entry_window: snapshot.artifact.databank.config.scout.entry_window,
    };
    update_phase(
        &job,
        "Replaying frozen H4 cohort on Development M1",
        "3/6/12-month evidence and ranking are computed without loading validation or sealed rows.",
    );

    let mut replays = BTreeMap::<String, ProductionLaneReplay>::new();
    let mut completed = 0usize;
    const REPLAY_BATCH: usize = 4;
    for batch in candidates.chunks(REPLAY_BATCH) {
        if stop.load(Ordering::SeqCst) {
            if let Ok(mut view) = job.write() {
                view.status = "stopped";
                view.phase = "Production Lane stopped safely".into();
                view.message = "No selection was made and no Holding strategy was promoted.".into();
                view.running = 0;
                view.queued = 0;
                view.stop_requested = false;
            }
            return Ok(());
        }
        for elite in batch {
            mark_item(&job, elite.structural_fingerprint.as_str(), "running", None);
        }
        let outcomes = batch
            .par_iter()
            .map(|elite| {
                let result = match quote_development.as_ref() {
                    Some(quote_dataset) => evaluate_strategy_m1_with_quotes(
                        &elite.strategy,
                        &development,
                        &m1_development,
                        quote_dataset,
                        &broker,
                        &judge,
                    ),
                    None => evaluate_strategy_m1(
                        &elite.strategy,
                        &development,
                        &m1_development,
                        &broker,
                        &judge,
                    ),
                };
                (elite.structural_fingerprint.to_string(), result)
            })
            .collect::<Vec<_>>();
        for (fingerprint, result) in outcomes {
            completed += 1;
            match result {
                Ok(result) => {
                    mark_item(&job, &fingerprint, "replayed", None);
                    replays.insert(
                        fingerprint,
                        ProductionLaneReplay {
                            metrics: result.metrics,
                            trades: result.trades,
                        },
                    );
                }
                Err(error) => {
                    mark_item(
                        &job,
                        &fingerprint,
                        "rejected",
                        Some(format!("Development M1 replay failed: {error}")),
                    );
                }
            }
        }
        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        if let Ok(mut view) = job.write() {
            view.completed = completed;
            view.queued = candidates.len().saturating_sub(completed);
            view.running = 0;
            view.batteries_per_hour = completed as f64 / elapsed * 3600.0;
            view.elapsed_seconds = elapsed;
            let remaining = candidates.len().saturating_sub(completed);
            view.eta_seconds =
                (completed > 0).then_some(remaining as f64 / (completed as f64 / elapsed));
            view.phase = format!("Production Lane replay {completed}/{}", candidates.len());
            view.message = format!(
                "Development-only M1 replay: {completed}/{} complete.",
                candidates.len()
            );
        }
    }

    update_phase(
        &job,
        "Selecting Production Lane cohort",
        "Applying fixed basic gates, 6/12-month stability and expectancy × √trades ranking.",
    );
    let report = run_production_lane(
        &snapshot.artifact.databank,
        &candidates,
        &replays,
        development.data_hash.clone(),
        development_start,
        development_end,
        ProductionLaneConfig::default(),
    )?;
    for row in &report.rows {
        let (status, reason) = if row.selected {
            ("selected", None)
        } else if row.eligible {
            (
                "eligible",
                Some("Eligible but outside the top-20%/diversity budget".into()),
            )
        } else {
            ("rejected", Some(row.rejection_reasons.join("; ")))
        };
        mark_item(&job, &row.fingerprint, status, reason);
    }

    let report_path = production_lane_report_path(&snapshot.databank_path)?;
    let artifact = ProductionLaneArtifact {
        schema_version: 1,
        source_databank_path: snapshot.databank_path.clone(),
        source_artifact_hash,
        decision_timeframe: "H4",
        sealed_start_timestamp_ms: sealed_start,
        sealed_fraction: snapshot.sealed_fraction,
        validation_fraction: snapshot.validation_fraction,
        report: report.clone(),
    };
    quantforge_storage::write_json_new(&report_path, &artifact)
        .map_err(|error| format!("cannot write immutable Production Lane report: {error}"))?;

    if !report.selected_fingerprints.is_empty() {
        let selected = report
            .selected_fingerprints
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        promote_selected_holding_without_robustness(
            &mut snapshot.artifact.databank,
            &selected,
            Some(GateResult {
                name: "production_lane_v1".into(),
                passed: true,
                detail: format!(
                    "Selected using H4 Development only: 6/12-month stability and {}. Sealed final was not opened.",
                    report.score_formula
                ),
            }),
        );
        persist_snapshot(&mut snapshot)?;
    }
    // Refresh the shared backend state from the exact bytes just written and
    // carry the resulting workspace in the completed job. The frontend can
    // update atomically instead of racing a second polling-triggered reload.
    let workspace = reload_workspace_from_path(Path::new(&snapshot.databank_path), &desktop)?;

    let elapsed = started.elapsed().as_secs_f64();
    if let Ok(mut view) = job.write() {
        view.status = "completed";
        view.phase = "Production Lane complete".into();
        view.message = format!(
            "{} frozen → {} Development-eligible → {} selected and promoted. Sealed final remained unopened.",
            report.source_cohort_size, report.eligible, report.selected
        );
        view.completed = candidates.len();
        view.queued = 0;
        view.running = 0;
        view.passed = report.selected;
        view.rejected = report.source_cohort_size.saturating_sub(report.eligible);
        view.holding_remaining = snapshot.artifact.databank.holding.len();
        view.databank_elites = snapshot.artifact.databank.elites.len();
        view.elapsed_seconds = elapsed;
        view.eta_seconds = Some(0.0);
        view.stop_requested = false;
        view.revision += 1;
        view.report_path = Some(report_path.display().to_string());
        view.workspace = Some(workspace);
    }
    Ok(())
}

fn slice_dataset_by_time(
    dataset: &BarDataset,
    start_timestamp_ms: i64,
    end_timestamp_ms_exclusive: i64,
) -> Result<BarDataset, String> {
    let bars = dataset
        .bars
        .iter()
        .filter(|bar| {
            bar.timestamp_ms >= start_timestamp_ms && bar.timestamp_ms < end_timestamp_ms_exclusive
        })
        .cloned()
        .collect::<Vec<_>>();
    if bars.is_empty() {
        return Err("no M1 rows cover the H4 Development partition".into());
    }
    Ok(BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: dataset.delimiter,
        source_timezone: dataset.source_timezone.clone(),
        bars,
    })
}

fn slice_quotes_by_time(
    quotes: &QuoteBarDataset,
    start_timestamp_ms: i64,
    end_timestamp_ms_exclusive: i64,
) -> QuoteBarDataset {
    let bars = quotes
        .bars
        .iter()
        .filter(|bar| {
            bar.timestamp_ms >= start_timestamp_ms && bar.timestamp_ms < end_timestamp_ms_exclusive
        })
        .cloned()
        .collect::<Vec<_>>();
    QuoteBarDataset {
        data_hash: quote_bar_content_hash(&bars),
        source_rows: bars.len(),
        source_timezone: quotes.source_timezone.clone(),
        schema_version: quotes.schema_version,
        source_model: quotes.source_model,
        bars,
    }
}

fn production_lane_report_path(databank_path: &str) -> Result<PathBuf, String> {
    let path = Path::new(databank_path);
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("databank");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    Ok(path.with_file_name(format!("{stem}_production_lane_v1_{timestamp}.json")))
}

fn ranked_holding_fingerprints(bank: &Databank, limit: Option<usize>) -> Vec<String> {
    let mut rows: Vec<_> = bank.holding.iter().collect();
    rows.sort_by(|left, right| {
        holding_factory_score(right.metrics.trade_count, right.metrics.expectancy_r)
            .partial_cmp(&holding_factory_score(
                left.metrics.trade_count,
                left.metrics.expectancy_r,
            ))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.metrics.trade_count.cmp(&left.metrics.trade_count))
    });
    let fingerprints = rows
        .into_iter()
        .map(|elite| elite.structural_fingerprint.to_string());
    match limit {
        Some(n) => fingerprints.take(n).collect(),
        None => fingerprints.collect(),
    }
}

fn persist_snapshot(snapshot: &mut ArchiveSnapshot) -> Result<(), String> {
    snapshot.artifact.coverage = snapshot.artifact.databank.coverage();
    snapshot.artifact.qd_score = snapshot.artifact.databank.qd_score();
    quantforge_storage::write_json_replacing(Path::new(&snapshot.databank_path), &snapshot.artifact)
        .map_err(|error| error.to_string())
}

/// Persist a completed promotion immediately and, for an interactive desktop
/// job, publish the exact refreshed workspace. This makes a passed candidate
/// appear in Databank before the next candidate begins, while failed names
/// remain visible in Holding.
fn checkpoint_battery_promotion(
    snapshot: &mut ArchiveSnapshot,
    desktop: Option<&DesktopState>,
    job: &Arc<RwLock<BatteryJobView>>,
    factory_checkpoint: Option<&FactoryCheckpoint>,
) -> Result<(), String> {
    persist_snapshot(snapshot)?;
    if let Some(publish) = factory_checkpoint {
        publish(&snapshot.artifact);
    }
    let workspace = desktop
        .map(|desktop| reload_workspace_from_path(Path::new(&snapshot.databank_path), desktop))
        .transpose()?;
    if let Ok(mut view) = job.write() {
        if let Some(workspace) = workspace {
            view.workspace = Some(workspace);
        }
        // This is an archive checkpoint revision, not merely a progress tick.
        view.revision += 1;
    }
    Ok(())
}

fn run_factory_or_battery(
    job: Arc<RwLock<BatteryJobView>>,
    stop: Arc<AtomicBool>,
    mut snapshot: ArchiveSnapshot,
    request: HoldingBatteryRequest,
    desktop: Option<DesktopState>,
    factory_checkpoint: Option<FactoryCheckpoint>,
) -> Result<(), String> {
    let holding_before = snapshot.artifact.databank.holding.len();
    if request.shrink_first {
        update_phase(
            &job,
            "Shrinking Holding",
            "Dropping daily P/L clones before the battery.",
        );
        shrink_holding_snapshot(
            &mut snapshot,
            request.max_correlation.unwrap_or(DEFAULT_FACTORY_CORR),
        )?;
        persist_snapshot(&mut snapshot)?;
    }
    let holding_after = snapshot.artifact.databank.holding.len();
    let queue_take = optional_positive(request.queue_limit);
    let target_databank = (!request.audit_and_graduate)
        .then(|| optional_positive(request.target_databank))
        .flatten();
    let fingerprints = if request.ranked || request.fingerprints.is_empty() {
        ranked_holding_fingerprints(&snapshot.artifact.databank, queue_take)
    } else {
        request.fingerprints.clone()
    };
    if fingerprints.is_empty() {
        return Err("Holding is empty after shrink — nothing to battery".into());
    }
    let items: Vec<_> = fingerprints
        .iter()
        .filter_map(|fingerprint| {
            snapshot
                .artifact
                .databank
                .holding
                .iter()
                .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
                .map(|elite| BatteryItemView {
                    fingerprint: fingerprint.clone(),
                    strategy_id: elite.strategy.id.clone(),
                    status: "queued",
                    reason: None,
                    evidence: elite.evidence.total,
                    trades: elite.metrics.trade_count,
                })
        })
        .collect();
    if items.is_empty() {
        return Err("none of the queued fingerprints are still in Holding".into());
    }
    if let Ok(mut view) = job.write() {
        view.total = items.len();
        view.queued = items.len();
        view.items = items;
        view.holding_before_shrink = holding_before;
        view.holding_after_shrink = holding_after;
        view.holding_remaining = snapshot.artifact.databank.holding.len();
        view.databank_elites = snapshot.artifact.databank.elites.len();
        view.target_databank = target_databank;
        view.revision += 1;
        view.phase = "Loading market data".into();
        let stop_note = match target_databank {
            Some(n) => format!("Battery until {n} Databank names."),
            None => "Battery everyone queued; keep every passer.".to_owned(),
        };
        let queue_stage = if request.shrink_first {
            "after optional correlation shrink"
        } else {
            "ready"
        };
        view.message = format!(
            "Funnel {} Holding → {} {queue_stage} → {} queued. {stop_note}",
            holding_before,
            holding_after,
            fingerprints.len(),
        );
    }

    run_battery_job(
        job,
        stop,
        snapshot,
        fingerprints,
        target_databank,
        desktop,
        factory_checkpoint,
        request.audit_and_graduate,
    )
}

fn shrink_holding_snapshot(
    snapshot: &mut ArchiveSnapshot,
    max_correlation: f64,
) -> Result<(), String> {
    let allowed = [0.3, 0.4, 0.5, 0.6];
    if !allowed
        .iter()
        .any(|value| (max_correlation - value).abs() < 1e-9)
    {
        return Err("pick a daily P/L correlation cap of 0.3, 0.4, 0.5 or 0.6".into());
    }
    let loaded = crate::data_lab::load_data_source(
        &snapshot.source,
        snapshot.metadata_path.as_deref(),
        None,
    )?;
    let mut dataset = loaded.dataset;
    apply_history_start_year(
        &mut dataset,
        snapshot.artifact.databank.config.history_start_year,
    )?;
    let broker = load_bound_broker(&snapshot.broker_path, loaded.metadata.as_ref())?;
    let plan = DataSplitPlan::chronological(
        &dataset,
        snapshot.validation_fraction,
        snapshot.sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let development = slice_bars(&dataset, 0, plan.development.bar_count)?;
    let cache = IndicatorBufferCache::new(development.bars.len());
    let scout = snapshot.artifact.databank.config.scout.clone();
    let timezone = broker.timezone.clone();
    let daily_pnl: Vec<_> = snapshot
        .artifact
        .databank
        .holding
        .par_iter()
        .map(|elite| {
            evaluate_strategy_cached(&elite.strategy, &development, &broker, &scout, &cache)
                .map(|result| daily_pnl_from_trades(&result.trades, &timezone))
                .unwrap_or_default()
        })
        .collect();
    apply_holding_daily_corr_shrink(&mut snapshot.artifact.databank, &daily_pnl, max_correlation);
    Ok(())
}

fn run_battery_job(
    job: Arc<RwLock<BatteryJobView>>,
    stop: Arc<AtomicBool>,
    mut snapshot: ArchiveSnapshot,
    fingerprints: Vec<String>,
    target_databank: Option<usize>,
    desktop: Option<DesktopState>,
    factory_checkpoint: Option<FactoryCheckpoint>,
    audit_and_graduate: bool,
) -> Result<(), String> {
    let started = Instant::now();
    update_phase(
        &job,
        "Loading market data",
        "Loading Development / M1 partitions for the battery…",
    );

    let mut decision_source = crate::data_lab::load_data_source(
        &snapshot.source,
        snapshot.metadata_path.as_deref(),
        None,
    )?;
    let mut m1 = crate::data_lab::load_data_source(
        &snapshot.m1_source,
        snapshot.m1_metadata_path.as_deref(),
        None,
    )?;
    let broker = load_bound_broker(&snapshot.broker_path, decision_source.metadata.as_ref())?;
    load_bound_broker(&snapshot.broker_path, m1.metadata.as_ref())?;
    let mut quote_dataset = infer_quote_sidecar_path(&snapshot.m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    // Rebuild the selected timeframe from the bound M1 chronology. The source
    // file supplies the H1 grid only; H4 archives commonly retain an H1 source
    // path for display, so its bars can never be used blindly for promotion.
    trim_market_history_to_year(
        &mut decision_source.dataset,
        &mut m1.dataset,
        quote_dataset.as_mut(),
        snapshot.artifact.databank.config.history_start_year,
    )?;
    let h1 = match quote_dataset.as_ref() {
        Some(quotes) => build_decision_from_m1_quotes(
            &m1.dataset,
            Some(&decision_source.dataset),
            quotes,
            broker.point,
        ),
        None => build_decision_from_m1(&m1.dataset, Some(&decision_source.dataset)),
    }
    .map_err(|error| format!("cannot reconstruct H1 from M1: {error}"))?;
    let h1_plan =
        DataSplitPlan::chronological(&h1, snapshot.validation_fraction, snapshot.sealed_fraction)
            .map_err(|error| error.to_string())?;
    let h1_development = slice_bars(&h1, 0, h1_plan.development.bar_count)?;
    let (decision_timeframe, development, development_end) = if h1_development.data_hash
        == snapshot.artifact.databank.data_hash
    {
        let end = h1
            .bars
            .get(h1_plan.development.bar_count)
            .map(|bar| bar.timestamp_ms)
            .ok_or_else(|| "H1 split has no later validation/sealed boundary".to_owned())?;
        ("H1", h1_development, end)
    } else {
        let h4 = match quote_dataset.as_ref() {
            Some(quotes) => build_timeframe_from_m1_with_quotes(
                &m1.dataset,
                quotes,
                broker.point,
                14_400_000,
                None,
            ),
            None => build_timeframe_from_m1(&m1.dataset, 14_400_000, None),
        }
        .map_err(|error| format!("cannot reconstruct H4 from M1: {error}"))?;
        let h4_plan = DataSplitPlan::chronological(
            &h4,
            snapshot.validation_fraction,
            snapshot.sealed_fraction,
        )
        .map_err(|error| error.to_string())?;
        let h4_development = slice_bars(&h4, 0, h4_plan.development.bar_count)?;
        if h4_development.data_hash == snapshot.artifact.databank.data_hash {
            let end = h4
                .bars
                .get(h4_plan.development.bar_count)
                .map(|bar| bar.timestamp_ms)
                .ok_or_else(|| "H4 split has no later validation/sealed boundary".to_owned())?;
            ("H4", h4_development, end)
        } else {
            return Err(format!(
                "This Holding archive is not bound to the reconstructed H1 or H4 Development data (archive {}, H1 {}, H4 {}). Nothing was evaluated or promoted.",
                snapshot.artifact.databank.data_hash,
                h1_development.data_hash,
                h4_development.data_hash,
            ));
        }
    };
    let development_start = development
        .bars
        .first()
        .map(|bar| bar.timestamp_ms)
        .ok_or_else(|| "H4 Development partition is empty".to_owned())?;
    let m1_development = slice_dataset_by_time(&m1.dataset, development_start, development_end)?;
    let quote_development = quote_dataset
        .as_ref()
        .map(|quotes| slice_quotes_by_time(quotes, development_start, development_end));
    // Older Holding archives bind the complete M1 execution stream. This is
    // safe: the judge receives Development decision bars only and iterates its
    // M1 cursor only through those decision-bar intervals. Newer archives may
    // instead bind the time-clipped Development M1 stream. Accept either exact
    // binding; never substitute a third dataset.
    let (m1_owned, quote_for_battery, execution_binding) = if snapshot
        .artifact
        .databank
        .execution_data_hash
        == m1.dataset.data_hash
    {
        (m1.dataset.clone(), quote_dataset.clone(), "full M1")
    } else if snapshot.artifact.databank.execution_data_hash == m1_development.data_hash {
        (m1_development, quote_development, "Development M1")
    } else {
        return Err(format!(
            "This Holding archive is not bound to either approved {decision_timeframe} M1 execution dataset (archive {}, full {}, Development {}). Nothing was evaluated or promoted.",
            snapshot.artifact.databank.execution_data_hash,
            m1.dataset.data_hash,
            m1_development.data_hash,
        ));
    };

    update_phase(
        &job,
        if audit_and_graduate {
            "Running full battery audit"
        } else {
            "Running battery"
        },
        &if audit_and_graduate {
            format!(
                "Verified {decision_timeframe} Development decisions with the hash-bound {execution_binding} execution stream. Every result is recorded; every tested strategy will move to Databank."
            )
        } else {
            format!(
                "Verified {decision_timeframe} Development decisions with the hash-bound {execution_binding} execution stream. Only full passes move to Databank; failures stay in Holding."
            )
        },
    );

    let mut passed = 0usize;
    let mut rejected = 0usize;
    let mut completed = 0usize;
    let mut dirty = false;
    let total = fingerprints.len();

    for (index, fingerprint) in fingerprints.iter().enumerate() {
        if stop.load(Ordering::SeqCst) {
            break;
        }

        mark_item(&job, fingerprint, "running", None);
        set_running_counts(
            &job,
            total.saturating_sub(index),
            1,
            completed,
            passed,
            rejected,
        );

        let holding_elite = snapshot
            .artifact
            .databank
            .holding
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            .cloned();
        let target = holding_elite
            .as_ref()
            .map(|elite| elite.structural_fingerprint.clone());

        let outcome: Result<(bool, Option<String>), (String, String)> = match target {
            None => Err(("other".to_owned(), "not in Holding".to_owned())),
            Some(hash) if audit_and_graduate => {
                let audit = quantforge_discover::audit_holding_battery(
                    &mut snapshot.artifact.databank,
                    &hash,
                    &development,
                    &m1_owned,
                    quote_for_battery.as_ref(),
                    &broker,
                );
                let (audit_passed, audit_reason) = match audit {
                    Ok(result) => (result.passed, result.reason),
                    Err(error) => (false, Some(error.to_string())),
                };
                let selected = BTreeSet::from([hash.to_string()]);
                let graduated = quantforge_discover::promote_selected_holding_without_robustness(
                    &mut snapshot.artifact.databank,
                    &selected,
                    None,
                );
                if graduated.promoted + graduated.replaced != 1 {
                    Err((
                        "other".to_owned(),
                        "could not graduate audited Holding strategy".to_owned(),
                    ))
                } else {
                    Ok((audit_passed, audit_reason))
                }
            }
            Some(hash) => quantforge_discover::run_holding_battery_and_promote(
                &mut snapshot.artifact.databank,
                &hash,
                &development,
                None,
                &m1_owned,
                quote_for_battery.as_ref(),
                &broker,
            )
            .map(|_| (true, None))
            .map_err(|error| (error.kill_bucket().to_string(), error.to_string())),
        };

        completed += 1;
        let mut status = "rejected";
        let mut reason: Option<String> = None;
        match outcome {
            Ok((audit_passed, audit_reason)) => {
                dirty = true;
                if audit_passed {
                    passed += 1;
                    status = "passed";
                    mark_item(&job, fingerprint, "passed", None);
                } else {
                    rejected += 1;
                    status = "audited_failed";
                    reason = audit_reason.clone();
                    mark_item(
                        &job,
                        fingerprint,
                        "audited_failed",
                        audit_reason,
                    );
                }
            }
            Err((bucket, reject_reason)) => {
                rejected += 1;
                bump_kill_mix(&job, &bucket);
                reason = Some(reject_reason.clone());
                mark_item(&job, fingerprint, "rejected", Some(reject_reason));
            }
        }
        if let Some(elite) = holding_elite.as_ref() {
            let robustness = snapshot
                .artifact
                .databank
                .elites
                .iter()
                .find(|row| row.structural_fingerprint.as_str() == fingerprint)
                .and_then(|row| row.robustness.clone());
            let _ = write_battery_csv_row(
                &snapshot.databank_path,
                elite,
                robustness.as_ref(),
                status,
                reason.as_deref(),
            );
        }

        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        let rate = completed as f64 / elapsed * 3600.0;
        let target_hit =
            target_databank.is_some_and(|target| snapshot.artifact.databank.elites.len() >= target);
        let remaining = total.saturating_sub(completed);
        let eta = if completed > 0 && remaining > 0 && !target_hit {
            Some(remaining as f64 / (completed as f64 / elapsed))
        } else {
            Some(0.0)
        };

        let stopped_now = stop.load(Ordering::SeqCst);
        let finished = remaining == 0 || stopped_now || target_hit;
        if !finished {
            if let Some(next) = fingerprints.get(index + 1) {
                mark_item(&job, next, "running", None);
            }
        }
        if let Ok(mut view) = job.write() {
            view.queued = remaining.saturating_sub(if finished { 0 } else { 1 });
            view.running = if finished { 0 } else { 1 };
            view.completed = completed;
            view.passed = passed;
            view.rejected = rejected;
            view.holding_remaining = snapshot.artifact.databank.holding.len();
            view.databank_elites = snapshot.artifact.databank.elites.len();
            view.batteries_per_hour = rate;
            view.eta_seconds = eta;
            view.elapsed_seconds = elapsed;
            view.revision += 1;
            if finished {
                view.status = if stopped_now { "stopped" } else { "completed" };
                view.phase = if stopped_now {
                    "Stopped".into()
                } else if target_hit {
                    "Databank target reached".into()
                } else {
                    "Battery complete".into()
                };
                view.message = funnel_message(
                    &view,
                    passed,
                    rejected,
                    completed,
                    target_hit,
                    audit_and_graduate,
                );
                view.stop_requested = false;
                view.eta_seconds = Some(0.0);
                view.running = 0;
                view.queued = 0;
            } else {
                view.phase = format!(
                    "{} {completed}/{total}",
                    if audit_and_graduate { "Audit" } else { "Battery" }
                );
                view.message = if audit_and_graduate {
                    format!(
                        "{} Holding → {completed} audited → {passed} passed / {rejected} failed tests → all {completed} moved to Databank · {rate:.1}/hr",
                        view.holding_before_shrink
                    )
                } else {
                    format!(
                        "{} Holding → {completed} tested → {passed} moved to Databank / {rejected} remain in Holding · {rate:.1}/hr",
                        view.holding_before_shrink
                    )
                };
            }
        }

        // Rejects remain in Holding. Every successful candidate is saved and
        // published before moving on, so the visible Holding/Databank counts
        // can never lag a completed pass.
        if dirty {
            if !finished {
                let phase = format!("Promoted {passed}; syncing Databank");
                update_phase(
                    &job,
                    &phase,
                    "Saved the accepted strategy; starting the next one.",
                );
            }
            checkpoint_battery_promotion(
                &mut snapshot,
                desktop.as_ref(),
                &job,
                factory_checkpoint.as_ref(),
            )?;
            dirty = false;
        }

        if finished {
            break;
        }
    }

    if dirty {
        let _ = checkpoint_battery_promotion(
            &mut snapshot,
            desktop.as_ref(),
            &job,
            factory_checkpoint.as_ref(),
        );
    }
    if let Ok(mut view) = job.write() {
        if view.status == "running" {
            let stopped = stop.load(Ordering::SeqCst);
            view.status = if stopped { "stopped" } else { "completed" };
            view.running = 0;
            view.queued = 0;
            view.holding_remaining = snapshot.artifact.databank.holding.len();
            view.databank_elites = snapshot.artifact.databank.elites.len();
            view.elapsed_seconds = started.elapsed().as_secs_f64();
            view.eta_seconds = Some(0.0);
            view.revision += 1;
            view.phase = if stopped {
                "Stopped and checkpointed".into()
            } else {
                "Battery complete".into()
            };
            view.message = funnel_message(
                &view,
                view.passed,
                view.rejected,
                view.completed,
                false,
                audit_and_graduate,
            );
            view.stop_requested = false;
        }
    }
    Ok(())
}

fn funnel_message(
    view: &BatteryJobView,
    passed: usize,
    rejected: usize,
    completed: usize,
    target_hit: bool,
    audit_and_graduate: bool,
) -> String {
    if audit_and_graduate {
        return format!(
            "{} Holding → {completed} audited → {passed} passed / {rejected} failed tests → all {completed} graduated to Databank. Battery evidence is recorded, not used as a gate.",
            view.holding_before_shrink,
        );
    }
    let mix = &view.kill_mix;
    let target = if target_hit {
        " Databank target reached."
    } else {
        ""
    };
    format!(
        "{} Holding → {} queued → {completed} tested → {passed} Databank / {rejected} remain in Holding.{target} Kills: neighborhood {} · MC {} · folds {} · M1 {} · deposit {} · corr {} · clone {} · other {}.",
        view.holding_before_shrink,
        view.holding_after_shrink,
        mix.neighborhood,
        mix.monte_carlo,
        mix.folds,
        mix.m1,
        mix.deposit + mix.expectancy + mix.oos1,
        mix.correlation,
        mix.clone,
        mix.other
    )
}

fn battery_log_paths(databank_path: &str) -> (PathBuf, PathBuf) {
    let path = Path::new(databank_path);
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("databank");
    (
        path.with_file_name(format!("{stem}_battery.csv")),
        path.with_file_name(format!("{stem}_param")),
    )
}

fn csv_cell_f64(value: Option<f64>) -> String {
    value
        .filter(|number| number.is_finite())
        .map(|number| format!("{number:.6}"))
        .unwrap_or_default()
}

fn csv_cell_bool(value: Option<bool>) -> String {
    match value {
        Some(true) => "true".into(),
        Some(false) => "false".into(),
        None => String::new(),
    }
}

fn write_battery_csv_row(
    databank_path: &str,
    elite: &Elite,
    robustness: Option<&RobustnessEvidence>,
    status: &str,
    reason: Option<&str>,
) -> Result<(), String> {
    let (summary_path, param_dir) = battery_log_paths(databank_path);
    let neighborhood = robustness.map(|evidence| &evidence.parameter_neighborhood);
    let monte_carlo = robustness.map(|evidence| &evidence.monte_carlo);
    let orig_recovery = neighborhood
        .and_then(|row| row.original_metrics.as_ref())
        .map(|metrics| metrics.recovery_factor())
        .or_else(|| {
            let value = elite.metrics.recovery_factor();
            value.is_finite().then_some(value)
        });
    let new_file = !summary_path.exists();
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&summary_path)
        .map_err(|error| error.to_string())?;
    if new_file {
        writeln!(
            file,
            "status,strategy_id,fingerprint,evidence,trades,reason,orig_recovery_factor,neighborhood_median_recovery,orig_recovery_to_median,passed_retdd_0_85_1_25,neighborhood_samples_requested,neighborhood_samples_evaluated,neighborhood_surviving,neighborhood_survival_fraction,required_survival_fraction,adx_plateau_neighbors,adx_plateau_surviving,adx_plateau_survival_fraction,mc_passed,mc_p80_net_profit,mc_baseline_net_profit,mc_minimum_p80_retention"
        )
        .map_err(|error| error.to_string())?;
    }
    let reason = reason.unwrap_or("").replace(',', ";");
    writeln!(
        file,
        "{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        status,
        elite.strategy.id.replace(',', ";"),
        elite.structural_fingerprint,
        elite.evidence.total,
        elite.metrics.trade_count,
        reason,
        csv_cell_f64(orig_recovery),
        csv_cell_f64(neighborhood.and_then(|row| row.median_recovery_factor)),
        csv_cell_f64(neighborhood.and_then(|row| row.original_recovery_to_median)),
        csv_cell_bool(neighborhood.and_then(|row| row.passed_recovery_median_band)),
        neighborhood
            .map(|row| row.samples_requested.to_string())
            .unwrap_or_default(),
        neighborhood
            .map(|row| row.samples_evaluated.to_string())
            .unwrap_or_default(),
        neighborhood
            .map(|row| row.surviving_samples.to_string())
            .unwrap_or_default(),
        csv_cell_f64(neighborhood.map(|row| row.survival_fraction)),
        csv_cell_f64(neighborhood.map(|row| row.required_survival_fraction)),
        neighborhood
            .map(|row| row.plateau_neighbors.to_string())
            .unwrap_or_default(),
        neighborhood
            .map(|row| row.plateau_surviving.to_string())
            .unwrap_or_default(),
        csv_cell_f64(neighborhood.and_then(|row| row.plateau_survival_fraction)),
        csv_cell_bool(monte_carlo.map(|row| row.passed)),
        csv_cell_f64(monte_carlo.map(|row| row.p80_net_profit)),
        csv_cell_f64(monte_carlo.map(|row| row.baseline_net_profit)),
        csv_cell_f64(monte_carlo.map(|row| row.minimum_p80_profit_retention)),
    )
    .map_err(|error| error.to_string())?;

    if let Some(neighborhood) = neighborhood {
        if !neighborhood.samples.is_empty() {
            std::fs::create_dir_all(&param_dir).map_err(|error| error.to_string())?;
            let sample_path = param_dir.join(format!("{}.csv", elite.strategy.id));
            let mut writer =
                csv::Writer::from_path(&sample_path).map_err(|error| error.to_string())?;
            writer
                .write_record([
                    "sample_index",
                    "survived",
                    "recovery_factor",
                    "net_profit",
                    "return_percent",
                    "max_drawdown_percent",
                    "trade_count",
                    "profit_factor",
                    "sharpe_ratio",
                ])
                .map_err(|error| error.to_string())?;
            for sample in &neighborhood.samples {
                writer
                    .write_record([
                        sample.sample_index.to_string(),
                        sample.survived.to_string(),
                        csv_cell_f64(sample.recovery_factor),
                        format!("{:.6}", sample.net_profit),
                        format!("{:.6}", sample.return_percent),
                        format!("{:.6}", sample.max_drawdown_percent),
                        sample.trade_count.to_string(),
                        csv_cell_f64(sample.profit_factor),
                        csv_cell_f64(sample.sharpe_ratio),
                    ])
                    .map_err(|error| error.to_string())?;
            }
            writer.flush().map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

fn bump_kill_mix(job: &Arc<RwLock<BatteryJobView>>, bucket: &str) {
    if let Ok(mut view) = job.write() {
        match bucket {
            "neighborhood" => view.kill_mix.neighborhood += 1,
            "monte_carlo" => view.kill_mix.monte_carlo += 1,
            "folds" => view.kill_mix.folds += 1,
            "m1" => view.kill_mix.m1 += 1,
            "deposit" => view.kill_mix.deposit += 1,
            "expectancy" => view.kill_mix.expectancy += 1,
            "oos1" => view.kill_mix.oos1 += 1,
            "correlation" => view.kill_mix.correlation += 1,
            "clone" => view.kill_mix.clone += 1,
            _ => view.kill_mix.other += 1,
        }
    }
}

fn update_phase(job: &Arc<RwLock<BatteryJobView>>, phase: &str, message: &str) {
    if let Ok(mut view) = job.write() {
        view.phase = phase.into();
        view.message = message.into();
    }
}

fn mark_item(
    job: &Arc<RwLock<BatteryJobView>>,
    fingerprint: &str,
    status: &'static str,
    reason: Option<String>,
) {
    if let Ok(mut view) = job.write() {
        if let Some(item) = view
            .items
            .iter_mut()
            .find(|item| item.fingerprint == fingerprint)
        {
            item.status = status;
            item.reason = reason;
        }
    }
}

fn set_running_counts(
    job: &Arc<RwLock<BatteryJobView>>,
    queued: usize,
    running: usize,
    completed: usize,
    passed: usize,
    rejected: usize,
) {
    if let Ok(mut view) = job.write() {
        view.queued = queued.saturating_sub(running);
        view.running = running;
        view.completed = completed;
        view.passed = passed;
        view.rejected = rejected;
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingCorrShrinkRequest {
    pub max_correlation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldingCorrShrinkView {
    kept: usize,
    dropped: usize,
    max_correlation: f64,
    replayed: usize,
    workspace: crate::databank::DatabankWorkspace,
}

#[tauri::command]
pub async fn shrink_holding_by_daily_corr(
    request: HoldingCorrShrinkRequest,
    desktop: State<'_, DesktopState>,
    state: State<'_, BatteryJobState>,
) -> Result<HoldingCorrShrinkView, String> {
    let allowed = [0.3, 0.4, 0.5, 0.6];
    if !allowed
        .iter()
        .any(|value| (request.max_correlation - value).abs() < 1e-9)
    {
        return Err("pick a daily P/L correlation cap of 0.3, 0.4, 0.5 or 0.6".into());
    }
    {
        let current = state
            .job
            .read()
            .map_err(|_| "battery job state is unavailable")?;
        let stale_running = current.status == "running"
            && current.total > 0
            && current.completed >= current.total
            && current.running == 0;
        if current.status == "running" && !stale_running {
            return Err("stop or finish the Holding battery before shrinking Holding".into());
        }
    }

    let snapshot = {
        let loaded = desktop
            .loaded
            .read()
            .map_err(|_| "desktop databank state is unavailable".to_owned())?;
        let loaded = loaded.as_ref().ok_or_else(|| {
            "no databank is loaded — open the Discover checkpoint first".to_owned()
        })?;
        if loaded.legacy_read_only {
            return Err(
                "Schema-v5 databanks are read-only. Run a fresh Discover archive before shrinking Holding."
                    .into(),
            );
        }
        if loaded.bank.holding.is_empty() {
            return Err("Holding is empty".into());
        }
        (
            loaded.bank.clone(),
            loaded.databank_path.clone(),
            loaded.source.clone(),
            loaded.broker.clone(),
            loaded.metadata_path.clone(),
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
        validation_fraction,
        sealed_fraction,
    ) = snapshot;
    let max_correlation = request.max_correlation;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let loaded = crate::data_lab::load_data_source(&source, metadata_path.as_deref(), None)?;
        let mut dataset = loaded.dataset;
        apply_history_start_year(&mut dataset, bank.config.history_start_year)?;
        let broker = load_bound_broker(&broker_path, loaded.metadata.as_ref())?;
        let plan = DataSplitPlan::chronological(&dataset, validation_fraction, sealed_fraction)
            .map_err(|error| error.to_string())?;
        let development = slice_bars(&dataset, 0, plan.development.bar_count)?;
        let cache = IndicatorBufferCache::new(development.bars.len());
        let scout = bank.config.scout.clone();
        let timezone = broker.timezone.clone();
        let daily_pnl: Vec<_> = bank
            .holding
            .par_iter()
            .map(|elite| {
                evaluate_strategy_cached(&elite.strategy, &development, &broker, &scout, &cache)
                    .map(|result| daily_pnl_from_trades(&result.trades, &timezone))
                    .unwrap_or_default()
            })
            .collect();
        let replayed = daily_pnl.iter().filter(|days| !days.is_empty()).count();
        let report = apply_holding_daily_corr_shrink(&mut bank, &daily_pnl, max_correlation);
        persist_bank_file(&databank_path, &mut bank)?;
        Ok::<_, String>((report, replayed, databank_path, bank))
    })
    .await
    .map_err(|error| error.to_string())??;
    let (report, replayed, databank_path, bank) = result;
    persist_loaded_bank(&databank_path, &bank, &desktop)?;
    let workspace = reload_workspace_from_path(Path::new(&databank_path), &desktop)?;
    Ok(HoldingCorrShrinkView {
        kept: report.kept,
        dropped: report.dropped,
        max_correlation: report.max_correlation,
        replayed,
        workspace,
    })
}
