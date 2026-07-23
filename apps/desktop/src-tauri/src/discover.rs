use crate::data_lab::{display_path, load_bound_broker, load_data_source};
use crate::databank::{EvolveArtifact, verify_artifact};
use quantforge_discover::{Databank, DiscoverConfig, GateConfig, continue_evolution, evolve_new};
use quantforge_eval::{CostModel, SameBarPolicy, ScoutConfig};
use quantforge_storage::{RunManifest, RunRecipe, write_json_new, write_json_versioned};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverMode {
    New,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    mode: DiscoverMode,
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    broker_path: String,
    databank_path: String,
    generations: u64,
    initial_candidates: Option<usize>,
    batch_size: Option<usize>,
    correlation_threshold: Option<f64>,
    novelty_weight: Option<f64>,
    seed: Option<u64>,
    minimum_trades: Option<usize>,
    maximum_drawdown_percent: Option<f64>,
    minimum_return_percent: Option<f64>,
    minimum_profit_factor: Option<f64>,
    minimum_m1_return_retention: Option<f64>,
    commission_per_lot_round_turn: Option<f64>,
    slippage_points_per_side: Option<f64>,
    fallback_spread_points: Option<f64>,
    max_spread_points: Option<f64>,
    initial_balance: Option<f64>,
    promotion_split: Option<bool>,
    validation_fraction: Option<f64>,
    sealed_fraction: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverJobView {
    job_id: Option<String>,
    status: &'static str,
    mode: Option<DiscoverModeView>,
    phase: String,
    output_path: Option<String>,
    completed_generations: u64,
    requested_generations: u64,
    evaluation_count: u64,
    coverage: usize,
    qd_score: f64,
    rejected_clone: u64,
    rejected_correlated: u64,
    rejected_total: u64,
    stop_requested: bool,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverModeView {
    New,
    Continue,
}

impl From<DiscoverMode> for DiscoverModeView {
    fn from(value: DiscoverMode) -> Self {
        match value {
            DiscoverMode::New => Self::New,
            DiscoverMode::Continue => Self::Continue,
        }
    }
}

pub struct DiscoverState {
    job: Arc<RwLock<DiscoverJobView>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

impl Default for DiscoverState {
    fn default() -> Self {
        Self {
            job: Arc::new(RwLock::new(DiscoverJobView::idle())),
            paused: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl DiscoverJobView {
    fn idle() -> Self {
        Self {
            job_id: None,
            status: "idle",
            mode: None,
            phase: "Ready".into(),
            output_path: None,
            completed_generations: 0,
            requested_generations: 0,
            evaluation_count: 0,
            coverage: 0,
            qd_score: 0.0,
            rejected_clone: 0,
            rejected_correlated: 0,
            rejected_total: 0,
            stop_requested: false,
            message: "Configure a new search or continue an existing databank.".into(),
        }
    }
}

#[tauri::command]
pub fn start_discover(
    request: DiscoverRequest,
    state: State<'_, DiscoverState>,
) -> Result<DiscoverJobView, String> {
    validate_request(&request)?;
    {
        let current = state
            .job
            .read()
            .map_err(|_| "discover job state is unavailable")?;
        if matches!(current.status, "running" | "paused") {
            return Err("a discovery job is already active".into());
        }
    }

    state.paused.store(false, Ordering::SeqCst);
    state.stop.store(false, Ordering::SeqCst);
    let job_id = format!(
        "desktop-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis()
    );
    let started = DiscoverJobView {
        job_id: Some(job_id),
        status: "running",
        mode: Some(request.mode.into()),
        phase: "Loading and validating inputs".into(),
        output_path: Some(display_path(Path::new(&request.databank_path))),
        completed_generations: 0,
        requested_generations: request.generations,
        evaluation_count: 0,
        coverage: 0,
        qd_score: 0.0,
        rejected_clone: 0,
        rejected_correlated: 0,
        rejected_total: 0,
        stop_requested: false,
        message: "The Rust discovery worker is starting.".into(),
    };
    *state
        .job
        .write()
        .map_err(|_| "discover job state is unavailable")? = started.clone();

    let job = Arc::clone(&state.job);
    let paused = Arc::clone(&state.paused);
    let stop = Arc::clone(&state.stop);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(error) = run_discovery(request, &job, &paused, &stop) {
            if let Ok(mut view) = job.write() {
                view.status = "failed";
                view.phase = "Stopped with an error".into();
                view.message = error;
            }
        }
    });
    Ok(started)
}

#[tauri::command]
pub fn get_discover_job(state: State<'_, DiscoverState>) -> Result<DiscoverJobView, String> {
    state
        .job
        .read()
        .map(|value| value.clone())
        .map_err(|_| "discover job state is unavailable".into())
}

#[tauri::command]
pub fn pause_discover(state: State<'_, DiscoverState>) -> Result<DiscoverJobView, String> {
    let mut view = state
        .job
        .write()
        .map_err(|_| "discover job state is unavailable")?;
    if view.status != "running" {
        return Err("only a running discovery job can be paused".into());
    }
    state.paused.store(true, Ordering::SeqCst);
    view.status = "paused";
    view.phase = "Paused between generations".into();
    view.message = "Resume to continue the same in-memory deterministic job.".into();
    Ok(view.clone())
}

#[tauri::command]
pub fn resume_discover(state: State<'_, DiscoverState>) -> Result<DiscoverJobView, String> {
    let mut view = state
        .job
        .write()
        .map_err(|_| "discover job state is unavailable")?;
    if view.status != "paused" {
        return Err("only a paused discovery job can be resumed".into());
    }
    state.paused.store(false, Ordering::SeqCst);
    view.status = "running";
    view.phase = "Resuming evolution".into();
    view.message = "The next generation will start immediately.".into();
    Ok(view.clone())
}

#[tauri::command]
pub fn stop_discover(state: State<'_, DiscoverState>) -> Result<DiscoverJobView, String> {
    let mut view = state
        .job
        .write()
        .map_err(|_| "discover job state is unavailable")?;
    if !matches!(view.status, "running" | "paused") {
        return Err("no active discovery job can be stopped".into());
    }
    state.stop.store(true, Ordering::SeqCst);
    state.paused.store(false, Ordering::SeqCst);
    view.stop_requested = true;
    view.status = "running";
    view.phase = "Stopping after the current generation".into();
    view.message = "The current archive will be checkpointed before the job exits.".into();
    Ok(view.clone())
}

fn validate_request(request: &DiscoverRequest) -> Result<(), String> {
    if request.data_path.trim().is_empty()
        || request.m1_data_path.trim().is_empty()
        || request.broker_path.trim().is_empty()
        || request.databank_path.trim().is_empty()
    {
        return Err("data, broker and databank paths are required".into());
    }
    if request.generations == 0 {
        return Err("at least one generation is required".into());
    }
    let databank_exists = Path::new(&request.databank_path).exists();
    match request.mode {
        DiscoverMode::New if databank_exists => {
            return Err("new discovery refuses to replace an existing databank".into());
        }
        DiscoverMode::Continue if !databank_exists => {
            return Err("continuation requires an existing databank".into());
        }
        _ => {}
    }
    if request.mode == DiscoverMode::Continue
        && [
            request.initial_candidates.is_some(),
            request.batch_size.is_some(),
            request.correlation_threshold.is_some(),
            request.novelty_weight.is_some(),
            request.seed.is_some(),
            request.minimum_trades.is_some(),
            request.maximum_drawdown_percent.is_some(),
            request.minimum_return_percent.is_some(),
            request.minimum_profit_factor.is_some(),
            request.minimum_m1_return_retention.is_some(),
            request.commission_per_lot_round_turn.is_some(),
            request.slippage_points_per_side.is_some(),
            request.fallback_spread_points.is_some(),
            request.max_spread_points.is_some(),
            request.initial_balance.is_some(),
            request.promotion_split.is_some(),
            request.validation_fraction.is_some(),
            request.sealed_fraction.is_some(),
        ]
        .into_iter()
        .any(|configured| configured)
    {
        return Err(
            "continuation uses the immutable configuration stored in the databank; clear all new-search overrides"
                .into(),
        );
    }
    Ok(())
}

fn run_discovery(
    request: DiscoverRequest,
    job: &Arc<RwLock<DiscoverJobView>>,
    paused: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let m1 = load_data_source(
        &request.m1_data_path,
        request.m1_metadata_path.as_deref(),
        request.m1_source_timezone.as_deref(),
    )?;
    let quality = quantforge_data::DataQualityReport::analyze(&loaded.dataset);
    if quality.grade == quantforge_data::QualityGrade::Fail {
        return Err(format!(
            "Discover refuses failed-quality data (score {})",
            quality.score
        ));
    }
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    load_bound_broker(&request.broker_path, m1.metadata.as_ref())?;
    let m1_quality = quantforge_data::DataQualityReport::analyze(&m1.dataset);
    if m1_quality.grade == quantforge_data::QualityGrade::Fail {
        return Err(format!(
            "Discover refuses failed-quality M1 data (score {})",
            m1_quality.score
        ));
    }
    let promotion_split = request.promotion_split.unwrap_or(false);
    let validation_fraction = request.validation_fraction.unwrap_or(0.2);
    let sealed_fraction = request.sealed_fraction.unwrap_or(0.2);
    let development_dataset = (promotion_split || request.mode == DiscoverMode::Continue)
        .then(|| development_partition(&loaded.dataset, validation_fraction, sealed_fraction))
        .transpose()?;
    let new_dataset = development_dataset.as_ref().unwrap_or(&loaded.dataset);

    let (mut bank, continuation_recipe_hash, starting_generation) = match request.mode {
        DiscoverMode::New => {
            update_phase(
                job,
                "Evaluating initial grammar population",
                "The four seed families are being evaluated in parallel.",
            )?;
            let bank = evolve_new(new_dataset, &m1.dataset, &broker, new_config(&request)?, 0)
                .map_err(|error| error.to_string())?;
            update_bank(job, &bank, 0, request.generations)?;
            (bank, None, 0)
        }
        DiscoverMode::Continue => {
            let bytes = fs::read(&request.databank_path)
                .map_err(|error| format!("cannot read databank: {error}"))?;
            let artifact: EvolveArtifact = serde_json::from_slice(&bytes)
                .map_err(|error| format!("databank JSON is invalid: {error}"))?;
            verify_artifact(&artifact).map_err(|error| error.to_string())?;
            let starting_generation = artifact.databank.completed_generations;
            update_bank(job, &artifact.databank, 0, request.generations)?;
            (
                artifact.databank,
                Some(artifact.manifest.recipe_hash),
                starting_generation,
            )
        }
    };

    let mut completed_now = 0;
    while completed_now < request.generations {
        wait_if_paused(job, paused, stop)?;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        update_phase(
            job,
            &format!(
                "Evolving generation {} of {}",
                completed_now + 1,
                request.generations
            ),
            "Candidates are bred, evaluated in parallel, then deposited in deterministic order.",
        )?;
        let evaluation_dataset = if bank.data_hash == loaded.dataset.data_hash {
            &loaded.dataset
        } else {
            development_dataset.as_ref().ok_or_else(|| {
                "this databank was built from a development partition; enable the identical promotion split to continue it".to_owned()
            })?
        };
        bank = continue_evolution(bank, evaluation_dataset, &m1.dataset, &broker, 1)
            .map_err(|error| error.to_string())?;
        completed_now += 1;
        update_bank(job, &bank, completed_now, request.generations)?;
    }

    bank.validate_integrity().map_err(|error| {
        format!(
            "discovery produced no loadable checkpoint: {error}. Use more history or revise the explicit discovery gates"
        )
    })?;

    update_phase(
        job,
        "Writing immutable checkpoint",
        "The manifest and archive are being written atomically.",
    )?;
    let mut manifest_config = BTreeMap::<String, Value>::from([
        (
            "source".into(),
            json!(display_path(Path::new(&request.data_path))),
        ),
        (
            "broker".into(),
            json!(display_path(Path::new(&request.broker_path))),
        ),
        (
            "m1_source".into(),
            json!(display_path(Path::new(&request.m1_data_path))),
        ),
        (
            "databank".into(),
            json!(display_path(Path::new(&request.databank_path))),
        ),
        ("engine_tier".into(), json!(quantforge_tick::ENGINE_TIER)),
        (
            "discover_config".into(),
            serde_json::to_value(&bank.config).map_err(|error| error.to_string())?,
        ),
        ("generations_requested".into(), json!(request.generations)),
        ("starting_generation".into(), json!(starting_generation)),
        (
            "continued".into(),
            json!(request.mode == DiscoverMode::Continue),
        ),
        ("data_quality_grade".into(), json!(quality.grade)),
        ("data_quality_score".into(), json!(quality.score)),
        ("m1_data_hash".into(), json!(&m1.dataset.data_hash)),
        ("m1_quality_grade".into(), json!(m1_quality.grade)),
        ("m1_quality_score".into(), json!(m1_quality.score)),
        ("desktop_job".into(), json!(true)),
        (
            "promotion_split".into(),
            json!(bank.data_hash != loaded.dataset.data_hash),
        ),
        ("validation_fraction".into(), json!(validation_fraction)),
        ("sealed_fraction".into(), json!(sealed_fraction)),
        (
            "stopped_early".into(),
            json!(completed_now < request.generations),
        ),
    ]);
    if let Some(metadata) = &loaded.metadata {
        manifest_config.insert("metadata_hash".into(), json!(metadata.metadata_hash));
    }
    if let Some(metadata) = &m1.metadata {
        manifest_config.insert("m1_metadata_hash".into(), json!(metadata.metadata_hash));
    }
    if let Some(recipe_hash) = continuation_recipe_hash {
        manifest_config.insert("continued_recipe_hash".into(), json!(recipe_hash));
    }
    let manifest = RunManifest::new(
        "evolve",
        RunRecipe {
            data_hash: Some(bank.data_hash.clone()),
            broker_spec_hash: Some(bank.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: Some(bank.config.seed),
            config: manifest_config,
            override_flags: Vec::new(),
        },
    )
    .map_err(|error| error.to_string())?;
    let artifact = EvolveArtifact {
        manifest,
        source: display_path(Path::new(&request.data_path)),
        broker: display_path(Path::new(&request.broker_path)),
        metadata_hash: loaded.metadata.map(|value| value.metadata_hash),
        data_quality: quality,
        coverage: bank.coverage(),
        qd_score: bank.qd_score(),
        databank: bank,
    };
    match request.mode {
        DiscoverMode::New => {
            write_json_new(&request.databank_path, &artifact).map_err(|error| error.to_string())?;
        }
        DiscoverMode::Continue => {
            write_json_versioned(&request.databank_path, &artifact)
                .map_err(|error| error.to_string())?;
        }
    }

    let mut view = job
        .write()
        .map_err(|_| "discover job state is unavailable".to_owned())?;
    view.status = "completed";
    view.phase = if completed_now < request.generations {
        "Stopped and checkpointed".into()
    } else {
        "Discovery checkpoint complete".into()
    };
    view.output_path = Some(display_path(Path::new(&request.databank_path)));
    view.message = format!(
        "Saved {} niches after {} evaluations.",
        view.coverage, view.evaluation_count
    );
    Ok(())
}

fn development_partition(
    dataset: &quantforge_data::BarDataset,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<quantforge_data::BarDataset, String> {
    let plan = quantforge_quality::DataSplitPlan::chronological(
        dataset,
        validation_fraction,
        sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let bars = dataset.bars[..plan.development.bar_count].to_vec();
    Ok(quantforge_data::BarDataset {
        data_hash: quantforge_data::bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: dataset.delimiter,
        source_timezone: dataset.source_timezone.clone(),
        bars,
    })
}

fn new_config(request: &DiscoverRequest) -> Result<DiscoverConfig, String> {
    let commission = request
        .commission_per_lot_round_turn
        .ok_or_else(|| "commission is required for a new databank".to_owned())?;
    Ok(DiscoverConfig {
        initial_candidates: request.initial_candidates.unwrap_or(500),
        batch_size: request.batch_size.unwrap_or(200),
        correlation_threshold: request.correlation_threshold.unwrap_or(0.88),
        novelty_weight: request.novelty_weight.unwrap_or(10.0),
        tournament_size: 4,
        structural_mutation_probability: 0.18,
        seed: request.seed.unwrap_or(42),
        gates: GateConfig {
            minimum_trades: request.minimum_trades.unwrap_or(20),
            maximum_drawdown_percent: request.maximum_drawdown_percent.unwrap_or(30.0),
            minimum_return_percent: request.minimum_return_percent.unwrap_or(0.0),
            minimum_profit_factor: request.minimum_profit_factor.unwrap_or(1.0),
        },
        precision: quantforge_discover::PrecisionGateConfig {
            minimum_return_retention: request.minimum_m1_return_retention.unwrap_or(0.95),
        },
        scout: ScoutConfig {
            initial_balance: request.initial_balance.unwrap_or(100_000.0),
            same_bar_policy: SameBarPolicy::Conservative,
            costs: CostModel {
                fallback_spread_points: request.fallback_spread_points,
                adverse_slippage_points_per_side: request.slippage_points_per_side.unwrap_or(0.0),
                commission_per_lot_round_turn: commission,
                max_spread_points: request.max_spread_points,
                include_costs_in_risk: true,
            },
        },
    })
}

fn wait_if_paused(
    job: &Arc<RwLock<DiscoverJobView>>,
    paused: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    while paused.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }
    if !stop.load(Ordering::SeqCst) {
        let mut view = job
            .write()
            .map_err(|_| "discover job state is unavailable".to_owned())?;
        if view.status == "paused" {
            view.status = "running";
        }
    }
    Ok(())
}

fn update_phase(
    job: &Arc<RwLock<DiscoverJobView>>,
    phase: &str,
    message: &str,
) -> Result<(), String> {
    let mut view = job
        .write()
        .map_err(|_| "discover job state is unavailable".to_owned())?;
    if view.status != "paused" {
        view.status = "running";
    }
    view.phase = phase.into();
    view.message = message.into();
    Ok(())
}

fn update_bank(
    job: &Arc<RwLock<DiscoverJobView>>,
    bank: &Databank,
    completed_now: u64,
    requested: u64,
) -> Result<(), String> {
    let telemetry = &bank.telemetry;
    let rejected_total = telemetry.rejected_gate
        + telemetry.rejected_clone
        + telemetry.rejected_correlated
        + telemetry.rejected_niche_not_improved
        + telemetry.rejected_precision
        + telemetry.rejected_evaluation;
    let mut view = job
        .write()
        .map_err(|_| "discover job state is unavailable".to_owned())?;
    view.completed_generations = completed_now;
    view.requested_generations = requested;
    view.evaluation_count = bank.evaluation_count;
    view.coverage = bank.coverage();
    view.qd_score = bank.qd_score();
    view.rejected_clone = telemetry.rejected_clone;
    view.rejected_correlated = telemetry.rejected_correlated;
    view.rejected_total = rejected_total;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture(name: &str) -> String {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("fixtures")
            .join(name)
            .display()
            .to_string()
    }

    fn request(databank_path: String) -> DiscoverRequest {
        DiscoverRequest {
            mode: DiscoverMode::New,
            data_path: fixture("EURUSD_M15_sample.tsv"),
            metadata_path: Some(fixture("EURUSD_M15_sample.metadata.csv")),
            source_timezone: None,
            m1_data_path: fixture("EURUSD_M1_sample.tsv"),
            m1_metadata_path: Some(fixture("EURUSD_M1_sample.metadata.csv")),
            m1_source_timezone: None,
            broker_path: fixture("EURUSD_fixture_broker.json"),
            databank_path,
            generations: 1,
            initial_candidates: Some(16),
            batch_size: Some(8),
            correlation_threshold: Some(0.88),
            novelty_weight: Some(10.0),
            seed: Some(42),
            minimum_trades: Some(0),
            maximum_drawdown_percent: Some(100.0),
            minimum_return_percent: Some(-100.0),
            minimum_profit_factor: Some(0.0),
            minimum_m1_return_retention: Some(0.95),
            commission_per_lot_round_turn: Some(0.0),
            slippage_points_per_side: Some(0.0),
            fallback_spread_points: None,
            max_spread_points: None,
            initial_balance: Some(100_000.0),
            promotion_split: None,
            validation_fraction: None,
            sealed_fraction: None,
        }
    }

    #[test]
    fn continuation_rejects_new_search_overrides() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("bank.json");
        fs::write(&path, "{}").expect("placeholder databank");
        let mut request = request(path.display().to_string());
        request.mode = DiscoverMode::Continue;
        let error = validate_request(&request).expect_err("override must fail");
        assert!(error.contains("immutable configuration"));
    }

    #[test]
    fn promotion_search_uses_only_the_development_partition() {
        let loaded = load_data_source(
            &fixture("EURUSD_M15_sample.tsv"),
            Some(&fixture("EURUSD_M15_sample.metadata.csv")),
            None,
        )
        .expect("fixture should load");
        let plan = quantforge_quality::DataSplitPlan::chronological(&loaded.dataset, 0.2, 0.2)
            .expect("split should be valid");
        let development = development_partition(&loaded.dataset, 0.2, 0.2)
            .expect("development partition should materialize");
        assert_eq!(development.bars.len(), plan.development.bar_count);
        assert_eq!(development.data_hash, plan.development.data_hash);
        assert_ne!(development.data_hash, loaded.dataset.data_hash);
    }

    #[test]
    fn native_worker_writes_a_verified_databank() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("bank.json");
        let request = request(path.display().to_string());
        let job = Arc::new(RwLock::new(DiscoverJobView::idle()));
        run_discovery(
            request,
            &job,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
        )
        .expect("discovery should complete");

        let artifact: EvolveArtifact =
            serde_json::from_slice(&fs::read(&path).expect("saved artifact"))
                .expect("valid artifact JSON");
        verify_artifact(&artifact).expect("desktop must accept its own artifact");
        assert_eq!(artifact.databank.completed_generations, 1);
        assert_eq!(job.read().expect("job state").status, "completed");
    }

    #[test]
    fn worker_never_persists_an_unloadable_empty_archive() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("empty-bank.json");
        let mut request = request(path.display().to_string());
        request.minimum_trades = Some(usize::MAX);
        let job = Arc::new(RwLock::new(DiscoverJobView::idle()));
        let error = run_discovery(
            request,
            &job,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
        )
        .expect_err("an empty archive must fail before persistence");

        assert!(error.contains("no loadable checkpoint"));
        assert!(!path.exists());
    }
}
