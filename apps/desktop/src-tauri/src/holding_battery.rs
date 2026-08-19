//! Discover-style job runner for on-demand Holding → Databank battery.

use crate::data_lab::{
    apply_history_start_year, load_bound_broker, load_quote_sidecar, trim_market_history_to_year,
};
use crate::databank::{
    DesktopState, HoldingBatteryRequest, infer_quote_sidecar_path, persist_bank_file,
    persist_loaded_bank, reload_workspace_from_path, slice_bars,
};
use quantforge_data::BarDataset;
use quantforge_discover::{
    Databank, apply_holding_daily_corr_shrink, daily_pnl_from_trades, holding_factory_score,
};
use quantforge_eval::{IndicatorBufferCache, evaluate_strategy_cached};
use quantforge_quality::DataSplitPlan;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

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
    other: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatteryJobView {
    job_id: Option<String>,
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
}

impl BatteryJobView {
    fn idle() -> Self {
        Self {
            job_id: None,
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
pub fn get_holding_battery_job(state: State<'_, BatteryJobState>) -> Result<BatteryJobView, String> {
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
        status: "running",
        phase: if request.shrink_first || request.ranked {
            "Preparing factory".into()
        } else {
            "Loading market data".into()
        },
        message: if request.ranked {
            "Ranking Holding after shrink, then running the robustness battery on the remaining names.".into()
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
        holding_remaining: snapshot.bank.holding.len(),
        databank_elites: snapshot.bank.elites.len(),
        batteries_per_hour: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0.0,
        stop_requested: false,
        revision: 0,
        items: Vec::new(),
        kill_mix: BatteryKillMix::default(),
        holding_before_shrink: snapshot.bank.holding.len(),
        holding_after_shrink: snapshot.bank.holding.len(),
        target_databank: request.target_databank,
    };
    *state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable")? = started.clone();

    let job = Arc::clone(&state.job);
    let stop = Arc::clone(&state.stop);
    let job_for_err = Arc::clone(&job);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_factory_or_battery(job, stop, snapshot, request) {
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

/// Overnight Discover → shrink → ranked battery. Safe to call after a checkpoint write.
pub(crate) fn spawn_factory_from_archive(
    databank_path: String,
    request: HoldingBatteryRequest,
    state: &BatteryJobState,
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
        status: "running",
        phase: "Preparing factory".into(),
        message: "Discover finished — shrinking Holding and ranking the battery queue.".into(),
        databank_path: Some(databank_path),
        total: 0,
        queued: 0,
        running: 0,
        completed: 0,
        passed: 0,
        rejected: 0,
        holding_remaining: snapshot.bank.holding.len(),
        databank_elites: snapshot.bank.elites.len(),
        batteries_per_hour: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0.0,
        stop_requested: false,
        revision: 0,
        items: Vec::new(),
        kill_mix: BatteryKillMix::default(),
        holding_before_shrink: snapshot.bank.holding.len(),
        holding_after_shrink: snapshot.bank.holding.len(),
        target_databank: request.target_databank,
    };
    *state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable")? = started.clone();
    let job = Arc::clone(&state.job);
    let stop = Arc::clone(&state.stop);
    let job_for_err = Arc::clone(&job);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_factory_or_battery(job, stop, snapshot, request) {
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
    bank: Databank,
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

fn snapshot_loaded_archive(desktop: &DesktopState) -> Result<ArchiveSnapshot, String> {
    let loaded = desktop
        .loaded
        .read()
        .map_err(|_| "desktop databank state is unavailable".to_owned())?;
    let loaded = loaded
        .as_ref()
        .ok_or_else(|| "no databank is loaded — open the Discover checkpoint first".to_owned())?;
    Ok(ArchiveSnapshot {
        bank: loaded.bank.clone(),
        databank_path: loaded.databank_path.clone(),
        source: loaded.source.clone(),
        broker_path: loaded.broker.clone(),
        metadata_path: loaded.metadata_path.clone(),
        m1_source: loaded
            .m1_source
            .clone()
            .ok_or_else(|| {
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
    let m1_source = crate::databank::manifest_path(&artifact, "m1_source")
        .ok_or_else(|| "This archive does not bind an M1 source; cannot run the factory.".to_owned())?;
    let validation_fraction =
        crate::databank::manifest_fraction(&artifact, "validation_fraction", 0.0);
    let sealed_fraction =
        crate::databank::manifest_fraction(&artifact, "sealed_fraction", 1.0 / 3.0);
    let metadata_path = Path::new(&source)
        .with_extension("metadata.csv")
        .is_file()
        .then(|| Path::new(&source).with_extension("metadata.csv").display().to_string());
    let m1_metadata_path = Path::new(&m1_source)
        .with_extension("metadata.csv")
        .is_file()
        .then(|| Path::new(&m1_source).with_extension("metadata.csv").display().to_string());
    Ok(ArchiveSnapshot {
        bank: artifact.databank,
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

fn run_factory_or_battery(
    job: Arc<RwLock<BatteryJobView>>,
    stop: Arc<AtomicBool>,
    mut snapshot: ArchiveSnapshot,
    request: HoldingBatteryRequest,
) -> Result<(), String> {
    let holding_before = snapshot.bank.holding.len();
    if request.shrink_first {
        update_phase(
            &job,
            "Shrinking Holding",
            "Dropping daily P/L clones before the battery.",
        );
        shrink_holding_snapshot(&mut snapshot, request.max_correlation.unwrap_or(DEFAULT_FACTORY_CORR))?;
        persist_bank_file(&snapshot.databank_path, &snapshot.bank)?;
    }
    let holding_after = snapshot.bank.holding.len();
    let queue_take = optional_positive(request.queue_limit);
    let target_databank = optional_positive(request.target_databank);
    let fingerprints = if request.ranked || request.fingerprints.is_empty() {
        ranked_holding_fingerprints(&snapshot.bank, queue_take)
    } else {
        request.fingerprints.clone()
    };
    if fingerprints.is_empty() {
        return Err("Holding is empty after shrink — nothing to battery".into());
    }
    let items: Vec<_> = fingerprints
        .iter()
        .filter_map(|fingerprint| {
            snapshot.bank.holding.iter().find(|elite| {
                elite.structural_fingerprint.as_str() == fingerprint
            }).map(|elite| BatteryItemView {
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
        view.holding_remaining = snapshot.bank.holding.len();
        view.databank_elites = snapshot.bank.elites.len();
        view.target_databank = target_databank;
        view.revision += 1;
        view.phase = "Loading market data".into();
        let stop_note = match target_databank {
            Some(n) => format!("Battery until {n} Databank names."),
            None => "Battery everyone queued; keep every passer.".to_owned(),
        };
        view.message = format!(
            "Funnel {} Holding → {} after shrink → {} queued. {stop_note}",
            holding_before,
            holding_after,
            fingerprints.len(),
        );
    }

    run_battery_job(
        job,
        stop,
        snapshot.bank,
        snapshot.databank_path,
        snapshot.source,
        snapshot.metadata_path,
        snapshot.m1_source,
        snapshot.m1_metadata_path,
        snapshot.broker_path,
        snapshot.validation_fraction,
        snapshot.sealed_fraction,
        fingerprints,
        target_databank,
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
    apply_history_start_year(&mut dataset, snapshot.bank.config.history_start_year)?;
    let broker = load_bound_broker(&snapshot.broker_path, loaded.metadata.as_ref())?;
    let plan = DataSplitPlan::chronological(
        &dataset,
        snapshot.validation_fraction,
        snapshot.sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let development = slice_bars(&dataset, 0, plan.development.bar_count)?;
    let cache = IndicatorBufferCache::new(development.bars.len());
    let scout = snapshot.bank.config.scout.clone();
    let timezone = broker.timezone.clone();
    let daily_pnl: Vec<_> = snapshot
        .bank
        .holding
        .par_iter()
        .map(|elite| {
            evaluate_strategy_cached(&elite.strategy, &development, &broker, &scout, &cache)
                .map(|result| daily_pnl_from_trades(&result.trades, &timezone))
                .unwrap_or_default()
        })
        .collect();
    apply_holding_daily_corr_shrink(&mut snapshot.bank, &daily_pnl, max_correlation);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_battery_job(
    job: Arc<RwLock<BatteryJobView>>,
    stop: Arc<AtomicBool>,
    mut bank: Databank,
    databank_path: String,
    source: String,
    metadata_path: Option<String>,
    m1_source: String,
    m1_metadata_path: Option<String>,
    broker_path: String,
    validation_fraction: f64,
    sealed_fraction: f64,
    fingerprints: Vec<String>,
    target_databank: Option<usize>,
) -> Result<(), String> {
    let started = Instant::now();
    update_phase(
        &job,
        "Loading market data",
        "Loading Development / M1 partitions for the battery…",
    );

    let mut decision = crate::data_lab::load_data_source(&source, metadata_path.as_deref(), None)?;
    let mut m1 = crate::data_lab::load_data_source(&m1_source, m1_metadata_path.as_deref(), None)?;
    let broker = load_bound_broker(&broker_path, decision.metadata.as_ref())?;
    load_bound_broker(&broker_path, m1.metadata.as_ref())?;
    let mut quote_dataset = infer_quote_sidecar_path(&m1_source)
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
    let plan = DataSplitPlan::chronological(&decision.dataset, validation_fraction, sealed_fraction)
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
    let m1_owned: BarDataset = if bank.execution_data_hash == m1_development.data_hash {
        m1_development
    } else if bank.execution_data_hash == m1.dataset.data_hash {
        m1.dataset.clone()
    } else {
        m1_development
    };

    update_phase(
        &job,
        "Running battery",
        "Only full passes move to Databank; failures stay in Holding.",
    );

    let mut passed = 0usize;
    let mut rejected = 0usize;
    let mut completed = 0usize;
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

        let target = bank
            .holding
            .iter()
            .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            .map(|elite| elite.structural_fingerprint.clone());

        let outcome = match target {
            None => Err(("other".to_owned(), "not in Holding".to_owned())),
            Some(hash) => quantforge_discover::run_holding_battery_and_promote(
                &mut bank,
                &hash,
                &development,
                oos1.as_ref(),
                &m1_owned,
                quote_dataset.as_ref(),
                &broker,
            )
            .map(|_| ())
            .map_err(|error| (error.kill_bucket().to_string(), error.to_string())),
        };

        completed += 1;
        let mut bank_changed = false;
        match outcome {
            Ok(()) => {
                passed += 1;
                bank_changed = true;
                mark_item(&job, fingerprint, "passed", None);
            }
            Err((bucket, reason)) => {
                rejected += 1;
                bump_kill_mix(&job, &bucket);
                mark_item(&job, fingerprint, "rejected", Some(reason));
            }
        }

        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        let rate = completed as f64 / elapsed * 3600.0;
        let target_hit = target_databank.is_some_and(|target| bank.elites.len() >= target);
        let remaining = total.saturating_sub(completed);
        let eta = if completed > 0 && remaining > 0 && !target_hit {
            Some(remaining as f64 / (completed as f64 / elapsed))
        } else {
            Some(0.0)
        };

        // Publish counters immediately — do not wait on the multi-MB archive write.
        // When the queue is empty, flip to a terminal status in the same write so
        // the UI can re-enable Run battery without waiting on checkpoint I/O.
        let stopped_now = stop.load(Ordering::SeqCst);
        let finished = remaining == 0 || stopped_now || target_hit;
        if let Ok(mut view) = job.write() {
            view.queued = remaining;
            view.running = 0;
            view.completed = completed;
            view.passed = passed;
            view.rejected = rejected;
            view.holding_remaining = bank.holding.len();
            view.databank_elites = bank.elites.len();
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
                view.message = funnel_message(&view, passed, rejected, completed, target_hit);
                view.stop_requested = false;
                view.eta_seconds = Some(0.0);
            } else {
                view.phase = format!("Battery {completed}/{total}");
                view.message = format!(
                    "{} → {} after shrink → {completed} battered → {passed} pass / {rejected} fail · {rate:.1}/hr",
                    view.holding_before_shrink,
                    view.holding_after_shrink
                );
            }
        }

        // Rejects leave Holding unchanged — skip the expensive full-archive rewrite.
        if bank_changed {
            persist_bank_file(&databank_path, &bank)?;
            if let Ok(mut view) = job.write() {
                view.revision += 1;
            }
        }

        if finished {
            break;
        }

        std::thread::sleep(Duration::from_millis(5));
    }

    // Extra checkpoint only if a pass landed and we may have broken before writing.
    if passed > 0 {
        let _ = persist_bank_file(&databank_path, &bank);
    }
    // Ensure terminal status even if the loop exited via stop before the
    // per-item finish write (e.g. stop before first strategy).
    if let Ok(mut view) = job.write() {
        if view.status == "running" {
            let stopped = stop.load(Ordering::SeqCst);
            view.status = if stopped { "stopped" } else { "completed" };
            view.running = 0;
            view.queued = 0;
            view.holding_remaining = bank.holding.len();
            view.databank_elites = bank.elites.len();
            view.elapsed_seconds = started.elapsed().as_secs_f64();
            view.eta_seconds = Some(0.0);
            view.revision += 1;
            view.phase = if stopped {
                "Stopped and checkpointed".into()
            } else {
                "Battery complete".into()
            };
            view.message = funnel_message(&view, view.passed, view.rejected, view.completed, false);
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
) -> String {
    let mix = &view.kill_mix;
    let target = if target_hit {
        " Databank target reached."
    } else {
        ""
    };
    format!(
        "{} Holding → {} after shrink → {completed} battered → {passed} Databank / {rejected} fail.{target} Kills: neighborhood {} · MC {} · folds {} · M1 {} · deposit {} · other {}.",
        view.holding_before_shrink,
        view.holding_after_shrink,
        mix.neighborhood,
        mix.monte_carlo,
        mix.folds,
        mix.m1,
        mix.deposit + mix.expectancy + mix.oos1,
        mix.other
    )
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
            return Err(
                "stop or finish the Holding battery before shrinking Holding".into(),
            );
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
        let plan = DataSplitPlan::chronological(
            &dataset,
            validation_fraction,
            sealed_fraction,
        )
        .map_err(|error| error.to_string())?;
        let development = slice_bars(&dataset, 0, plan.development.bar_count)?;
        let cache = IndicatorBufferCache::new(development.bars.len());
        let scout = bank.config.scout.clone();
        let timezone = broker.timezone.clone();
        let daily_pnl: Vec<_> = bank
            .holding
            .par_iter()
            .map(|elite| {
                evaluate_strategy_cached(
                    &elite.strategy,
                    &development,
                    &broker,
                    &scout,
                    &cache,
                )
                .map(|result| daily_pnl_from_trades(&result.trades, &timezone))
                .unwrap_or_default()
            })
            .collect();
        let replayed = daily_pnl
            .iter()
            .filter(|days| !days.is_empty())
            .count();
        let report = apply_holding_daily_corr_shrink(&mut bank, &daily_pnl, max_correlation);
        persist_bank_file(&databank_path, &bank)?;
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
