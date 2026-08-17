//! Discover-style job runner for on-demand Holding → Databank battery.

use crate::data_lab::{load_bound_broker, load_quote_sidecar};
use crate::databank::{
    DesktopState, HoldingBatteryRequest, infer_quote_sidecar_path, persist_bank_file, slice_bars,
};
use quantforge_data::BarDataset;
use quantforge_discover::Databank;
use quantforge_quality::DataSplitPlan;
use serde::Serialize;
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
        }
    }
}

pub struct BatteryJobState {
    job: Arc<RwLock<BatteryJobView>>,
    stop: Arc<AtomicBool>,
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

#[tauri::command]
pub fn start_holding_battery_job(
    request: HoldingBatteryRequest,
    desktop: State<'_, DesktopState>,
    state: State<'_, BatteryJobState>,
) -> Result<BatteryJobView, String> {
    if request.fingerprints.is_empty() {
        return Err("select at least one Holding strategy".into());
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
            return Err("a Holding battery job is already running".into());
        }
    }

    let snapshot = {
        let loaded = desktop
            .loaded
            .read()
            .map_err(|_| "desktop databank state is unavailable".to_owned())?;
        let loaded = loaded
            .as_ref()
            .ok_or_else(|| "no databank is loaded — open the Discover checkpoint first".to_owned())?;
        if loaded.legacy_read_only {
            return Err(
                "Schema-v5 databanks are read-only. Run a fresh Discover archive before Holding battery."
                    .into(),
            );
        }
        let mut fingerprints = Vec::new();
        let mut items = Vec::new();
        for fingerprint in &request.fingerprints {
            let Some(elite) = loaded
                .bank
                .holding
                .iter()
                .find(|elite| elite.structural_fingerprint.as_str() == fingerprint)
            else {
                return Err(format!("{fingerprint} is not in Holding"));
            };
            fingerprints.push(fingerprint.clone());
            items.push(BatteryItemView {
                fingerprint: fingerprint.clone(),
                strategy_id: elite.strategy.id.clone(),
                status: "queued",
                reason: None,
                evidence: elite.evidence.total,
                trades: elite.metrics.trade_count,
            });
        }
        (
            loaded.bank.clone(),
            loaded.databank_path.clone(),
            loaded.source.clone(),
            loaded.broker.clone(),
            loaded.metadata_path.clone(),
            loaded
                .m1_source
                .clone()
                .ok_or_else(|| {
                    "This archive does not bind an M1 source; cannot run Holding battery.".to_owned()
                })?,
            loaded.m1_metadata_path.clone(),
            loaded.validation_fraction,
            loaded.sealed_fraction,
            fingerprints,
            items,
        )
    };

    let (
        bank,
        databank_path,
        source,
        broker_path,
        metadata_path,
        m1_source,
        m1_metadata_path,
        validation_fraction,
        sealed_fraction,
        fingerprints,
        items,
    ) = snapshot;

    state.stop.store(false, Ordering::SeqCst);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis() as u64;
    let started = BatteryJobView {
        job_id: Some(format!("battery-{now_ms}")),
        status: "running",
        phase: "Loading market data".into(),
        message: format!(
            "Queued {} Holding strategies for the robustness battery.",
            items.len()
        ),
        databank_path: Some(databank_path.clone()),
        total: items.len(),
        queued: items.len(),
        running: 0,
        completed: 0,
        passed: 0,
        rejected: 0,
        holding_remaining: bank.holding.len(),
        databank_elites: bank.elites.len(),
        batteries_per_hour: 0.0,
        eta_seconds: None,
        elapsed_seconds: 0.0,
        stop_requested: false,
        revision: 0,
        items,
    };
    *state
        .job
        .write()
        .map_err(|_| "battery job state is unavailable")? = started.clone();

    let job = Arc::clone(&state.job);
    let stop = Arc::clone(&state.stop);
    let job_for_err = Arc::clone(&job);

    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_battery_job(
            job,
            stop,
            bank,
            databank_path,
            source,
            metadata_path,
            m1_source,
            m1_metadata_path,
            broker_path,
            validation_fraction,
            sealed_fraction,
            fingerprints,
        ) {
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
) -> Result<(), String> {
    let started = Instant::now();
    update_phase(
        &job,
        "Loading market data",
        "Loading Development / M1 partitions for the battery…",
    );

    let decision = crate::data_lab::load_data_source(&source, metadata_path.as_deref(), None)?;
    let m1 = crate::data_lab::load_data_source(&m1_source, m1_metadata_path.as_deref(), None)?;
    let broker = load_bound_broker(&broker_path, decision.metadata.as_ref())?;
    load_bound_broker(&broker_path, m1.metadata.as_ref())?;
    let quote_dataset = infer_quote_sidecar_path(&m1_source)
        .filter(|path| path.is_file())
        .map(|path| load_quote_sidecar(&path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
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
            None => Err("not in Holding".to_owned()),
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
            .map_err(|error| error.to_string()),
        };

        completed += 1;
        let mut bank_changed = false;
        match outcome {
            Ok(()) => {
                passed += 1;
                bank_changed = true;
                mark_item(&job, fingerprint, "passed", None);
            }
            Err(reason) => {
                rejected += 1;
                mark_item(&job, fingerprint, "rejected", Some(reason));
            }
        }

        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        let rate = completed as f64 / elapsed * 3600.0;
        let remaining = total.saturating_sub(completed);
        let eta = if completed > 0 && remaining > 0 {
            Some(remaining as f64 / (completed as f64 / elapsed))
        } else {
            Some(0.0)
        };

        // Publish counters immediately — do not wait on the multi-MB archive write.
        // When the queue is empty, flip to a terminal status in the same write so
        // the UI can re-enable Run battery without waiting on checkpoint I/O.
        let stopped_now = stop.load(Ordering::SeqCst);
        let finished = remaining == 0 || stopped_now;
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
                } else {
                    "Battery complete".into()
                };
                view.message = format!(
                    "{passed} passed into Databank, {rejected} rejected (still in Holding) after {completed} strategies."
                );
                view.stop_requested = false;
                view.eta_seconds = Some(0.0);
            } else {
                view.phase = format!("Battery {completed}/{total}");
                view.message = format!(
                    "{passed} passed → Databank · {rejected} rejected (still Holding) · {rate:.1}/hr"
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
            view.message = format!(
                "{} passed into Databank, {} rejected (still in Holding) after {} strategies.",
                view.passed, view.rejected, view.completed
            );
            view.stop_requested = false;
        }
    }
    Ok(())
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
