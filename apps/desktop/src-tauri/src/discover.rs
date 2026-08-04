use crate::data_lab::{
    build_decision_from_m1, display_path, load_bound_broker, load_data_source, load_quote_sidecar,
};
use crate::databank::{EvolveArtifact, verify_artifact};
use quantforge_data::{
    BarDataset, bar_content_hash, build_timeframe_from_m1, infer_median_interval_ms,
};
use quantforge_discover::{
    ConditionBakeoffConfig, ConditionBakeoffReport, DEFAULT_FX_PACK, Databank, DiscoverConfig,
    DiscoverRunMode, GateConfig, PackSymbol, SearchRangeProfile, UniversalGrammarConfig,
    evolve_new_with_pack_and_quotes, run_condition_bakeoff as evolve_condition_bakeoff,
};
use quantforge_eval::{CostModel, SameBarPolicy, ScoutConfig};
use quantforge_storage::{RunManifest, RunRecipe, write_json_new, write_json_versioned};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverMode {
    New,
    Continue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
enum DecisionTimeframe {
    H1,
    M15,
}

impl DecisionTimeframe {
    const fn interval_ms(self) -> i64 {
        match self {
            Self::H1 => 3_600_000,
            Self::M15 => 900_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    mode: DiscoverMode,
    data_path: String,
    /// M15 is deterministically built from the bound M1 execution stream.
    decision_timeframe: Option<DecisionTimeframe>,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    broker_path: String,
    databank_path: String,
    /// Soft generation budget. Ignored as a hard stop when `run_until_stopped` is true
    /// and this is 0; otherwise the worker stops after this many generations.
    generations: u64,
    /// When true (default), keep evolving until Stop (or optional soft `generations` budget).
    run_until_stopped: Option<bool>,
    initial_candidates: Option<usize>,
    batch_size: Option<usize>,
    correlation_threshold: Option<f64>,
    novelty_weight: Option<f64>,
    seed: Option<u64>,
    /// Scout (early H1) gate thresholds.
    minimum_trades: Option<usize>,
    maximum_drawdown_percent: Option<f64>,
    minimum_return_percent: Option<f64>,
    minimum_profit_factor: Option<f64>,
    minimum_return_drawdown: Option<f64>,
    /// Deposit (final pot) gate thresholds.
    deposit_minimum_trades: Option<usize>,
    deposit_maximum_drawdown_percent: Option<f64>,
    deposit_minimum_return_percent: Option<f64>,
    deposit_minimum_profit_factor: Option<f64>,
    deposit_minimum_return_drawdown: Option<f64>,
    minimum_m1_return_retention: Option<f64>,
    oos1_expectancy_retention: Option<f64>,
    /// Downstream preference only — Discover never runs M1. When true, portfolio
    /// / export may insist on an explicit M1 fidelity pass after the run.
    require_m1_precision: Option<bool>,
    /// Legacy selected-TF compatibility profile. Explicit feature toggles below
    /// take precedence; they widen search without forcing M1 during Discover.
    simple_exits: Option<bool>,
    allow_break_even: Option<bool>,
    allow_trailing_stops: Option<bool>,
    allow_partial_exits: Option<bool>,
    /// Entry order kinds the search may sample. At least one must stay enabled.
    allow_market_entries: Option<bool>,
    allow_stop_entries: Option<bool>,
    allow_limit_entries: Option<bool>,
    flatten_at_22: Option<bool>,
    end_of_day_hour: Option<u8>,
    /// Broker-local hour from which entries may be placed (inclusive).
    entry_window_start_hour: Option<u32>,
    /// Broker-local hour from which entries stop being placed (exclusive).
    entry_window_end_hour: Option<u32>,
    /// Cap to one filled entry per broker-local day (default true).
    max_one_entry_per_day: Option<bool>,
    mutate_after_elites: Option<usize>,
    random_fill_fraction: Option<f64>,
    worker_threads: Option<usize>,
    require_m1_robustness: Option<bool>,
    robustness_folds: Option<usize>,
    robustness_monte_carlo_trials: Option<usize>,
    robustness_monte_carlo_block_length: Option<usize>,
    robustness_monte_carlo_skip_trade_probability: Option<f64>,
    robustness_monte_carlo_p80_profit_retention: Option<f64>,
    robustness_monte_carlo_max_drawdown_ratio: Option<f64>,
    robustness_neighborhood_samples: Option<usize>,
    /// Size of the ±% jitter applied to every numeric gene (default 0.20).
    robustness_perturbation_fraction: Option<f64>,
    /// Fraction of ±param neighbors that must survive (default 0.7; Quota uses 0.5).
    minimum_neighborhood_survival_fraction: Option<f64>,
    /// Broker-local calendar-year folds; every year must pass (strict opt-in).
    calendar_year_folds: Option<bool>,
    /// Hard deflated-Sharpe floor; omit for report-only.
    minimum_deflated_trade_sharpe: Option<f64>,
    /// Require this many FX pack symbols profitable with identical params (0 = off).
    multi_symbol_minimum_pass: Option<usize>,
    /// Directory of pack H1 TSV + broker JSON files for the multi-symbol screen.
    pack_data_dir: Option<String>,
    /// Family-free entry/exit cardinality and completed-bar shift bounds
    /// (entry 2..=4, exit 1..=3). This is the only grammar selector.
    universal_grammar: Option<UniversalGrammarConfig>,
    /// `fast_scout`, `full_harvest`, or `quota_harvest`.
    run_mode: Option<String>,
    /// Early-stop when accepted pot reaches this size (Fast Scout / Quota).
    early_stop_pot_elites: Option<usize>,
    /// Early-stop when databank reaches this many elites (Quota Harvest default 20).
    target_databank_elites: Option<usize>,
    search_ranges: Option<SearchRangeProfile>,
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
pub struct EvaluationErrorCount {
    message: String,
    count: u64,
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
    run_until_stopped: bool,
    evaluation_count: u64,
    accepted_total: u64,
    /// Distinct niches currently occupied (= elites in the pot).
    pot_elites: usize,
    pot_new_niches: u64,
    databank_elites: usize,
    target_databank_elites: Option<usize>,
    mutate_after_elites: usize,
    breeding_active: bool,
    worker_threads: usize,
    coverage: usize,
    qd_score: f64,
    rejected_gate: u64,
    rejected_deposit_gate: u64,
    rejected_precision: u64,
    rejected_ambiguous: u64,
    rejected_oos1: u64,
    rejected_m1_fidelity: u64,
    rejected_walk_forward: u64,
    rejected_monte_carlo: u64,
    rejected_param_neighborhood: u64,
    rejected_multi_symbol: u64,
    rejected_deflated_sharpe: u64,
    rejected_clone: u64,
    rejected_correlated: u64,
    rejected_niche_not_improved: u64,
    rejected_evaluation: u64,
    rejected_total: u64,
    evaluations_per_hour: f64,
    accepts_per_hour: f64,
    best_is_expectancy: Option<f64>,
    best_oos1_expectancy: Option<f64>,
    top_evaluation_errors: Vec<EvaluationErrorCount>,
    m1_bars_repaired: u64,
    started_at_ms: Option<u64>,
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
            run_until_stopped: true,
            evaluation_count: 0,
            accepted_total: 0,
            pot_elites: 0,
            pot_new_niches: 0,
            databank_elites: 0,
            target_databank_elites: None,
            mutate_after_elites: 300,
            breeding_active: false,
            worker_threads: 0,
            coverage: 0,
            qd_score: 0.0,
            rejected_gate: 0,
            rejected_deposit_gate: 0,
            rejected_precision: 0,
            rejected_ambiguous: 0,
            rejected_oos1: 0,
            rejected_m1_fidelity: 0,
            rejected_walk_forward: 0,
            rejected_monte_carlo: 0,
            rejected_param_neighborhood: 0,
            rejected_multi_symbol: 0,
            rejected_deflated_sharpe: 0,
            rejected_clone: 0,
            rejected_correlated: 0,
            rejected_niche_not_improved: 0,
            rejected_evaluation: 0,
            rejected_total: 0,
            evaluations_per_hour: 0.0,
            accepts_per_hour: 0.0,
            best_is_expectancy: None,
            best_oos1_expectancy: None,
            top_evaluation_errors: Vec::new(),
            m1_bars_repaired: 0,
            started_at_ms: None,
            stop_requested: false,
            message: "Configure a new search or continue an existing databank.".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionBakeoffRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    broker_path: String,
    generations: u64,
    initial_candidates: usize,
    seed: u64,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    fallback_spread_points: Option<f64>,
    validation_fraction: f64,
    sealed_fraction: f64,
    /// Entry-condition counts to compare on equal budget. Defaults to 2, 3, 4.
    #[serde(default)]
    entry_condition_counts: Vec<usize>,
}

#[tauri::command]
pub fn run_condition_bakeoff(
    request: ConditionBakeoffRequest,
) -> Result<ConditionBakeoffReport, String> {
    let entry_condition_counts = parse_entry_condition_counts(&request.entry_condition_counts)?;
    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let m1_loaded = load_data_source(
        &request.m1_data_path,
        request.m1_metadata_path.as_deref(),
        request.m1_source_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    load_bound_broker(&request.broker_path, m1_loaded.metadata.as_ref())?;
    let validation_fraction = request.validation_fraction.clamp(0.05, 0.4);
    let sealed_fraction = request.sealed_fraction.clamp(0.05, 0.4);
    if validation_fraction + sealed_fraction >= 0.9 {
        return Err(format!(
            "validation ({validation_fraction:.2}) + sealed ({sealed_fraction:.2}) leaves less than 10% for IS"
        ));
    }
    let search_h1 = development_partition(&loaded.dataset, validation_fraction, sealed_fraction)?;
    let oos1 = oos1_partition(&loaded.dataset, validation_fraction, sealed_fraction)?;
    let m1_is = development_partition(&m1_loaded.dataset, validation_fraction, sealed_fraction)?;
    let mut discover = DiscoverConfig {
        run_mode: DiscoverRunMode::FastScout,
        initial_candidates: request.initial_candidates.max(20),
        batch_size: request.initial_candidates.clamp(10, 30),
        seed: request.seed,
        require_m1_robustness: false,
        require_m1_precision: false,
        worker_threads: 0,
        ..DiscoverConfig::default()
    };
    discover.scout.costs.commission_per_lot_round_turn = request.commission_per_lot_round_turn;
    discover.scout.costs.adverse_slippage_points_per_side = request.slippage_points_per_side;
    discover.scout.costs.fallback_spread_points = request.fallback_spread_points;
    let config = ConditionBakeoffConfig {
        discover,
        generations: request.generations.max(1),
        entry_condition_counts,
    };
    evolve_condition_bakeoff(
        &search_h1,
        Some(&oos1),
        &m1_is,
        &broker,
        &[],
        &broker.symbol,
        config,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn start_discover(
    mut request: DiscoverRequest,
    state: State<'_, DiscoverState>,
) -> Result<DiscoverJobView, String> {
    if request.mode == DiscoverMode::New && request.databank_path.trim().is_empty() {
        request.databank_path = automatic_databank_path(&request)?;
    }
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
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis() as u64;
    let job_id = format!("desktop-{now_ms}");
    let run_until_stopped = request.run_until_stopped.unwrap_or(true);
    let started = DiscoverJobView {
        job_id: Some(job_id),
        status: "running",
        mode: Some(request.mode.into()),
        phase: "Loading and validating inputs".into(),
        output_path: Some(display_path(Path::new(&request.databank_path))),
        completed_generations: 0,
        requested_generations: request.generations,
        run_until_stopped,
        evaluation_count: 0,
        accepted_total: 0,
        pot_elites: 0,
        pot_new_niches: 0,
        databank_elites: 0,
        target_databank_elites: request.target_databank_elites,
        mutate_after_elites: request.mutate_after_elites.unwrap_or(300),
        breeding_active: false,
        worker_threads: request.worker_threads.unwrap_or(0),
        coverage: 0,
        qd_score: 0.0,
        rejected_gate: 0,
        rejected_deposit_gate: 0,
        rejected_precision: 0,
        rejected_ambiguous: 0,
        rejected_oos1: 0,
        rejected_m1_fidelity: 0,
        rejected_walk_forward: 0,
        rejected_monte_carlo: 0,
        rejected_param_neighborhood: 0,
        rejected_multi_symbol: 0,
        rejected_deflated_sharpe: 0,
        rejected_clone: 0,
        rejected_correlated: 0,
        rejected_niche_not_improved: 0,
        rejected_evaluation: 0,
        rejected_total: 0,
        evaluations_per_hour: 0.0,
        accepts_per_hour: 0.0,
        best_is_expectancy: None,
        best_oos1_expectancy: None,
        top_evaluation_errors: Vec::new(),
        m1_bars_repaired: 0,
        started_at_ms: Some(now_ms),
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

/// New discovery archives must never overwrite a previous run. The UI may
/// leave this blank; in that case derive a human-readable, collision-proof
/// archive path beside the QuantForge workspace instead of opening a save
/// dialog for every run.
fn automatic_databank_path(request: &DiscoverRequest) -> Result<String, String> {
    let source = Path::new(&request.data_path);
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "cannot derive an archive name from decision OHLC path".to_owned())?;
    let symbol = stem
        .split(['_', '-'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("strategy")
        .to_ascii_uppercase();
    let timeframe = match request.decision_timeframe.unwrap_or(DecisionTimeframe::H1) {
        DecisionTimeframe::H1 => "H1",
        DecisionTimeframe::M15 => "M15",
    };
    let root = source
        .ancestors()
        .find(|candidate| {
            candidate.file_name().and_then(|name| name.to_str()) == Some("QuantForge")
        })
        .map(Path::to_path_buf)
        .or_else(|| source.parent().map(Path::to_path_buf))
        .ok_or_else(|| "cannot derive an archive directory from decision OHLC path".to_owned())?;
    let directory = root.join("runs").join(&symbol).join("Databank");
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let base = format!("{symbol}_{timeframe}_databank_{now_ms}");
    let mut candidate = directory.join(format!("{base}.json"));
    let mut suffix = 2usize;
    while candidate.exists() {
        candidate = directory.join(format!("{base}_{suffix}.json"));
        suffix += 1;
    }
    Ok(candidate.display().to_string())
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
    if request.mode == DiscoverMode::New {
        if let Some(grammar) = request.universal_grammar.as_ref() {
            validate_universal_grammar(grammar)?;
        }
        if let Some(mode) = request.run_mode.as_deref() {
            if parse_run_mode(mode).is_none() {
                return Err(format!(
                    "unknown run mode '{mode}' (use fast_scout, full_harvest, or quota_harvest)"
                ));
            }
        }
        let validation = request.validation_fraction.unwrap_or(0.2);
        let sealed = request.sealed_fraction.unwrap_or(0.2);
        normalize_split_fractions(validation, sealed)?;
    }
    let run_until_stopped = request.run_until_stopped.unwrap_or(true);
    if !run_until_stopped && request.generations == 0 {
        return Err("at least one generation is required when not running until stopped".into());
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
            request.universal_grammar.is_some(),
            request.run_mode.is_some(),
            request.early_stop_pot_elites.is_some(),
            request.minimum_trades.is_some(),
            request.maximum_drawdown_percent.is_some(),
            request.minimum_return_percent.is_some(),
            request.minimum_profit_factor.is_some(),
            request.minimum_return_drawdown.is_some(),
            request.deposit_minimum_trades.is_some(),
            request.deposit_maximum_drawdown_percent.is_some(),
            request.deposit_minimum_return_percent.is_some(),
            request.deposit_minimum_profit_factor.is_some(),
            request.deposit_minimum_return_drawdown.is_some(),
            request.minimum_m1_return_retention.is_some(),
            request.require_m1_precision.is_some(),
            request.simple_exits.is_some(),
            request.allow_break_even.is_some(),
            request.allow_trailing_stops.is_some(),
            request.allow_partial_exits.is_some(),
            request.allow_market_entries.is_some(),
            request.allow_stop_entries.is_some(),
            request.allow_limit_entries.is_some(),
            request.flatten_at_22.is_some(),
            request.entry_window_start_hour.is_some(),
            request.entry_window_end_hour.is_some(),
            request.max_one_entry_per_day.is_some(),
            request.mutate_after_elites.is_some(),
            request.random_fill_fraction.is_some(),
            request.worker_threads.is_some(),
            request.require_m1_robustness.is_some(),
            request.robustness_folds.is_some(),
            request.robustness_monte_carlo_trials.is_some(),
            request.robustness_monte_carlo_block_length.is_some(),
            request
                .robustness_monte_carlo_skip_trade_probability
                .is_some(),
            request
                .robustness_monte_carlo_p80_profit_retention
                .is_some(),
            request
                .robustness_monte_carlo_max_drawdown_ratio
                .is_some(),
            request.robustness_neighborhood_samples.is_some(),
            request.robustness_perturbation_fraction.is_some(),
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

/// Locate the canonical bid/ask M1 sidecar written beside an imported MT5
/// pack. Decision-timeframe paths are allowed here because H1/M15 packs are
/// derived from the same M1 stream and therefore share its sibling sidecar.
pub fn infer_quote_path_public(m1_path: &str) -> Option<PathBuf> {
    infer_quote_path(m1_path)
}

fn infer_quote_path(m1_path: &str) -> Option<PathBuf> {
    let path = Path::new(m1_path);
    let stem = path.file_stem()?.to_str()?;
    let mut candidates = vec![path.with_file_name(format!("{stem}.quotes.csv"))];
    for suffix in ["_H1", "_M15"] {
        if let Some(base) = stem.strip_suffix(suffix) {
            candidates.push(path.with_file_name(format!("{base}_M1.quotes.csv")));
        }
    }
    // Also accept short EURUSD_M1.quotes.csv siblings next to pack M1 files.
    if let Some(symbol) = stem.split('_').find(|part| part.len() >= 6) {
        candidates.push(path.with_file_name(format!("{symbol}_M1.quotes.csv")));
    }
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn metadata_is_canonical_bid_ask(metadata: Option<&quantforge_data::Mt5ExportMetadata>) -> bool {
    let Some(metadata) = metadata else {
        return false;
    };
    let price_basis = metadata.properties.get("price_basis");
    let import_kind = metadata.properties.get("import_kind");
    price_basis.is_some_and(|value| value.eq_ignore_ascii_case("bid"))
        && import_kind.is_some_and(|value| value.to_ascii_lowercase().contains("bid_ask"))
}

fn run_discovery(
    request: DiscoverRequest,
    job: &Arc<RwLock<DiscoverJobView>>,
    paused: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let clock = ActiveClock::new();
    let run_until_stopped = request.run_until_stopped.unwrap_or(true);
    let soft_budget = request.generations;

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
    let quote_path = infer_quote_path(&request.m1_data_path);
    let quote_dataset = quote_path
        .as_ref()
        .map(|path| load_quote_sidecar(path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    if let Some(quotes) = quote_dataset.as_ref() {
        quotes
            .validate_against(&m1.dataset)
            .map_err(|error| format!("quote sidecar does not match M1 data: {error}"))?;
    } else if metadata_is_canonical_bid_ask(m1.metadata.as_ref()) {
        return Err(
            "canonical bid/ask M1 metadata is present but its .quotes.csv sidecar was not found"
                .into(),
        );
    }
    let wants_pending = request.allow_stop_entries.unwrap_or(false)
        || request.allow_limit_entries.unwrap_or(false);
    if wants_pending && quote_dataset.is_none() {
        return Err(
            "stop/limit Discover requires a bid/ask M1 quote sidecar beside the M1 pack \
             (re-import ticks with qf-import-market, then scripts/install_icmarkets_pack.py). \
             Without it pending fills are not MT5-certifiable"
                .into(),
        );
    }
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

    let decision_timeframe = request.decision_timeframe.unwrap_or(DecisionTimeframe::H1);
    let (search_decision, decision_bars_built) = {
        let built = match decision_timeframe {
            DecisionTimeframe::H1 => build_decision_from_m1(&m1.dataset, Some(&loaded.dataset))?,
            DecisionTimeframe::M15 => {
                build_timeframe_from_m1(&m1.dataset, DecisionTimeframe::M15.interval_ms(), None)
                    .map_err(|error| error.to_string())?
            }
        };
        let count = built.bars.len() as u64;
        (built, count)
    };
    let decision_timeframe = decision_timeframe_label(&search_decision)?;
    {
        let mut view = job
            .write()
            .map_err(|_| "discover job state is unavailable".to_owned())?;
        view.m1_bars_repaired = decision_bars_built;
        view.message = format!(
            "Built {decision_bars_built} {decision_timeframe} decision bars from M1 before search."
        );
    }

    let promotion_split = request.promotion_split.unwrap_or(true);
    // New runs take the UI split. Continuations must reuse the fractions sealed
    // into the databank manifest — defaulting to 0.2/0.2 here used to rewrite
    // checkpoints onto a different IS/OOS1/OOS2 cut than the elites were gated on.
    let continued_artifact = match request.mode {
        DiscoverMode::Continue => {
            let bytes = fs::read(&request.databank_path)
                .map_err(|error| format!("cannot read databank: {error}"))?;
            let artifact: EvolveArtifact = serde_json::from_slice(&bytes)
                .map_err(|error| format!("databank JSON is invalid: {error}"))?;
            verify_artifact(&artifact).map_err(|error| error.to_string())?;
            Some(artifact)
        }
        DiscoverMode::New => None,
    };
    let (validation_fraction, sealed_fraction) = match (&request.mode, &continued_artifact) {
        (DiscoverMode::Continue, Some(artifact)) => (
            recipe_fraction(artifact, "validation_fraction", 0.2),
            recipe_fraction(artifact, "sealed_fraction", 0.2),
        ),
        _ => (
            request.validation_fraction.unwrap_or(0.2),
            request.sealed_fraction.unwrap_or(0.2),
        ),
    };
    let (validation_fraction, sealed_fraction) =
        normalize_split_fractions(validation_fraction, sealed_fraction)?;
    let development_dataset = (promotion_split || request.mode == DiscoverMode::Continue)
        .then(|| development_partition(&search_decision, validation_fraction, sealed_fraction))
        .transpose()?;
    let oos1_dataset = (promotion_split || request.mode == DiscoverMode::Continue)
        .then(|| oos1_partition(&search_decision, validation_fraction, sealed_fraction))
        .transpose()?;
    let new_dataset = development_dataset.as_ref().unwrap_or(&search_decision);
    let oos1_ref = oos1_dataset.as_ref();
    // Always retain the full M1 stream.  The evaluator only consumes the M1
    // minutes covered by the supplied decision partition, so this lets OOS1 be
    // replayed at the same precision as IS instead of comparing M1 IS against
    // an H1/M15 OOS result.
    let m1_eval = &m1.dataset;
    let pack = load_fx_pack(
        request.pack_data_dir.as_deref(),
        &broker.symbol,
        validation_fraction,
        sealed_fraction,
        promotion_split || request.mode == DiscoverMode::Continue,
        &decision_timeframe,
    )?;

    let (mut bank, continuation_recipe_hash, starting_generation) = match request.mode {
        DiscoverMode::New => {
            update_phase(
                job,
                "Evaluating initial grammar population",
                &format!(
                    "H1 gates fill the breeding pot only. After breeding unlocks: OOS1 → M1 robustness → M1 fidelity → databank."
                ),
            )?;
            let mut config = new_config(&request)?;
            if !pack.is_empty()
                && config.multi_symbol_minimum_pass == 0
                && request.multi_symbol_minimum_pass.is_none()
            {
                // Default to 6-of-N when a pack is supplied and the UI left the gate unset.
                config.multi_symbol_minimum_pass = 6.min(pack.len() + 1);
            }
            let bank = evolve_new_with_pack_and_quotes(
                new_dataset,
                oos1_ref,
                m1_eval,
                quote_dataset.as_ref(),
                &broker,
                &pack,
                &broker.symbol,
                config,
                0,
            )
            .map_err(|error| error.to_string())?;
            update_bank(job, &bank, 0, soft_budget, run_until_stopped, &clock)?;
            (bank, None, 0u64)
        }
        DiscoverMode::Continue => {
            let artifact = continued_artifact
                .expect("continuation always loads the databank before partitioning");
            let starting_generation = artifact.databank.completed_generations;
            update_bank(
                job,
                &artifact.databank,
                0,
                soft_budget,
                run_until_stopped,
                &clock,
            )?;
            (
                artifact.databank,
                Some(artifact.manifest.recipe_hash),
                starting_generation,
            )
        }
    };

    let mut completed_now = 0u64;
    let mut wrote_checkpoint = Path::new(&request.databank_path).exists();

    if bank.coverage() > 0 {
        write_discover_checkpoint(
            &request,
            &bank,
            display_path(Path::new(&request.data_path)),
            display_path(Path::new(&request.broker_path)),
            loaded
                .metadata
                .as_ref()
                .map(|value| value.metadata_hash.clone()),
            &quality,
            &m1_quality,
            &m1.dataset.data_hash,
            validation_fraction,
            sealed_fraction,
            starting_generation,
            continuation_recipe_hash.clone(),
            completed_now,
            soft_budget,
            run_until_stopped,
            true,
            wrote_checkpoint,
        )?;
        wrote_checkpoint = true;
    }

    let quota_met = |bank: &Databank| -> bool {
        bank.config
            .target_databank_elites
            .is_some_and(|target| bank.elites.len() >= target)
    };

    // Dataset selection is fixed for the run: `validate_resume` requires the
    // bank's hashes to keep matching whatever it is advanced against. Resolving
    // it once lets a single session own the indicator cache for every
    // generation, instead of rebuilding the cache on each one-generation call.
    let evaluation_dataset = if bank.data_hash == search_decision.data_hash {
        &search_decision
    } else {
        development_dataset.as_ref().ok_or_else(|| {
            "this databank was built from an IS partition; enable the identical promotion split to continue it".to_owned()
        })?
    };
    let evaluation_oos1 = if bank.data_hash == search_decision.data_hash {
        None
    } else {
        oos1_ref
    };
    let evaluation_m1 = if bank.execution_data_hash == m1.dataset.data_hash {
        &m1.dataset
    } else {
        m1_eval
    };
    let session = quantforge_discover::EvolutionSession::new(
        bank.config.worker_threads,
        evaluation_dataset.bars.len(),
    )
    .map_err(|error| error.to_string())?;

    loop {
        wait_if_paused(job, paused, stop, &clock)?;
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if quota_met(&bank) {
            break;
        }
        if !run_until_stopped && completed_now >= soft_budget {
            break;
        }
        if run_until_stopped && soft_budget > 0 && completed_now >= soft_budget {
            break;
        }

        let phase_label = if run_until_stopped && soft_budget == 0 {
            format!("Evolving generation {}", completed_now + 1)
        } else {
            format!(
                "Evolving generation {} of {}",
                completed_now + 1,
                soft_budget.max(1)
            )
        };
        update_phase(
            job,
            &phase_label,
            "Candidates enter the pot on Selected-TF H1 gates only. After breeding unlocks: OOS1 databank gate → M1 robustness → M1 fidelity → databank (M1 metrics only).",
        )?;

        bank = session
            .advance_with_quotes(
                bank,
                evaluation_dataset,
                evaluation_oos1,
                evaluation_m1,
                quote_dataset.as_ref(),
                &broker,
                &pack,
                &broker.symbol,
                1,
            )
            .map_err(|error| error.to_string())?;
        completed_now += 1;
        update_bank(
            job,
            &bank,
            completed_now,
            soft_budget,
            run_until_stopped,
            &clock,
        )?;

        if bank.coverage() > 0 {
            write_discover_checkpoint(
                &request,
                &bank,
                display_path(Path::new(&request.data_path)),
                display_path(Path::new(&request.broker_path)),
                loaded
                    .metadata
                    .as_ref()
                    .map(|value| value.metadata_hash.clone()),
                &quality,
                &m1_quality,
                &m1.dataset.data_hash,
                validation_fraction,
                sealed_fraction,
                starting_generation,
                continuation_recipe_hash.clone(),
                completed_now,
                soft_budget,
                run_until_stopped,
                true,
                wrote_checkpoint,
            )?;
            wrote_checkpoint = true;
            if let Ok(mut view) = job.write() {
                view.output_path = Some(display_path(Path::new(&request.databank_path)));
                view.message = format!(
                    "Bank growing: {} niches after {} evaluations.",
                    bank.coverage(),
                    bank.evaluation_count
                );
            }
        }

        // Quota Harvest (and any run with a databank target): stop when filled.
        if quota_met(&bank) {
            break;
        }
    }

    finish_discovery(
        request,
        job,
        bank,
        &loaded,
        &m1,
        &quality,
        &m1_quality,
        validation_fraction,
        sealed_fraction,
        starting_generation,
        continuation_recipe_hash,
        completed_now,
        soft_budget,
        run_until_stopped,
        wrote_checkpoint,
        &clock,
    )
}

#[allow(clippy::too_many_arguments)]
fn finish_discovery(
    request: DiscoverRequest,
    job: &Arc<RwLock<DiscoverJobView>>,
    bank: Databank,
    loaded: &crate::data_lab::LoadedDataSource,
    m1: &crate::data_lab::LoadedDataSource,
    quality: &quantforge_data::DataQualityReport,
    m1_quality: &quantforge_data::DataQualityReport,
    validation_fraction: f64,
    sealed_fraction: f64,
    starting_generation: u64,
    continuation_recipe_hash: Option<quantforge_core::ContentHash>,
    completed_now: u64,
    soft_budget: u64,
    run_until_stopped: bool,
    wrote_checkpoint: bool,
    clock: &ActiveClock,
) -> Result<(), String> {
    update_bank(
        job,
        &bank,
        completed_now,
        soft_budget,
        run_until_stopped,
        clock,
    )?;

    if bank.elites.is_empty() {
        let funnel = funnel_summary(&bank);
        let mut view = job
            .write()
            .map_err(|_| "discover job state is unavailable".to_owned())?;
        view.status = "completed";
        view.phase = "Completed with an empty bank".into();
        view.message = format!(
            "No elites passed the post-breed pipeline (OOS1 → M1 robustness → M1) after {} evaluations across {} generations. {funnel} Keep searching until breeding unlocks, loosen gates, or check data.",
            bank.evaluation_count, completed_now
        );
        view.output_path = None;
        return Ok(());
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
    write_discover_checkpoint(
        &request,
        &bank,
        display_path(Path::new(&request.data_path)),
        display_path(Path::new(&request.broker_path)),
        loaded
            .metadata
            .as_ref()
            .map(|value| value.metadata_hash.clone()),
        quality,
        m1_quality,
        &m1.dataset.data_hash,
        validation_fraction,
        sealed_fraction,
        starting_generation,
        continuation_recipe_hash,
        completed_now,
        soft_budget,
        run_until_stopped,
        false,
        wrote_checkpoint,
    )?;

    let mut view = job
        .write()
        .map_err(|_| "discover job state is unavailable".to_owned())?;
    view.status = "completed";
    let quota_complete = bank
        .config
        .target_databank_elites
        .is_some_and(|target| bank.elites.len() >= target);
    view.phase = if quota_complete {
        format!("Quota complete · {} databank elites", bank.elites.len())
    } else if stop_was_early(completed_now, soft_budget, run_until_stopped) {
        "Stopped and checkpointed".into()
    } else {
        "Discovery checkpoint complete".into()
    };
    view.output_path = Some(display_path(Path::new(&request.databank_path)));
    view.message = if quota_complete {
        format!(
            "Reached databank quota ({}/{}). Saved after {} evaluations. Start a new Discover for the next family or asset.",
            bank.elites.len(),
            bank.config.target_databank_elites.unwrap_or(0),
            view.evaluation_count
        )
    } else {
        format!(
            "Saved {} niches after {} evaluations.",
            view.coverage, view.evaluation_count
        )
    };
    Ok(())
}

fn stop_was_early(completed_now: u64, soft_budget: u64, run_until_stopped: bool) -> bool {
    if run_until_stopped && soft_budget == 0 {
        true
    } else {
        soft_budget > 0 && completed_now < soft_budget
    }
}

fn funnel_summary(bank: &Databank) -> String {
    let telemetry = &bank.telemetry;
    format!(
        "Rejects — scout {}, deposit {}, ambiguous {}, M1 retention {}, WF {}, MC {}, param {}, OOS1 {}, clone {}, corr {}, niche {}, eval {}.",
        telemetry.rejected_gate,
        telemetry.rejected_deposit_gate,
        telemetry.rejected_ambiguous,
        telemetry.rejected_m1_fidelity,
        telemetry.rejected_walk_forward,
        telemetry.rejected_monte_carlo,
        telemetry.rejected_param_neighborhood,
        telemetry.rejected_oos1,
        telemetry.rejected_clone,
        telemetry.rejected_correlated,
        telemetry.rejected_niche_not_improved,
        telemetry.rejected_evaluation
    )
}

#[allow(clippy::too_many_arguments)]
fn write_discover_checkpoint(
    request: &DiscoverRequest,
    bank: &Databank,
    source: String,
    broker: String,
    metadata_hash: Option<quantforge_core::ContentHash>,
    quality: &quantforge_data::DataQualityReport,
    m1_quality: &quantforge_data::DataQualityReport,
    m1_data_hash: &quantforge_core::ContentHash,
    validation_fraction: f64,
    sealed_fraction: f64,
    starting_generation: u64,
    continuation_recipe_hash: Option<quantforge_core::ContentHash>,
    completed_now: u64,
    soft_budget: u64,
    run_until_stopped: bool,
    partial: bool,
    already_exists: bool,
) -> Result<(), String> {
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
        ("generations_requested".into(), json!(soft_budget)),
        ("starting_generation".into(), json!(starting_generation)),
        (
            "continued".into(),
            json!(request.mode == DiscoverMode::Continue),
        ),
        ("data_quality_grade".into(), json!(quality.grade)),
        ("data_quality_score".into(), json!(quality.score)),
        ("m1_data_hash".into(), json!(m1_data_hash)),
        ("m1_quality_grade".into(), json!(m1_quality.grade)),
        ("m1_quality_score".into(), json!(m1_quality.score)),
        ("desktop_job".into(), json!(true)),
        ("promotion_split".into(), json!(true)),
        ("validation_fraction".into(), json!(validation_fraction)),
        ("sealed_fraction".into(), json!(sealed_fraction)),
        (
            "stopped_early".into(),
            json!(stop_was_early(
                completed_now,
                soft_budget,
                run_until_stopped
            )),
        ),
        ("run_until_stopped".into(), json!(run_until_stopped)),
        ("partial_checkpoint".into(), json!(partial)),
        (
            "require_m1_precision".into(),
            json!(bank.config.require_m1_precision),
        ),
        // Discover seals research-grade until at least one elite survives the
        // post-breed M1 pipeline into the databank.
        ("research_grade".into(), json!(bank.elites.is_empty())),
        ("simple_exits".into(), json!(bank.config.simple_exits)),
        (
            "max_one_entry_per_day".into(),
            json!(bank.config.max_one_entry_per_day),
        ),
        (
            "m1_fidelity_verified".into(),
            json!(bank.elites.iter().any(|elite| elite.robustness.is_some())),
        ),
        (
            "require_m1_robustness".into(),
            json!(bank.config.require_m1_robustness),
        ),
    ]);
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
        source,
        broker,
        metadata_hash,
        data_quality: quality.clone(),
        coverage: bank.coverage(),
        qd_score: bank.qd_score(),
        databank: bank.clone(),
    };
    if already_exists || request.mode == DiscoverMode::Continue {
        write_json_versioned(&request.databank_path, &artifact)
            .map_err(|error| error.to_string())?;
    } else {
        write_json_new(&request.databank_path, &artifact).map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// Load matching decision-timeframe pack symbols for the identical-parameter
/// multi-symbol gate.
///
/// Expects files named like `*_{SYMBOL}_{TIMEFRAME}_*.tsv`, optional
/// `*.metadata.csv`, and `{SYMBOL}.broker.json`.
fn load_fx_pack(
    pack_dir: Option<&str>,
    primary_symbol: &str,
    validation_fraction: f64,
    sealed_fraction: f64,
    apply_promotion_split: bool,
    decision_timeframe: &str,
) -> Result<Vec<PackSymbol>, String> {
    let Some(dir) = pack_dir.filter(|value| !value.is_empty()) else {
        return Ok(Vec::new());
    };
    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        return Err(format!("pack data directory is not a directory: {dir}"));
    }
    let mut pack = Vec::new();
    for symbol in DEFAULT_FX_PACK {
        if symbol.eq_ignore_ascii_case(primary_symbol) {
            continue;
        }
        let broker_path = dir_path.join(format!("{symbol}.broker.json"));
        if !broker_path.is_file() {
            continue;
        }
        let Some(decision_path) = find_timeframe_tsv(dir_path, symbol, decision_timeframe)? else {
            continue;
        };
        let market_broker = load_bound_broker(&display_path(&broker_path), None)?;
        // `foo.tsv` → `foo.metadata.csv` beside it.
        let meta_beside = {
            let stem = decision_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            decision_path.with_file_name(format!("{stem}.metadata.csv"))
        };
        let loaded = if meta_beside.is_file() {
            load_data_source(
                &display_path(&decision_path),
                Some(&display_path(&meta_beside)),
                None,
            )?
        } else {
            load_data_source(
                &display_path(&decision_path),
                None,
                Some(&market_broker.timezone),
            )?
        };
        let market_broker =
            load_bound_broker(&display_path(&broker_path), loaded.metadata.as_ref())?;
        let dataset = if apply_promotion_split {
            development_partition(&loaded.dataset, validation_fraction, sealed_fraction)?
        } else {
            loaded.dataset
        };
        pack.push(PackSymbol {
            symbol: (*symbol).into(),
            dataset,
            broker: market_broker,
        });
    }
    Ok(pack)
}

fn find_timeframe_tsv(
    dir: &Path,
    symbol: &str,
    timeframe: &str,
) -> Result<Option<std::path::PathBuf>, String> {
    let mut matches = Vec::new();
    let entries = fs::read_dir(dir).map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        let upper = name.to_uppercase();
        if upper.contains(&format!(
            "_{}_{}_",
            symbol.to_uppercase(),
            timeframe.to_uppercase()
        )) && (name.ends_with(".tsv") || name.ends_with(".csv"))
        {
            matches.push(entry.path());
        }
    }
    matches.sort();
    Ok(matches.into_iter().next())
}

fn decision_timeframe_label(dataset: &BarDataset) -> Result<String, String> {
    let interval_ms = infer_median_interval_ms(&dataset.bars)
        .ok_or_else(|| "cannot infer the decision timeframe".to_owned())?;
    if interval_ms % 60_000 != 0 {
        return Err("decision timeframe must be a whole number of minutes".into());
    }
    let minutes = interval_ms / 60_000;
    Ok(if minutes % 60 == 0 {
        format!("H{}", minutes / 60)
    } else {
        format!("M{minutes}")
    })
}

/// Threads Rayon will actually use. `0` is the "global pool" sentinel.
fn effective_worker_threads(configured: usize) -> usize {
    if configured > 0 {
        return configured;
    }
    std::thread::available_parallelism()
        .map(|cores| cores.get())
        .unwrap_or(1)
}

fn recipe_fraction(artifact: &EvolveArtifact, key: &str, fallback: f64) -> f64 {
    artifact
        .manifest
        .recipe
        .config
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && (0.0..1.0).contains(value))
        .unwrap_or(fallback)
}

fn normalize_split_fractions(
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<(f64, f64), String> {
    if validation_fraction.is_finite()
        && sealed_fraction.is_finite()
        && validation_fraction + sealed_fraction >= 0.9
    {
        return Err(format!(
            "validation ({validation_fraction:.2}) + sealed ({sealed_fraction:.2}) leaves less than 10% for IS"
        ));
    }
    // Fractions are part of the persisted experiment contract.  Silently
    // clamping a saved value changes the intended IS/OOS split and can make a
    // result appear comparable when it was actually evaluated on a different
    // sample.  Reject invalid input explicitly instead.
    if !validation_fraction.is_finite()
        || !sealed_fraction.is_finite()
        || !(0.05..=0.4).contains(&validation_fraction)
        || !(0.05..=0.4).contains(&sealed_fraction)
    {
        return Err("validation and sealed fractions must each be between 5% and 40%".into());
    }
    let validation = validation_fraction;
    let sealed = sealed_fraction;
    if validation + sealed >= 0.9 {
        return Err(format!(
            "validation ({validation:.2}) + sealed ({sealed:.2}) leaves less than 10% for IS"
        ));
    }
    Ok((validation, sealed))
}

fn development_partition(
    dataset: &BarDataset,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<BarDataset, String> {
    let plan = quantforge_quality::DataSplitPlan::chronological(
        dataset,
        validation_fraction,
        sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    slice_partition(dataset, 0, plan.development.bar_count)
}

fn oos1_partition(
    dataset: &BarDataset,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<BarDataset, String> {
    let plan = quantforge_quality::DataSplitPlan::chronological(
        dataset,
        validation_fraction,
        sealed_fraction,
    )
    .map_err(|error| error.to_string())?;
    let start = plan.development.bar_count;
    let end = start + plan.validation.bar_count;
    slice_partition(dataset, start, end)
}

fn slice_partition(dataset: &BarDataset, start: usize, end: usize) -> Result<BarDataset, String> {
    if end <= start || end > dataset.bars.len() {
        return Err(format!(
            "invalid partition slice {start}..{end} for {} bars",
            dataset.bars.len()
        ));
    }
    let bars = dataset.bars[start..end].to_vec();
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

#[cfg(test)]
fn clip_dataset_to_window(dataset: &BarDataset, window: &BarDataset) -> Result<BarDataset, String> {
    let (Some(first), Some(last)) = (window.bars.first(), window.bars.last()) else {
        return Err("cannot clip M1: IS window is empty".into());
    };
    // Decision timestamps are bar *opens*. M1 must cover the full last bar
    // `[open, open+interval)` or Judge fails "M1 high aggregate …".
    let interval_ms = infer_median_interval_ms(&window.bars).unwrap_or(3_600_000);
    let start_ms = first.timestamp_ms;
    let end_exclusive_ms = last
        .timestamp_ms
        .checked_add(interval_ms)
        .ok_or_else(|| "IS window end overflow".to_owned())?;
    let bars: Vec<_> = dataset
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms >= start_ms && bar.timestamp_ms < end_exclusive_ms)
        .cloned()
        .collect();
    if bars.len() < 2 {
        return Err(format!(
            "M1 has fewer than 2 bars inside the IS window [{start_ms}..{end_exclusive_ms})"
        ));
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

const MAX_ENTRY_CONDITIONS: usize = UniversalGrammarConfig::MAX_ENTRY_CONDITIONS;
const MAX_EXIT_CONDITIONS: usize = 3;

fn validate_universal_grammar(grammar: &UniversalGrammarConfig) -> Result<(), String> {
    if grammar.minimum_entry_conditions < 2
        || grammar.maximum_entry_conditions > MAX_ENTRY_CONDITIONS
        || grammar.minimum_entry_conditions > grammar.maximum_entry_conditions
    {
        return Err(format!(
            "entry conditions must be an ordered range within 2..={MAX_ENTRY_CONDITIONS}"
        ));
    }
    if grammar.minimum_exit_conditions == 0
        || grammar.maximum_exit_conditions > MAX_EXIT_CONDITIONS
        || grammar.minimum_exit_conditions > grammar.maximum_exit_conditions
    {
        return Err(format!(
            "exit conditions must be an ordered range within 1..={MAX_EXIT_CONDITIONS}"
        ));
    }
    if grammar.minimum_shift == 0 || grammar.minimum_shift > grammar.maximum_shift {
        return Err("completed-bar shifts must be an ordered range starting at 1".into());
    }
    Ok(())
}

/// Entry-condition counts for the condition tester. Empty means the 2/3/4 default.
fn parse_entry_condition_counts(values: &[usize]) -> Result<Vec<usize>, String> {
    if values.is_empty() {
        return Ok(ConditionBakeoffConfig::default().entry_condition_counts);
    }
    let mut counts = Vec::with_capacity(values.len());
    for value in values {
        if !(2..=MAX_ENTRY_CONDITIONS).contains(value) {
            return Err(format!(
                "entry-condition counts must be within 2..={MAX_ENTRY_CONDITIONS}: got {value}"
            ));
        }
        if !counts.contains(value) {
            counts.push(*value);
        }
    }
    Ok(counts)
}

fn parse_run_mode(value: &str) -> Option<quantforge_discover::DiscoverRunMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fast_scout" | "fastscout" | "scout" => {
            Some(quantforge_discover::DiscoverRunMode::FastScout)
        }
        "full_harvest" | "fullharvest" | "harvest" => {
            Some(quantforge_discover::DiscoverRunMode::FullHarvest)
        }
        "quota_harvest" | "quotaharvest" | "quota" => {
            Some(quantforge_discover::DiscoverRunMode::QuotaHarvest)
        }
        _ => None,
    }
}

fn new_config(request: &DiscoverRequest) -> Result<DiscoverConfig, String> {
    let commission = request
        .commission_per_lot_round_turn
        .ok_or_else(|| "commission is required for a new databank".to_owned())?;
    Ok(DiscoverConfig {
        initial_candidates: request.initial_candidates.unwrap_or(500),
        batch_size: request.batch_size.unwrap_or(200),
        correlation_threshold: request.correlation_threshold.unwrap_or(0.85),
        novelty_weight: request.novelty_weight.unwrap_or(10.0),
        tournament_size: 4,
        structural_mutation_probability: 0.18,
        seed: request.seed.unwrap_or(42),
        universal_grammar: request.universal_grammar.clone().unwrap_or_default(),
        run_mode: request
            .run_mode
            .as_deref()
            .and_then(parse_run_mode)
            .unwrap_or(quantforge_discover::DiscoverRunMode::FullHarvest),
        early_stop_pot_elites: request.early_stop_pot_elites,
        target_databank_elites: request.target_databank_elites,
        trial_budget_warning: quantforge_discover::TRIAL_BUDGET_WARNING,
        gates: GateConfig {
            minimum_trades: request.minimum_trades.unwrap_or(10),
            maximum_drawdown_percent: request.maximum_drawdown_percent.unwrap_or(40.0),
            minimum_return_percent: request.minimum_return_percent.unwrap_or(0.0),
            minimum_profit_factor: request.minimum_profit_factor.unwrap_or(1.0),
            minimum_recovery_factor: request.minimum_return_drawdown.unwrap_or(0.0),
        },
        deposit_gates: GateConfig {
            minimum_trades: request.deposit_minimum_trades.unwrap_or(20),
            maximum_drawdown_percent: request.deposit_maximum_drawdown_percent.unwrap_or(30.0),
            minimum_return_percent: request.deposit_minimum_return_percent.unwrap_or(0.0),
            minimum_profit_factor: request.deposit_minimum_profit_factor.unwrap_or(1.0),
            minimum_recovery_factor: request.deposit_minimum_return_drawdown.unwrap_or(0.0),
        },
        precision: quantforge_discover::PrecisionGateConfig {
            minimum_return_retention: request.minimum_m1_return_retention.unwrap_or(0.80),
        },
        search_ranges: request.search_ranges.clone().unwrap_or_default(),
        oos1_expectancy_retention: request.oos1_expectancy_retention.unwrap_or(0.7),
        require_m1_precision: request.require_m1_precision.unwrap_or(true),
        simple_exits: request.simple_exits.unwrap_or(true),
        allow_break_even: request.allow_break_even.unwrap_or(false),
        allow_trailing_stops: request.allow_trailing_stops.unwrap_or(false),
        allow_partial_exits: request.allow_partial_exits.unwrap_or(false),
        allow_market_entries: request.allow_market_entries.unwrap_or(true),
        allow_stop_entries: request.allow_stop_entries.unwrap_or(false),
        allow_limit_entries: request.allow_limit_entries.unwrap_or(false),
        flatten_at_22: request.flatten_at_22.unwrap_or(false),
        end_of_day_hour: request.end_of_day_hour.unwrap_or(23),
        max_one_entry_per_day: request.max_one_entry_per_day.unwrap_or(true),
        mutate_after_elites: request.mutate_after_elites.unwrap_or(300),
        random_fill_fraction: request.random_fill_fraction.unwrap_or(0.4),
        worker_threads: request.worker_threads.unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|cores| cores.get().saturating_sub(1).max(1))
                .unwrap_or(1)
        }),
        require_m1_robustness: request.require_m1_robustness.unwrap_or(true),
        robustness_folds: request.robustness_folds.unwrap_or(3),
        robustness_monte_carlo_trials: request.robustness_monte_carlo_trials.unwrap_or(250),
        robustness_monte_carlo_block_length: request
            .robustness_monte_carlo_block_length
            .unwrap_or(5),
        robustness_monte_carlo_skip_trade_probability: request
            .robustness_monte_carlo_skip_trade_probability
            .unwrap_or(quantforge_discover::MONTE_CARLO_SKIP_TRADE_PROBABILITY),
        robustness_monte_carlo_p80_profit_retention: request
            .robustness_monte_carlo_p80_profit_retention
            .unwrap_or(quantforge_discover::MONTE_CARLO_P80_PROFIT_RETENTION),
        robustness_monte_carlo_max_drawdown_ratio: request
            .robustness_monte_carlo_max_drawdown_ratio
            .unwrap_or(quantforge_discover::MONTE_CARLO_MAX_DRAWDOWN_RATIO),
        robustness_neighborhood_samples: request.robustness_neighborhood_samples.unwrap_or(8),
        robustness_perturbation_fraction: request
            .robustness_perturbation_fraction
            .unwrap_or(quantforge_discover::PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION),
        minimum_neighborhood_survival_fraction: request
            .minimum_neighborhood_survival_fraction
            .unwrap_or(0.7),
        calendar_year_folds: request.calendar_year_folds.unwrap_or(false),
        minimum_deflated_trade_sharpe: request.minimum_deflated_trade_sharpe,
        multi_symbol_minimum_pass: request.multi_symbol_minimum_pass.unwrap_or(0),
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
            indicator_engine: quantforge_eval::IndicatorEngine::Mt5,
            entry_window: entry_window(
                request.entry_window_start_hour,
                request.entry_window_end_hour,
            ),
            // Search sets this per batch from the drawdown gate.
            abandon_above_drawdown_percent: None,
        },
    })
}

/// Falls back to the engine default so a request that omits the window keeps the
/// session every stored databank was built with.
pub(crate) fn entry_window(
    start_hour: Option<u32>,
    end_hour: Option<u32>,
) -> quantforge_eval::EntryWindow {
    let default = quantforge_eval::EntryWindow::default();
    quantforge_eval::EntryWindow::new(
        start_hour.unwrap_or(default.start_hour),
        end_hour.unwrap_or(default.end_hour),
    )
}

/// Run timer that measures working time, not elapsed time.
///
/// Throughput is reported as a per-hour rate, so counting the minutes a job sat
/// paused would make every resumed run look permanently slower than it is.
#[derive(Clone)]
struct ActiveClock {
    started: Instant,
    paused_millis: Arc<AtomicU64>,
}

impl ActiveClock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            paused_millis: Arc::new(AtomicU64::new(0)),
        }
    }

    fn add_paused(&self, span: Duration) {
        self.paused_millis
            .fetch_add(span.as_millis() as u64, Ordering::SeqCst);
    }

    fn active_hours(&self) -> f64 {
        let paused = self.paused_millis.load(Ordering::SeqCst) as f64 / 1_000.0;
        let active = (self.started.elapsed().as_secs_f64() - paused).max(1.0);
        active / 3600.0
    }
}

fn wait_if_paused(
    job: &Arc<RwLock<DiscoverJobView>>,
    paused: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
    clock: &ActiveClock,
) -> Result<(), String> {
    let pause_started = Instant::now();
    let mut was_paused = false;
    while paused.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
        was_paused = true;
        thread::sleep(Duration::from_millis(100));
    }
    if was_paused {
        clock.add_paused(pause_started.elapsed());
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
    run_until_stopped: bool,
    clock: &ActiveClock,
) -> Result<(), String> {
    let telemetry = &bank.telemetry;
    let accepted_total = telemetry.pot_accepted
        + telemetry.pot_replaced
        + telemetry.databank_accepted
        + telemetry.databank_replaced;
    let rejected_total = telemetry.rejected_gate
        + telemetry.rejected_deposit_gate
        + telemetry.rejected_clone
        + telemetry.rejected_correlated
        + telemetry.rejected_niche_not_improved
        + telemetry.rejected_precision
        + telemetry.rejected_ambiguous
        + telemetry.rejected_oos1
        + telemetry.rejected_m1_fidelity
        + telemetry.rejected_walk_forward
        + telemetry.rejected_monte_carlo
        + telemetry.rejected_param_neighborhood
        + telemetry.rejected_multi_symbol
        + telemetry.rejected_deflated_sharpe
        + telemetry.rejected_evaluation;
    let hours = clock.active_hours();
    let risk = quantforge_discover::FIXED_RISK_PER_TRADE;
    let best_is = bank
        .elites
        .iter()
        .chain(bank.accepted_pool.iter())
        .map(|elite| elite.is_expectancy / risk)
        .fold(None, |best: Option<f64>, value| {
            Some(best.map_or(value, |current| current.max(value)))
        });
    let best_oos1 = bank
        .elites
        .iter()
        .chain(bank.accepted_pool.iter())
        .filter_map(|elite| elite.oos1_expectancy.map(|value| value / risk))
        .fold(None, |best: Option<f64>, value| {
            Some(best.map_or(value, |current| current.max(value)))
        });
    let mut errors: Vec<EvaluationErrorCount> = telemetry
        .evaluation_errors
        .iter()
        .map(|(message, count)| EvaluationErrorCount {
            message: message.clone(),
            count: *count,
        })
        .collect();
    errors.sort_by_key(|error| std::cmp::Reverse(error.count));
    errors.truncate(5);

    let mutate_after = bank.config.mutate_after_elites;
    let pot_elites = bank.pot_size();
    let databank_elites = bank.coverage();
    let target_databank = bank.config.target_databank_elites;
    let breeding_active = pot_elites >= mutate_after && !bank.accepted_pool.is_empty();
    let phase = if let Some(target) = target_databank {
        format!(
            "Quota · databank {databank_elites}/{target} · pot {pot_elites} · gen {completed_now}"
        )
    } else if breeding_active {
        format!(
            "Breeding from pot · pot {pot_elites} · databank {databank_elites} · gen {completed_now}"
        )
    } else {
        format!(
            "Filling initial pot · {pot_elites}/{mutate_after} · databank {databank_elites} · gen {completed_now}"
        )
    };
    let pot_message = if let Some(target) = target_databank {
        format!(
            "Quota Harvest: databank {databank_elites}/{target} (stop at {target}). Pot {pot_elites} is only a breeding bag — not the goal. {}",
            funnel_summary(bank)
        )
    } else {
        format!(
            "Initial pot {pot_elites} (breed at {mutate_after}). Databank {databank_elites} only after breeding (OOS1→robustness→M1). {} · {}",
            if breeding_active {
                "Breeding unlocked — databank pipeline active".to_owned()
            } else {
                format!(
                    "{} more pot elites until breeding (no databank yet)",
                    mutate_after.saturating_sub(pot_elites)
                )
            },
            funnel_summary(bank)
        )
    };

    let mut view = job
        .write()
        .map_err(|_| "discover job state is unavailable".to_owned())?;
    if view.status != "paused" {
        view.status = "running";
        view.phase = phase;
        view.message = pot_message;
    }
    view.completed_generations = completed_now;
    view.requested_generations = requested;
    view.run_until_stopped = run_until_stopped;
    view.evaluation_count = bank.evaluation_count;
    view.accepted_total = accepted_total;
    view.pot_elites = pot_elites;
    view.pot_new_niches = telemetry.pot_accepted;
    view.databank_elites = databank_elites;
    view.target_databank_elites = target_databank;
    view.mutate_after_elites = mutate_after;
    view.breeding_active = breeding_active;
    // `0` means "Rayon global pool", which is every logical CPU. Report the
    // threads that actually run, not the sentinel.
    view.worker_threads = effective_worker_threads(bank.config.worker_threads);
    view.coverage = databank_elites;
    view.qd_score = bank.qd_score();
    view.rejected_gate = telemetry.rejected_gate;
    view.rejected_deposit_gate = telemetry.rejected_deposit_gate;
    view.rejected_precision = telemetry.rejected_precision;
    view.rejected_ambiguous = telemetry.rejected_ambiguous;
    view.rejected_oos1 = telemetry.rejected_oos1;
    view.rejected_m1_fidelity = telemetry.rejected_m1_fidelity;
    view.rejected_walk_forward = telemetry.rejected_walk_forward;
    view.rejected_monte_carlo = telemetry.rejected_monte_carlo;
    view.rejected_param_neighborhood = telemetry.rejected_param_neighborhood;
    view.rejected_multi_symbol = telemetry.rejected_multi_symbol;
    view.rejected_deflated_sharpe = telemetry.rejected_deflated_sharpe;
    view.rejected_clone = telemetry.rejected_clone;
    view.rejected_correlated = telemetry.rejected_correlated;
    view.rejected_niche_not_improved = telemetry.rejected_niche_not_improved;
    view.rejected_evaluation = telemetry.rejected_evaluation;
    view.rejected_total = rejected_total;
    view.evaluations_per_hour = bank.evaluation_count as f64 / hours;
    view.accepts_per_hour = accepted_total as f64 / hours;
    view.best_is_expectancy = best_is;
    view.best_oos1_expectancy = best_oos1;
    view.top_evaluation_errors = errors;
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
            decision_timeframe: Some(DecisionTimeframe::M15),
            metadata_path: Some(fixture("EURUSD_M15_sample.metadata.csv")),
            source_timezone: None,
            m1_data_path: fixture("EURUSD_M1_sample.tsv"),
            m1_metadata_path: Some(fixture("EURUSD_M1_sample.metadata.csv")),
            m1_source_timezone: None,
            broker_path: fixture("EURUSD_fixture_broker.json"),
            databank_path,
            generations: 1,
            run_until_stopped: Some(false),
            initial_candidates: Some(16),
            batch_size: Some(8),
            correlation_threshold: Some(0.88),
            novelty_weight: Some(10.0),
            seed: Some(42),
            minimum_trades: Some(0),
            maximum_drawdown_percent: Some(100.0),
            minimum_return_percent: Some(-100.0),
            minimum_profit_factor: Some(0.0),
            minimum_return_drawdown: Some(0.0),
            deposit_minimum_trades: Some(0),
            deposit_maximum_drawdown_percent: Some(100.0),
            deposit_minimum_return_percent: Some(-100.0),
            deposit_minimum_profit_factor: Some(0.0),
            deposit_minimum_return_drawdown: Some(0.0),
            minimum_m1_return_retention: Some(0.90),
            oos1_expectancy_retention: Some(0.7),
            require_m1_precision: Some(false),
            simple_exits: Some(true),
            allow_break_even: Some(false),
            allow_trailing_stops: Some(false),
            allow_partial_exits: Some(false),
            allow_market_entries: Some(true),
            allow_stop_entries: Some(false),
            allow_limit_entries: Some(false),
            flatten_at_22: Some(false),
            end_of_day_hour: Some(23),
            entry_window_start_hour: None,
            entry_window_end_hour: None,
            max_one_entry_per_day: Some(true),
            mutate_after_elites: Some(0),
            random_fill_fraction: Some(0.0),
            worker_threads: Some(1),
            require_m1_robustness: Some(false),
            robustness_folds: Some(3),
            robustness_monte_carlo_trials: Some(50),
            robustness_monte_carlo_block_length: Some(5),
            robustness_monte_carlo_skip_trade_probability: Some(0.10),
            robustness_monte_carlo_p80_profit_retention: Some(0.60),
            robustness_monte_carlo_max_drawdown_ratio: Some(1.75),
            robustness_neighborhood_samples: Some(2),
            robustness_perturbation_fraction: Some(0.20),
            minimum_neighborhood_survival_fraction: Some(0.0),
            calendar_year_folds: Some(false),
            minimum_deflated_trade_sharpe: None,
            multi_symbol_minimum_pass: Some(0),
            pack_data_dir: None,
            universal_grammar: None,
            run_mode: Some("full_harvest".into()),
            early_stop_pot_elites: None,
            target_databank_elites: None,
            search_ranges: None,
            commission_per_lot_round_turn: Some(0.0),
            slippage_points_per_side: Some(0.0),
            fallback_spread_points: None,
            max_spread_points: None,
            initial_balance: Some(100_000.0),
            promotion_split: Some(false),
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
    fn new_runs_need_no_family_and_reject_an_impossible_entry_range() {
        let directory = tempdir().expect("temp directory");
        let mut request = request(
            directory
                .path()
                .join("fresh-bank.json")
                .display()
                .to_string(),
        );
        validate_request(&request).expect("universal grammar defaults are enough");
        request.universal_grammar = Some(UniversalGrammarConfig {
            minimum_entry_conditions: 4,
            maximum_entry_conditions: 2,
            ..UniversalGrammarConfig::default()
        });
        let error = validate_request(&request).expect_err("inverted range must fail");
        assert!(error.contains("entry conditions"));
    }

    #[test]
    fn throughput_excludes_time_spent_paused() {
        let clock = ActiveClock::new();
        let before = clock.active_hours();
        clock.add_paused(Duration::from_secs(7_200));
        // A two-hour pause must not become two hours of "working" time, which
        // would otherwise halve the reported evaluations per hour.
        assert!(clock.active_hours() <= before);
        assert!(clock.active_hours() > 0.0);
    }

    #[test]
    fn condition_bakeoff_defaults_to_two_three_and_four() {
        assert_eq!(parse_entry_condition_counts(&[]), Ok(vec![2, 3, 4]));
        assert_eq!(parse_entry_condition_counts(&[3, 3, 2]), Ok(vec![3, 2]));
        assert!(parse_entry_condition_counts(&[5]).is_err());
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
    fn split_fractions_reject_an_undersized_is_window() {
        assert!(normalize_split_fractions(0.2, 0.2).is_ok());
        assert!(normalize_split_fractions(0.2, 0.1).is_ok());
        let error = normalize_split_fractions(0.5, 0.45).expect_err("IS must remain");
        assert!(error.contains("less than 10%"));
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
    fn worker_soft_completes_an_empty_archive_without_persisting() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("empty-bank.json");
        let mut request = request(path.display().to_string());
        request.minimum_trades = Some(usize::MAX);
        let job = Arc::new(RwLock::new(DiscoverJobView::idle()));
        run_discovery(
            request,
            &job,
            &Arc::new(AtomicBool::new(false)),
            &Arc::new(AtomicBool::new(false)),
        )
        .expect("empty archive should soft-complete");

        let view = job.read().expect("job state");
        assert_eq!(view.status, "completed");
        assert!(view.phase.contains("empty"));
        assert!(!path.exists());
        assert!(view.rejected_total > 0 || view.evaluation_count > 0);
    }

    #[test]
    fn clip_m1_covers_the_full_last_decision_bar() {
        use quantforge_data::Bar;
        let h1 = BarDataset {
            data_hash: bar_content_hash(&[]),
            source_rows: 2,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars: vec![
                Bar {
                    timestamp_ms: 0,
                    open: 1.0,
                    high: 1.1,
                    low: 0.9,
                    close: 1.05,
                    tick_volume: 60,
                    real_volume: 0,
                    spread_points: Some(1),
                },
                Bar {
                    timestamp_ms: 3_600_000,
                    open: 1.05,
                    high: 1.2,
                    low: 1.0,
                    close: 1.1,
                    tick_volume: 60,
                    real_volume: 0,
                    spread_points: Some(1),
                },
            ],
        };
        let m1_bars: Vec<_> = (0..120)
            .map(|minute| Bar {
                timestamp_ms: minute * 60_000,
                open: 1.0,
                high: if minute == 90 { 1.2 } else { 1.05 },
                low: 0.95,
                close: 1.0,
                tick_volume: 1,
                real_volume: 0,
                spread_points: Some(1),
            })
            .collect();
        let m1 = BarDataset {
            data_hash: bar_content_hash(&m1_bars),
            source_rows: m1_bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars: m1_bars,
        };
        let clipped = clip_dataset_to_window(&m1, &h1).expect("clip");
        assert_eq!(clipped.bars.len(), 120);
        assert!(
            clipped
                .bars
                .iter()
                .any(|bar| (bar.high - 1.2).abs() < 1e-12)
        );
    }

    #[test]
    fn decision_bars_are_built_from_m1_including_gappy_hours() {
        use quantforge_data::Bar;
        let decision = BarDataset {
            data_hash: bar_content_hash(&[]),
            source_rows: 1,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars: vec![Bar {
                timestamp_ms: 0,
                open: 1.0,
                high: 1.5, // exported H1 high that M1 never printed
                low: 0.9,
                close: 1.1,
                tick_volume: 1,
                real_volume: 0,
                spread_points: None,
            }],
        };
        let mut m1_bars = Vec::new();
        for minute in 0..60 {
            if minute == 30 {
                continue;
            }
            m1_bars.push(Bar {
                timestamp_ms: minute * 60_000,
                open: if minute == 0 { 1.0 } else { 1.05 },
                high: 1.2,
                low: 0.95,
                close: 1.1,
                tick_volume: 1,
                real_volume: 0,
                spread_points: None,
            });
        }
        let m1 = BarDataset {
            data_hash: bar_content_hash(&m1_bars),
            source_rows: m1_bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars: m1_bars,
        };
        let built = crate::data_lab::build_decision_from_m1(&m1, Some(&decision)).expect("build");
        assert_eq!(built.bars.len(), 1);
        assert!((built.bars[0].high - 1.2).abs() < 1e-9);
        assert_eq!(built.bars[0].tick_volume, 59);
    }
}
