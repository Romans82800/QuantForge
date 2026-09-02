use crate::data_lab::{
    apply_history_start_year, build_decision_from_m1, build_decision_from_m1_quotes, display_path,
    load_bound_broker, load_data_source, load_quote_sidecar, trim_market_history_to_year,
};
use crate::databank::{
    DesktopState, EvolveArtifact, install_live_databank_artifact, verify_artifact,
};
use quantforge_data::{
    BarDataset, bar_content_hash, build_timeframe_from_m1, build_timeframe_from_m1_with_quotes,
    infer_median_interval_ms,
};
use quantforge_discover::{
    ConditionBakeoffConfig, ConditionBakeoffReport, DEFAULT_FX_PACK, Databank, DiscoverConfig,
    DiscoverRunMode, GateConfig, PackSymbol, SearchRangeProfile, TimeframeAblationConfig,
    TimeframeAblationReport, TimeframeBakeoffConfig, TimeframeBakeoffReport, TimeframeGateConfig,
    TimeframeRollingWindow, UniversalGrammarConfig, new_databank,
    run_condition_bakeoff as evolve_condition_bakeoff,
    run_timeframe_ablation as evolve_timeframe_ablation,
    run_timeframe_bakeoff as evolve_timeframe_bakeoff,
    run_timeframe_rolling_ablation as evolve_timeframe_rolling_ablation,
};
use quantforge_eval::{CostModel, SameBarPolicy, ScoutConfig};
use quantforge_storage::{RunManifest, RunRecipe, write_json_new, write_json_replacing};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::State;

const RECOVERY_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(30 * 60);
const ROLLING_THROUGHPUT_WINDOW: Duration = Duration::from_secs(5 * 60);
const HOLDING_STALL_GENERATIONS: u64 = 25;
const HOLDING_STALL_MIN: usize = 40;

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
    H4,
}

impl DecisionTimeframe {
    const fn interval_ms(self) -> i64 {
        match self {
            Self::H1 => 3_600_000,
            Self::M15 => 900_000,
            Self::H4 => 14_400_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverRequest {
    mode: DiscoverMode,
    /// Explicit symbol chosen in the UI. It must match both metadata files and
    /// the broker profile; stale profile paths are never allowed to run.
    selected_symbol: Option<String>,
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
    minimum_development_expectancy_r: Option<f64>,
    oos1_expectancy_retention: Option<f64>,
    /// Downstream preference only — Discover never runs M1. When true, portfolio
    /// / export may insist on an explicit M1 fidelity pass after the run.
    require_m1_precision: Option<bool>,
    /// Legacy selected-TF compatibility profile. Explicit feature toggles below
    /// take precedence; they widen search without forcing M1 during Discover.
    simple_exits: Option<bool>,
    /// Constrained profile: only SL, TP and end-of-day close can exit.
    sl_tp_only_exits: Option<bool>,
    /// Add a fixed-pip SL/TP family beside the ATR/R protective family.
    allow_fixed_pip_stops: Option<bool>,
    /// Explicit research-only exit genes. They are off in the constrained recipe.
    allow_indicator_exit_rules: Option<bool>,
    allow_time_stops: Option<bool>,
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
    /// Dedicated Development robustness→M1 workers. `0` / omit = auto (2–4).
    promotion_worker_threads: Option<usize>,
    /// Max waiting + in-flight promotions before backpressure.
    promotion_queue_capacity: Option<usize>,
    /// Process resident-memory ceiling. The worker stops and writes its final
    /// artifact before the operating system is forced to kill it.
    max_memory_mb: Option<u64>,
    require_m1_robustness: Option<bool>,
    /// When true, Discover fills Holding instead of Databank. Default false:
    /// overnight Discover grows Databank after H1 permutation/folds/MC.
    build_to_holding: Option<bool>,
    robustness_folds: Option<usize>,
    robustness_monte_carlo_trials: Option<usize>,
    robustness_monte_carlo_block_length: Option<usize>,
    robustness_monte_carlo_skip_trade_probability: Option<f64>,
    robustness_monte_carlo_p80_profit_retention: Option<f64>,
    robustness_monte_carlo_max_drawdown_ratio: Option<f64>,
    robustness_neighborhood_samples: Option<usize>,
    /// Size of the ±% jitter applied to every numeric gene (default 0.20).
    robustness_perturbation_fraction: Option<f64>,
    /// Fraction of ±param neighbors that must survive (default 0.55).
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
    /// `fast_scout`, `full_harvest`, `quota_harvest`, or `high_performance_islands`.
    run_mode: Option<String>,
    general_island_count: Option<usize>,
    refinement_island_count: Option<usize>,
    exploration_island_count: Option<usize>,
    migration_interval: Option<u64>,
    migration_elites: Option<usize>,
    /// Early-stop when accepted pot reaches this size (Fast Scout / Quota).
    early_stop_pot_elites: Option<usize>,
    /// Early-stop when databank reaches this many elites (Quota Harvest default 100).
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
    /// Broker-local calendar year of the first bar kept (`2016` or `2020`).
    history_start_year: Option<u16>,
    /// After Discover checkpoints, shrink Holding and battery remaining names.
    #[serde(default)]
    factory_after_discover: Option<bool>,
    #[serde(default)]
    factory_queue_limit: Option<usize>,
    #[serde(default)]
    factory_target_databank: Option<usize>,
    #[serde(default)]
    factory_max_correlation: Option<f64>,
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
    /// Pre-battery M1 survivors awaiting on-demand battery.
    holding_elites: usize,
    databank_elites: usize,
    /// Changes on both databank admission and elite replacement. The UI uses
    /// this instead of the count alone so a same-size improved bank refreshes.
    live_databank_revision: u64,
    target_databank_elites: Option<usize>,
    mutate_after_elites: usize,
    breeding_active: bool,
    worker_threads: usize,
    promotion_worker_threads: usize,
    promotion_queue_capacity: usize,
    max_memory_mb: u64,
    resident_memory_mb: u64,
    promotion_queue_depth: u64,
    promotion_inflight: u64,
    promotions_enqueued: u64,
    promotions_completed: u64,
    promotion_backpressure_events: u64,
    promotions_per_hour: f64,
    coverage: usize,
    qd_score: f64,
    rejected_gate: u64,
    rejected_deposit_gate: u64,
    rejected_precision: u64,
    rejected_ambiguous: u64,
    rejected_oos1: u64,
    rejected_development_expectancy: u64,
    rejected_m1_fidelity: u64,
    rejected_walk_forward: u64,
    rejected_monte_carlo: u64,
    rejected_param_neighborhood: u64,
    rejected_multi_symbol: u64,
    rejected_deflated_sharpe: u64,
    rejected_clone: u64,
    rejected_correlated: u64,
    rejected_niche_not_improved: u64,
    #[serde(default)]
    rejected_family_not_improved: u64,
    rejected_evaluation: u64,
    rejected_total: u64,
    /// Five-minute moving throughput, suitable for detecting current slowdown.
    rolling_evaluations_per_hour: f64,
    /// Whole-run active-time average (paused time excluded).
    lifetime_evaluations_per_hour: f64,
    /// Backward-compatible alias for the lifetime average.
    evaluations_per_hour: f64,
    accepts_per_hour: f64,
    best_is_expectancy: Option<f64>,
    best_oos1_expectancy: Option<f64>,
    top_evaluation_errors: Vec<EvaluationErrorCount>,
    m1_bars_repaired: u64,
    latest_immutable_snapshot_path: Option<String>,
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
    live_artifact: Arc<RwLock<Option<EvolveArtifact>>>,
    paused: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}

/// One independently-bound market lane inside a shared Portfolio Discover
/// campaign. A lane has its own data, broker, split, seed and Databank; the
/// campaign only shares the bounded CPU budget and user-facing status.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioDiscoverAsset {
    symbol: String,
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    broker_path: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioDiscoverRequest {
    /// The shared search recipe. Asset paths are replaced by each selected lane.
    recipe: DiscoverRequest,
    assets: Vec<PortfolioDiscoverAsset>,
    /// Total Scout workers for the whole campaign, not per asset. Zero uses
    /// available logical CPUs and is divided fairly between lanes.
    global_worker_threads: usize,
    /// Full M1 imports are memory-heavy. Limit how many lanes load and evolve
    /// concurrently; queued lanes retain the same frozen recipe and start as
    /// an earlier lane finishes. Two is the safe desktop default.
    concurrent_lanes: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioDiscoverLaneView {
    symbol: String,
    output_path: String,
    status: String,
    phase: String,
    evaluation_count: u64,
    holding_elites: usize,
    databank_elites: usize,
    evaluations_per_hour: f64,
    worker_threads: usize,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioDiscoverJobView {
    job_id: Option<String>,
    status: String,
    phase: String,
    global_worker_threads: usize,
    concurrent_lanes: usize,
    active_lanes: usize,
    completed_lanes: usize,
    total_lanes: usize,
    total_evaluation_count: u64,
    total_holding_elites: usize,
    total_databank_elites: usize,
    started_at_ms: Option<u64>,
    stop_requested: bool,
    message: String,
    lanes: Vec<PortfolioDiscoverLaneView>,
}

#[derive(Clone)]
struct PortfolioLiveLane {
    output_path: String,
    artifact: Arc<RwLock<Option<EvolveArtifact>>>,
}

pub struct PortfolioDiscoverState {
    job: Arc<RwLock<PortfolioDiscoverJobView>>,
    stop: Arc<AtomicBool>,
    live_lanes: Arc<RwLock<BTreeMap<String, PortfolioLiveLane>>>,
}

impl Default for DiscoverState {
    fn default() -> Self {
        Self {
            job: Arc::new(RwLock::new(DiscoverJobView::idle())),
            live_artifact: Arc::new(RwLock::new(None)),
            paused: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for PortfolioDiscoverState {
    fn default() -> Self {
        Self {
            job: Arc::new(RwLock::new(PortfolioDiscoverJobView::idle())),
            stop: Arc::new(AtomicBool::new(false)),
            live_lanes: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

impl PortfolioDiscoverJobView {
    fn idle() -> Self {
        Self {
            job_id: None,
            status: "idle".into(),
            phase: "Ready".into(),
            global_worker_threads: 0,
            concurrent_lanes: 0,
            active_lanes: 0,
            completed_lanes: 0,
            total_lanes: 0,
            total_evaluation_count: 0,
            total_holding_elites: 0,
            total_databank_elites: 0,
            started_at_ms: None,
            stop_requested: false,
            message: "Choose 2–7 complete symbols and start a shared recipe.".into(),
            lanes: Vec::new(),
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
            holding_elites: 0,
            databank_elites: 0,
            live_databank_revision: 0,
            target_databank_elites: None,
            mutate_after_elites: 300,
            breeding_active: false,
            worker_threads: 0,
            promotion_worker_threads: 0,
            promotion_queue_capacity: 64,
            max_memory_mb: 8_192,
            resident_memory_mb: 0,
            promotion_queue_depth: 0,
            promotion_inflight: 0,
            promotions_enqueued: 0,
            promotions_completed: 0,
            promotion_backpressure_events: 0,
            promotions_per_hour: 0.0,
            coverage: 0,
            qd_score: 0.0,
            rejected_gate: 0,
            rejected_deposit_gate: 0,
            rejected_precision: 0,
            rejected_ambiguous: 0,
            rejected_oos1: 0,
            rejected_development_expectancy: 0,
            rejected_m1_fidelity: 0,
            rejected_walk_forward: 0,
            rejected_monte_carlo: 0,
            rejected_param_neighborhood: 0,
            rejected_multi_symbol: 0,
            rejected_deflated_sharpe: 0,
            rejected_clone: 0,
            rejected_correlated: 0,
            rejected_niche_not_improved: 0,
            rejected_family_not_improved: 0,
            rejected_evaluation: 0,
            rejected_total: 0,
            rolling_evaluations_per_hour: 0.0,
            lifetime_evaluations_per_hour: 0.0,
            evaluations_per_hour: 0.0,
            accepts_per_hour: 0.0,
            best_is_expectancy: None,
            best_oos1_expectancy: None,
            top_evaluation_errors: Vec::new(),
            m1_bars_repaired: 0,
            latest_immutable_snapshot_path: None,
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
    /// Broker-local calendar year of the first bar kept (`2016` or `2020`).
    #[serde(default)]
    history_start_year: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeBakeoffRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    broker_path: String,
    draws_per_cell: usize,
    seed: u64,
    minimum_trades: usize,
    minimum_return_percent: f64,
    minimum_profit_factor: f64,
    maximum_drawdown_percent: f64,
    oos1_retention: f64,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    fallback_spread_points: Option<f64>,
    validation_fraction: f64,
    sealed_fraction: f64,
    minimum_entry_conditions: usize,
    maximum_entry_conditions: usize,
    minimum_exit_conditions: usize,
    maximum_exit_conditions: usize,
    #[serde(default)]
    history_start_year: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeframeAblationRequest {
    #[serde(flatten)]
    base: TimeframeBakeoffRequest,
    #[serde(default)]
    h1_gates: Option<TimeframeGateConfig>,
    #[serde(default)]
    h4_gates: Option<TimeframeGateConfig>,
}

struct TimeframeComparisonData {
    h1_is: BarDataset,
    h1_oos1: BarDataset,
    h1_sealed: BarDataset,
    h4_is: BarDataset,
    h4_oos1: BarDataset,
    h4_sealed: BarDataset,
    broker: quantforge_broker::SymbolSpecification,
}

fn load_timeframe_comparison_data(
    request: &TimeframeBakeoffRequest,
) -> Result<TimeframeComparisonData, String> {
    if request.minimum_entry_conditions > request.maximum_entry_conditions
        || request.minimum_exit_conditions > request.maximum_exit_conditions
    {
        return Err(
            "timeframe bakeoff minimum grammar bounds must not exceed maximum bounds".into(),
        );
    }
    if request.validation_fraction <= 0.0 {
        return Err("timeframe bakeoff requires a non-zero OOS1 reserve".into());
    }
    let mut loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let mut m1_loaded = load_data_source(
        &request.m1_data_path,
        request.m1_metadata_path.as_deref(),
        request.m1_source_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let history_start_year = request
        .history_start_year
        .unwrap_or(quantforge_data::DEFAULT_HISTORY_START_YEAR);
    let quote_path = infer_quote_path(&request.m1_data_path);
    let mut quote_dataset = quote_path
        .as_ref()
        .map(|path| load_quote_sidecar(path, m1_loaded.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut loaded.dataset,
        &mut m1_loaded.dataset,
        quote_dataset.as_mut(),
        history_start_year,
    )?;
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    load_bound_broker(&request.broker_path, m1_loaded.metadata.as_ref())?;
    let (validation_fraction, sealed_fraction) =
        normalize_split_fractions(request.validation_fraction, request.sealed_fraction)?;
    let h1_decision = match quote_dataset.as_ref() {
        Some(quotes) => build_decision_from_m1_quotes(
            &m1_loaded.dataset,
            Some(&loaded.dataset),
            quotes,
            broker.point,
        )?,
        None => build_decision_from_m1(&m1_loaded.dataset, Some(&loaded.dataset))?,
    };
    let h4_decision = match quote_dataset.as_ref() {
        Some(quotes) => build_timeframe_from_m1_with_quotes(
            &m1_loaded.dataset,
            quotes,
            broker.point,
            DecisionTimeframe::H4.interval_ms(),
            None,
        )
        .map_err(|error| error.to_string())?,
        None => build_timeframe_from_m1(
            &m1_loaded.dataset,
            DecisionTimeframe::H4.interval_ms(),
            None,
        )
        .map_err(|error| error.to_string())?,
    };
    Ok(TimeframeComparisonData {
        h1_is: development_partition(&h1_decision, validation_fraction, sealed_fraction)?,
        h1_oos1: oos1_partition(&h1_decision, validation_fraction, sealed_fraction)?,
        h1_sealed: sealed_partition(&h1_decision, validation_fraction, sealed_fraction)?,
        h4_is: development_partition(&h4_decision, validation_fraction, sealed_fraction)?,
        h4_oos1: oos1_partition(&h4_decision, validation_fraction, sealed_fraction)?,
        h4_sealed: sealed_partition(&h4_decision, validation_fraction, sealed_fraction)?,
        broker,
    })
}

fn timeframe_scout(request: &TimeframeBakeoffRequest) -> ScoutConfig {
    let mut scout = ScoutConfig::default();
    scout.costs = CostModel {
        commission_per_lot_round_turn: request.commission_per_lot_round_turn,
        adverse_slippage_points_per_side: request.slippage_points_per_side,
        fallback_spread_points: request.fallback_spread_points,
        ..scout.costs
    };
    scout
}

fn timeframe_gate_from_request(request: &TimeframeBakeoffRequest) -> TimeframeGateConfig {
    TimeframeGateConfig {
        minimum_trades: request.minimum_trades,
        minimum_return_percent: request.minimum_return_percent,
        minimum_profit_factor: request.minimum_profit_factor,
        maximum_drawdown_percent: request.maximum_drawdown_percent,
        oos1_retention: request.oos1_retention,
    }
}

#[tauri::command]
pub fn run_condition_bakeoff(
    request: ConditionBakeoffRequest,
) -> Result<ConditionBakeoffReport, String> {
    let entry_condition_counts = parse_entry_condition_counts(&request.entry_condition_counts)?;
    let mut loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let mut m1_loaded = load_data_source(
        &request.m1_data_path,
        request.m1_metadata_path.as_deref(),
        request.m1_source_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let history_start_year = request
        .history_start_year
        .unwrap_or(quantforge_data::DEFAULT_HISTORY_START_YEAR);
    trim_market_history_to_year(
        &mut loaded.dataset,
        &mut m1_loaded.dataset,
        None,
        history_start_year,
    )?;
    let broker = load_bound_broker(&request.broker_path, loaded.metadata.as_ref())?;
    load_bound_broker(&request.broker_path, m1_loaded.metadata.as_ref())?;
    let validation_fraction = request.validation_fraction.clamp(0.0, 0.5);
    let sealed_fraction = request.sealed_fraction.clamp(0.05, 0.4);
    if validation_fraction + sealed_fraction >= 0.9 {
        return Err(format!(
            "OOS1 ({validation_fraction:.2}) + sealed ({sealed_fraction:.2}) leaves less than 10% for Development"
        ));
    }
    let search_h1 = development_partition(&loaded.dataset, validation_fraction, sealed_fraction)?;
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
        None,
        &m1_is,
        &broker,
        &[],
        &broker.symbol,
        config,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn run_timeframe_bakeoff(
    request: TimeframeBakeoffRequest,
) -> Result<TimeframeBakeoffReport, String> {
    let data = load_timeframe_comparison_data(&request)?;
    let config = TimeframeBakeoffConfig {
        seed: request.seed,
        draws_per_cell: request.draws_per_cell.max(1),
        entry_condition_counts: (request.minimum_entry_conditions
            ..=request.maximum_entry_conditions)
            .collect(),
        exit_condition_counts: (request.minimum_exit_conditions..=request.maximum_exit_conditions)
            .collect(),
        scout: timeframe_scout(&request),
        minimum_trades: request.minimum_trades,
        minimum_return_percent: request.minimum_return_percent,
        minimum_profit_factor: request.minimum_profit_factor,
        maximum_drawdown_percent: request.maximum_drawdown_percent,
        oos1_retention: request.oos1_retention,
    };
    evolve_timeframe_bakeoff(
        &data.h1_is,
        &data.h1_oos1,
        &data.h4_is,
        &data.h4_oos1,
        &data.broker,
        config,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn run_timeframe_ablation(
    request: TimeframeAblationRequest,
) -> Result<TimeframeAblationReport, String> {
    let data = load_timeframe_comparison_data(&request.base)?;
    let shared_gates = timeframe_gate_from_request(&request.base);
    let config = TimeframeAblationConfig {
        seed: request.base.seed,
        draws_per_cell: request.base.draws_per_cell.max(1),
        entry_condition_counts: (request.base.minimum_entry_conditions
            ..=request.base.maximum_entry_conditions)
            .collect(),
        exit_condition_counts: (request.base.minimum_exit_conditions
            ..=request.base.maximum_exit_conditions)
            .collect(),
        scout: timeframe_scout(&request.base),
        h1_gates: request
            .h1_gates
            .clone()
            .unwrap_or_else(|| shared_gates.clone()),
        h4_gates: request
            .h4_gates
            .clone()
            .unwrap_or_else(|| shared_gates.clone()),
        shared_gates,
    };
    evolve_timeframe_ablation(
        &data.h1_is,
        &data.h1_oos1,
        &data.h4_is,
        &data.h4_oos1,
        &data.broker,
        config,
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn start_discover(
    mut request: DiscoverRequest,
    state: State<'_, DiscoverState>,
    battery: State<'_, crate::holding_battery::BatteryJobState>,
    portfolio: State<'_, PortfolioDiscoverState>,
) -> Result<DiscoverJobView, String> {
    if request.mode == DiscoverMode::New && request.databank_path.trim().is_empty() {
        request.databank_path = automatic_databank_path(&request)?;
    }
    validate_request(&request)?;
    if request.run_mode.as_deref().and_then(parse_run_mode)
        == Some(quantforge_discover::DiscoverRunMode::QuotaHarvest)
    {
        request.target_databank_elites = Some(
            request
                .target_databank_elites
                .unwrap_or(400)
                .clamp(40, 10_000),
        );
    }
    {
        let current = state
            .job
            .read()
            .map_err(|_| "discover job state is unavailable")?;
        if matches!(current.status, "running" | "paused") {
            return Err("a discovery job is already active".into());
        }
    }
    if portfolio
        .job
        .read()
        .map_err(|_| "multi-asset campaign state is unavailable")?
        .status
        == "running"
    {
        return Err(
            "a multi-asset Discover campaign is active; stop it before starting a single-asset job"
                .into(),
        );
    }

    state.paused.store(false, Ordering::SeqCst);
    state.stop.store(false, Ordering::SeqCst);
    *state
        .live_artifact
        .write()
        .map_err(|_| "discover live databank state is unavailable")? = None;
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
        holding_elites: 0,
        databank_elites: 0,
        live_databank_revision: 0,
        target_databank_elites: request.target_databank_elites,
        mutate_after_elites: request.mutate_after_elites.unwrap_or(300),
        breeding_active: false,
        worker_threads: request.worker_threads.unwrap_or(0),
        promotion_worker_threads: request.promotion_worker_threads.unwrap_or(0),
        promotion_queue_capacity: request.promotion_queue_capacity.unwrap_or(64),
        max_memory_mb: request.max_memory_mb.unwrap_or(8_192),
        resident_memory_mb: resident_memory_mb().unwrap_or(0),
        promotion_queue_depth: 0,
        promotion_inflight: 0,
        promotions_enqueued: 0,
        promotions_completed: 0,
        promotion_backpressure_events: 0,
        promotions_per_hour: 0.0,
        coverage: 0,
        qd_score: 0.0,
        rejected_gate: 0,
        rejected_deposit_gate: 0,
        rejected_precision: 0,
        rejected_ambiguous: 0,
        rejected_oos1: 0,
        rejected_development_expectancy: 0,
        rejected_m1_fidelity: 0,
        rejected_walk_forward: 0,
        rejected_monte_carlo: 0,
        rejected_param_neighborhood: 0,
        rejected_multi_symbol: 0,
        rejected_deflated_sharpe: 0,
        rejected_clone: 0,
        rejected_correlated: 0,
        rejected_niche_not_improved: 0,
        rejected_family_not_improved: 0,
        rejected_evaluation: 0,
        rejected_total: 0,
        rolling_evaluations_per_hour: 0.0,
        lifetime_evaluations_per_hour: 0.0,
        evaluations_per_hour: 0.0,
        accepts_per_hour: 0.0,
        best_is_expectancy: None,
        best_oos1_expectancy: None,
        top_evaluation_errors: Vec::new(),
        m1_bars_repaired: 0,
        latest_immutable_snapshot_path: None,
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
    let live_artifact = Arc::clone(&state.live_artifact);
    let battery_state = crate::holding_battery::BatteryJobState {
        job: Arc::clone(&battery.job),
        stop: Arc::clone(&battery.stop),
    };
    tauri::async_runtime::spawn_blocking(move || {
        match run_discovery(request.clone(), &job, &live_artifact, &paused, &stop) {
            Err(error) => {
                if let Ok(mut view) = job.write() {
                    view.status = "failed";
                    view.phase = "Stopped with an error".into();
                    view.message = error;
                }
            }
            Ok(()) => {
                if request.factory_after_discover.unwrap_or(false) {
                    let factory_queue = request.factory_queue_limit.filter(|&n| n > 0);
                    // An open-ended Discover is allowed to hand off only
                    // after the user explicitly stops it. When it does, the
                    // factory must examine the whole frozen Holding cohort,
                    // not silently stop after an old target such as `1`.
                    let factory_target = automatic_factory_target(
                        request.run_until_stopped.unwrap_or(true),
                        request.factory_target_databank,
                    );
                    let factory = crate::databank::HoldingBatteryRequest {
                        fingerprints: Vec::new(),
                        ranked: true,
                        // Correlation shrinking is a separate, explicit
                        // Holding action. An automatic full battery must not
                        // silently reduce the cohort before testing it.
                        shrink_first: false,
                        max_correlation: None,
                        queue_limit: factory_queue,
                        target_databank: factory_target,
                        audit_and_graduate: false,
                    };
                    // Discover has already completed by this point, but the
                    // factory can promote a Holding candidate seconds later.
                    // Keep the finished dashboard's live snapshot in lockstep
                    // with each durable factory checkpoint; otherwise it can
                    // misleadingly keep showing zero Databank strategies.
                    let factory_live_artifact = Arc::clone(&live_artifact);
                    let factory_discover_job = Arc::clone(&job);
                    let factory_checkpoint: crate::holding_battery::FactoryCheckpoint =
                        Arc::new(move |artifact| {
                            if let Ok(mut live) = factory_live_artifact.write() {
                                *live = Some(artifact.clone());
                            }
                            if let Ok(mut view) = factory_discover_job.write() {
                                view.holding_elites = artifact.databank.holding.len();
                                view.databank_elites = artifact.databank.elites.len();
                                view.coverage = artifact.databank.coverage();
                                view.qd_score = artifact.databank.qd_score();
                                view.live_databank_revision =
                                    view.live_databank_revision.saturating_add(1);
                                view.message = format!(
                                    "Factory checkpoint: {} Holding · {} Databank. {}",
                                    view.holding_elites, view.databank_elites, view.message
                                );
                            }
                        });
                    match crate::holding_battery::spawn_factory_from_archive(
                        request.databank_path.clone(),
                        factory,
                        &battery_state,
                        Some(factory_checkpoint),
                    ) {
                        Ok(_) => {
                            if let Ok(mut view) = job.write() {
                                let factory_note = match factory_target {
                                    Some(n) => format!(
                                        "Factory started: battery the Holding queue until {n} Databank names."
                                    ),
                                    None => {
                                        "Factory started: battery everyone in the current Holding queue."
                                            .to_owned()
                                    }
                                };
                                view.message = format!("{}. {factory_note}", view.message);
                            }
                        }
                        Err(error) => {
                            if let Ok(mut view) = job.write() {
                                view.message =
                                    format!("{}. Factory did not start: {error}", view.message);
                            }
                        }
                    }
                }
            }
        }
    });
    Ok(started)
}

/// Start independent Discover lanes under one shared CPU budget. Assets never
/// share bars, candidates, Databanks, OOS1 results or sealed OOS2 data.
#[tauri::command]
pub fn start_portfolio_discover(
    request: PortfolioDiscoverRequest,
    state: State<'_, PortfolioDiscoverState>,
    single_discover: State<'_, DiscoverState>,
) -> Result<PortfolioDiscoverJobView, String> {
    if request.assets.len() < 2 || request.assets.len() > 7 {
        return Err("Portfolio Discover requires between 2 and 7 assets".into());
    }
    if request.recipe.mode != DiscoverMode::New {
        return Err(
            "Portfolio Discover starts new isolated Databanks; continue each lane separately"
                .into(),
        );
    }
    if single_discover
        .job
        .read()
        .map_err(|_| "discover job state is unavailable")?
        .status
        == "running"
    {
        return Err(
            "stop the single-asset Discover job before starting a portfolio campaign".into(),
        );
    }
    {
        let current = state
            .job
            .read()
            .map_err(|_| "portfolio campaign state is unavailable")?;
        if current.status == "running" {
            return Err("a Portfolio Discover campaign is already active".into());
        }
    }

    let mut seen = std::collections::BTreeSet::new();
    for asset in &request.assets {
        let symbol = asset.symbol.trim().to_ascii_uppercase();
        if symbol.is_empty() || !seen.insert(symbol) {
            return Err("select each Portfolio Discover symbol once".into());
        }
    }

    let available = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let lane_count = request.assets.len();
    let global_workers = if request.global_worker_threads == 0 {
        available.max(lane_count)
    } else {
        request
            .global_worker_threads
            .max(lane_count)
            .min(available.max(lane_count))
    };
    // Starting seven full H1/M1 loads at once can exhaust file handles and
    // memory long before Scout CPU becomes the constraint. Keep the CPU budget
    // fully used, but stage the high-memory lanes. Users can raise this when
    // they have ample RAM; two is deliberately the default.
    let concurrent_lanes = request.concurrent_lanes.clamp(1, lane_count);
    let lane_workers = (global_workers / concurrent_lanes).max(1);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis() as u64;

    let mut lanes = Vec::with_capacity(lane_count);
    for (index, asset) in request.assets.iter().enumerate() {
        let mut lane_request = request.recipe.clone();
        lane_request.mode = DiscoverMode::New;
        lane_request.selected_symbol = Some(asset.symbol.trim().to_ascii_uppercase());
        lane_request.data_path = asset.data_path.clone();
        lane_request.metadata_path = asset.metadata_path.clone();
        lane_request.source_timezone = asset.source_timezone.clone();
        lane_request.m1_data_path = asset.m1_data_path.clone();
        lane_request.m1_metadata_path = asset.m1_metadata_path.clone();
        lane_request.m1_source_timezone = asset.m1_source_timezone.clone();
        lane_request.broker_path = asset.broker_path.clone();
        lane_request.databank_path = automatic_databank_path(&lane_request)?;
        lane_request.seed = Some(lane_request.seed.unwrap_or(42).wrapping_add(index as u64));
        lane_request.worker_threads = Some(lane_workers);
        // Holding work is deliberately not allowed to monopolise all cores on
        // each lane. The user can later run batteries per asset.
        lane_request.promotion_worker_threads = Some(1);
        lane_request.pack_data_dir = None;
        lane_request.multi_symbol_minimum_pass = Some(0);
        lane_request.factory_after_discover = Some(false);
        validate_request(&lane_request)?;
        lanes.push((
            asset.symbol.trim().to_ascii_uppercase(),
            lane_request,
            lane_workers,
            Arc::new(RwLock::new(None)),
        ));
    }

    state.stop.store(false, Ordering::SeqCst);
    {
        let mut live_lanes = state
            .live_lanes
            .write()
            .map_err(|_| "portfolio live Databank state is unavailable")?;
        live_lanes.clear();
        for (symbol, request, _, artifact) in &lanes {
            live_lanes.insert(
                symbol.clone(),
                PortfolioLiveLane {
                    output_path: request.databank_path.clone(),
                    artifact: Arc::clone(artifact),
                },
            );
        }
    }
    let initial_lanes = lanes
        .iter()
        .map(|(symbol, request, workers, _)| {
            portfolio_lane_view(symbol, &portfolio_lane_job(request, *workers))
        })
        .collect::<Vec<_>>();
    let started = PortfolioDiscoverJobView {
        job_id: Some(format!("portfolio-{now_ms}")),
        status: "running".into(),
        phase: "Preparing isolated asset lanes".into(),
        global_worker_threads: global_workers,
        concurrent_lanes,
        active_lanes: 0,
        completed_lanes: 0,
        total_lanes: lane_count,
        total_evaluation_count: 0,
        total_holding_elites: 0,
        total_databank_elites: 0,
        started_at_ms: Some(now_ms),
        stop_requested: false,
        message: format!(
            "{lane_count} isolated lanes share {global_workers} Scout worker{}; up to {concurrent_lanes} high-memory lane{} run at once. Each asset keeps its own Development, OOS1 and sealed OOS2 partitions.",
            if global_workers == 1 { "" } else { "s" },
            if concurrent_lanes == 1 { "" } else { "s" },
        ),
        lanes: initial_lanes,
    };
    *state
        .job
        .write()
        .map_err(|_| "portfolio campaign state is unavailable")? = started.clone();

    let campaign_job = Arc::clone(&state.job);
    let campaign_stop = Arc::clone(&state.stop);
    tauri::async_runtime::spawn_blocking(move || {
        let mut pending = lanes
            .into_iter()
            .map(|(symbol, request, workers, artifact)| {
                let job = Arc::new(RwLock::new(portfolio_lane_job(&request, workers)));
                (symbol, request, workers, job, artifact)
            })
            .collect::<VecDeque<_>>();
        let all_lanes = pending
            .iter()
            .map(|(symbol, _, _, job, _)| (symbol.clone(), Arc::clone(job)))
            .collect::<Vec<_>>();
        let mut running: Vec<(String, thread::JoinHandle<()>)> = Vec::new();

        loop {
            if campaign_stop.load(Ordering::SeqCst) && !pending.is_empty() {
                for (_, _, _, job, _) in pending.drain(..) {
                    if let Ok(mut view) = job.write() {
                        view.status = "stopped";
                        view.phase = "Not started".into();
                        view.message =
                            "The campaign was stopped before this queued lane began.".into();
                    }
                }
            }

            while !campaign_stop.load(Ordering::SeqCst) && running.len() < concurrent_lanes {
                let Some((symbol, lane_request, _, lane_job, lane_live_artifact)) =
                    pending.pop_front()
                else {
                    break;
                };
                if let Ok(mut view) = lane_job.write() {
                    view.status = "running";
                    view.phase = "Loading and validating inputs".into();
                    view.message = "This isolated lane is starting.".into();
                }
                let lane_paused = Arc::new(AtomicBool::new(false));
                let lane_stop = Arc::clone(&campaign_stop);
                let lane_job_for_thread = Arc::clone(&lane_job);
                let handle = thread::spawn(move || {
                    if let Err(error) = run_discovery(
                        lane_request,
                        &lane_job_for_thread,
                        &lane_live_artifact,
                        &lane_paused,
                        &lane_stop,
                    ) {
                        if let Ok(mut view) = lane_job_for_thread.write() {
                            view.status = "failed";
                            view.phase = "Stopped with an error".into();
                            view.message = error;
                        }
                    } else if let Ok(mut view) = lane_job_for_thread.write() {
                        if view.status != "failed" {
                            view.status = if lane_stop.load(Ordering::SeqCst) {
                                "stopped"
                            } else {
                                "completed"
                            };
                            view.phase = "Discover finished".into();
                        }
                    }
                });
                running.push((symbol, handle));
            }

            let lanes = all_lanes
                .iter()
                .filter_map(|(symbol, job)| {
                    job.read()
                        .ok()
                        .map(|view| portfolio_lane_view(symbol, &view))
                })
                .collect::<Vec<_>>();
            let active_lanes = lanes.iter().filter(|lane| lane.status == "running").count();
            let completed_lanes = lanes
                .iter()
                .filter(|lane| matches!(lane.status.as_str(), "completed" | "stopped" | "failed"))
                .count();
            let any_failed = lanes.iter().any(|lane| lane.status == "failed");
            let all_finished = pending.is_empty() && running.is_empty();
            if let Ok(mut view) = campaign_job.write() {
                view.active_lanes = active_lanes;
                view.completed_lanes = completed_lanes;
                view.total_evaluation_count = lanes.iter().map(|lane| lane.evaluation_count).sum();
                view.total_holding_elites = lanes.iter().map(|lane| lane.holding_elites).sum();
                view.total_databank_elites = lanes.iter().map(|lane| lane.databank_elites).sum();
                view.stop_requested = campaign_stop.load(Ordering::SeqCst);
                view.lanes = lanes;
                view.phase = if all_finished {
                    if any_failed {
                        "Campaign finished with lane errors"
                    } else if view.stop_requested {
                        "Campaign stopped"
                    } else {
                        "Campaign complete"
                    }
                    .into()
                } else if view.stop_requested {
                    "Stopping after each lane's current generation".into()
                } else {
                    format!(
                        "{active_lanes}/{} lanes active · {} queued",
                        view.total_lanes,
                        pending.len()
                    )
                };
                if all_finished {
                    view.status = if any_failed {
                        "failed"
                    } else if view.stop_requested {
                        "stopped"
                    } else {
                        "completed"
                    }
                    .into();
                    view.message = format!(
                        "{} lanes finished: {} Holding, {} Databank, {} candidates evaluated.",
                        view.total_lanes,
                        view.total_holding_elites,
                        view.total_databank_elites,
                        view.total_evaluation_count,
                    );
                }
            }
            if all_finished {
                break;
            }
            let mut finished = Vec::new();
            for (index, (_, handle)) in running.iter().enumerate() {
                if handle.is_finished() {
                    finished.push(index);
                }
            }
            for index in finished.into_iter().rev() {
                let (_, handle) = running.remove(index);
                let _ = handle.join();
            }
            thread::sleep(Duration::from_millis(350));
        }
        for (_, handle) in running {
            let _ = handle.join();
        }
    });
    Ok(started)
}

#[tauri::command]
pub fn get_portfolio_discover_job(
    state: State<'_, PortfolioDiscoverState>,
) -> Result<PortfolioDiscoverJobView, String> {
    state
        .job
        .read()
        .map(|view| view.clone())
        .map_err(|_| "portfolio campaign state is unavailable".into())
}

/// Opens the selected campaign lane's in-memory Holding/Databank snapshot.
/// Unlike the on-disk final archive, this updates while Discover is running.
#[tauri::command]
pub fn get_portfolio_live_databank(
    symbol: String,
    state: State<'_, PortfolioDiscoverState>,
    databank_state: State<'_, DesktopState>,
) -> Result<crate::databank::DatabankWorkspace, String> {
    let symbol = symbol.trim().to_ascii_uppercase();
    let lane = state
        .live_lanes
        .read()
        .map_err(|_| "portfolio live Databank state is unavailable")?
        .get(&symbol)
        .cloned()
        .ok_or_else(|| format!("{symbol} is not part of the current multi-asset campaign"))?;
    let artifact = lane
        .artifact
        .read()
        .map_err(|_| "portfolio live lane state is unavailable")?
        .clone()
        .ok_or_else(|| format!("{symbol} has no Holding or Databank strategies yet"))?;
    install_live_databank_artifact(artifact, PathBuf::from(lane.output_path), &databank_state)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn stop_portfolio_discover(
    state: State<'_, PortfolioDiscoverState>,
) -> Result<PortfolioDiscoverJobView, String> {
    let mut view = state
        .job
        .write()
        .map_err(|_| "portfolio campaign state is unavailable")?;
    if view.status != "running" {
        return Err("no active Portfolio Discover campaign can be stopped".into());
    }
    state.stop.store(true, Ordering::SeqCst);
    view.stop_requested = true;
    view.phase = "Stopping after each lane's current generation".into();
    view.message = "Every lane will checkpoint its own immutable Databank before it exits.".into();
    Ok(view.clone())
}

fn portfolio_lane_job(request: &DiscoverRequest, worker_threads: usize) -> DiscoverJobView {
    let mut job = DiscoverJobView::idle();
    job.job_id = Some(format!(
        "portfolio-{}",
        request.selected_symbol.as_deref().unwrap_or("lane")
    ));
    job.status = "queued";
    job.mode = Some(DiscoverModeView::New);
    job.phase = "Waiting for campaign worker".into();
    job.output_path = Some(request.databank_path.clone());
    job.requested_generations = request.generations;
    job.run_until_stopped = request.run_until_stopped.unwrap_or(true);
    job.worker_threads = worker_threads;
    job.promotion_worker_threads = 1;
    job.message = "Isolated Development search is starting.".into();
    job
}

fn portfolio_lane_view(symbol: &str, job: &DiscoverJobView) -> PortfolioDiscoverLaneView {
    PortfolioDiscoverLaneView {
        symbol: symbol.into(),
        // A stopped or completed open-ended Discover writes an immutable
        // snapshot beside the working archive. The campaign UI must reopen
        // that snapshot, not the transient working path which may not exist.
        output_path: job
            .latest_immutable_snapshot_path
            .clone()
            .or_else(|| job.output_path.clone())
            .unwrap_or_default(),
        status: job.status.into(),
        phase: job.phase.clone(),
        evaluation_count: job.evaluation_count,
        holding_elites: job.holding_elites,
        databank_elites: job.databank_elites,
        evaluations_per_hour: job.rolling_evaluations_per_hour,
        worker_threads: job.worker_threads,
        message: job.message.clone(),
    }
}

#[tauri::command]
pub fn get_discover_live_databank(
    state: State<'_, DiscoverState>,
    databank_state: State<'_, DesktopState>,
) -> Result<crate::databank::DatabankWorkspace, String> {
    let artifact = state
        .live_artifact
        .read()
        .map_err(|_| "discover live databank state is unavailable")?
        .clone()
        .ok_or_else(|| "the live archive has no Holding or Databank strategies yet".to_owned())?;
    let source_path = state
        .job
        .read()
        .map_err(|_| "discover job state is unavailable")?
        .output_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "the live databank output path is unavailable".to_owned())?;
    install_live_databank_artifact(artifact, source_path, &databank_state)
        .map_err(|error| error.to_string())
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
    let symbol = request
        .selected_symbol
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_ascii_uppercase)
        .unwrap_or_else(|| {
            stem.split(['_', '-'])
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("strategy")
                .to_ascii_uppercase()
        });
    let timeframe = match request.decision_timeframe.unwrap_or(DecisionTimeframe::H1) {
        DecisionTimeframe::H1 => "H1",
        DecisionTimeframe::M15 => "M15",
        DecisionTimeframe::H4 => "H4",
    };
    let history_year = request
        .history_start_year
        .unwrap_or(quantforge_data::DEFAULT_HISTORY_START_YEAR);
    // Always Documents/QuantForge/runs — never beside the Wine/MT5 data pack,
    // even when the pack itself lives under a folder named QuantForge.
    let directory = crate::assets::quantforge_runs_root()
        .join(&symbol)
        .join("Databank");
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_millis();
    let base = format!("{symbol}_{timeframe}_{history_year}_databank_{now_ms}");
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
    if request.mode == DiscoverMode::New
        && request
            .selected_symbol
            .as_deref()
            .is_none_or(|symbol| symbol.trim().is_empty())
    {
        return Err("select a symbol before starting Discover".into());
    }
    if request.mode == DiscoverMode::New {
        let broker = load_bound_broker(&request.broker_path, None)?;
        validate_selected_symbol(request.selected_symbol.as_deref(), &broker.symbol)?;
    }
    if request.mode == DiscoverMode::New {
        if let Some(grammar) = request.universal_grammar.as_ref() {
            validate_universal_grammar(grammar)?;
            if request.sl_tp_only_exits.unwrap_or(true) && grammar.minimum_entry_conditions > 3 {
                return Err(
                    "SL/TP-only exits allow 2–3 entry conditions; lower the minimum entry conditions or switch that profile off"
                        .into(),
                );
            }
        }
        if let Some(mode) = request.run_mode.as_deref() {
            if parse_run_mode(mode).is_none() {
                return Err(format!(
                    "unknown run mode '{mode}' (use fast_scout, full_harvest, quota_harvest, or high_performance_islands)"
                ));
            }
        }
        if let Some(year) = request.history_start_year {
            quantforge_data::normalize_history_start_year(year)
                .map_err(|error| error.to_string())?;
        }
        let validation = request
            .validation_fraction
            .unwrap_or(quantforge_quality::DEFAULT_VALIDATION_FRACTION);
        let sealed = request
            .sealed_fraction
            .unwrap_or(quantforge_quality::DEFAULT_SEALED_FRACTION);
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
            request.minimum_development_expectancy_r.is_some(),
            request.require_m1_precision.is_some(),
            request.simple_exits.is_some(),
            request.sl_tp_only_exits.is_some(),
            request.allow_fixed_pip_stops.is_some(),
            request.allow_indicator_exit_rules.is_some(),
            request.allow_time_stops.is_some(),
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
            request.promotion_worker_threads.is_some(),
            request.promotion_queue_capacity.is_some(),
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
            request.robustness_monte_carlo_max_drawdown_ratio.is_some(),
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
            request.history_start_year.is_some(),
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
/// pack. Decision-timeframe paths are allowed here because H1/M15/H4 packs are
/// derived from the same M1 stream and therefore share its sibling sidecar.
pub fn infer_quote_path_public(m1_path: &str) -> Option<PathBuf> {
    infer_quote_path(m1_path)
}

fn infer_quote_path(m1_path: &str) -> Option<PathBuf> {
    let path = Path::new(m1_path);
    let stem = path.file_stem()?.to_str()?;
    let mut candidates = vec![path.with_file_name(format!("{stem}.quotes.csv"))];
    for suffix in ["_H1", "_M15", "_H4"] {
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

fn requested_multi_symbol_minimum_pass(
    request: &DiscoverRequest,
    continued_artifact: Option<&EvolveArtifact>,
) -> usize {
    match (&request.mode, continued_artifact) {
        (DiscoverMode::Continue, Some(artifact)) => {
            artifact.databank.config.multi_symbol_minimum_pass
        }
        _ => request.multi_symbol_minimum_pass.unwrap_or(0),
    }
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
    live_artifact: &Arc<RwLock<Option<EvolveArtifact>>>,
    paused: &Arc<AtomicBool>,
    stop: &Arc<AtomicBool>,
) -> Result<(), String> {
    let clock = ActiveClock::new();
    clock.begin_evaluation_session(0);
    let run_until_stopped = request.run_until_stopped.unwrap_or(true);
    let soft_budget = request.generations;

    let mut loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let mut m1 = load_data_source(
        &request.m1_data_path,
        request.m1_metadata_path.as_deref(),
        request.m1_source_timezone.as_deref(),
    )?;
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
    let history_start_year = match &continued_artifact {
        Some(artifact) => artifact.databank.config.history_start_year,
        None => quantforge_data::normalize_history_start_year(
            request
                .history_start_year
                .unwrap_or(quantforge_data::DEFAULT_HISTORY_START_YEAR),
        )
        .map_err(|error| error.to_string())?,
    };
    let quote_path = infer_quote_path(&request.m1_data_path);
    let mut quote_dataset = quote_path
        .as_ref()
        .map(|path| load_quote_sidecar(path, m1.metadata.as_ref()))
        .transpose()
        .map_err(|error| format!("cannot load bid/ask quote sidecar: {error}"))?;
    trim_market_history_to_year(
        &mut loaded.dataset,
        &mut m1.dataset,
        quote_dataset.as_mut(),
        history_start_year,
    )?;
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
    let wants_pending =
        request.allow_stop_entries.unwrap_or(false) || request.allow_limit_entries.unwrap_or(false);
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
    validate_selected_symbol(request.selected_symbol.as_deref(), &broker.symbol)?;
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
            DecisionTimeframe::H1 => match quote_dataset.as_ref() {
                Some(quotes) => build_decision_from_m1_quotes(
                    &m1.dataset,
                    Some(&loaded.dataset),
                    quotes,
                    broker.point,
                )?,
                None => build_decision_from_m1(&m1.dataset, Some(&loaded.dataset))?,
            },
            DecisionTimeframe::M15 | DecisionTimeframe::H4 => match quote_dataset.as_ref() {
                Some(quotes) => quantforge_data::build_timeframe_from_m1_with_quotes(
                    &m1.dataset,
                    quotes,
                    broker.point,
                    decision_timeframe.interval_ms(),
                    None,
                )
                .map_err(|error| error.to_string())?,
                None => {
                    build_timeframe_from_m1(&m1.dataset, decision_timeframe.interval_ms(), None)
                        .map_err(|error| error.to_string())?
                }
            },
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
    // checkpoints onto a different Development/OOS1/OOS2 cut.
    let (validation_fraction, sealed_fraction) = match (&request.mode, &continued_artifact) {
        (DiscoverMode::Continue, Some(artifact)) => (
            recipe_fraction(
                artifact,
                "validation_fraction",
                quantforge_quality::DEFAULT_VALIDATION_FRACTION,
            ),
            recipe_fraction(
                artifact,
                "sealed_fraction",
                quantforge_quality::DEFAULT_SEALED_FRACTION,
            ),
        ),
        _ => (
            request
                .validation_fraction
                .unwrap_or(quantforge_quality::DEFAULT_VALIDATION_FRACTION),
            request
                .sealed_fraction
                .unwrap_or(quantforge_quality::DEFAULT_SEALED_FRACTION),
        ),
    };
    let (validation_fraction, sealed_fraction) =
        normalize_split_fractions(validation_fraction, sealed_fraction)?;
    if validation_fraction > 0.0 && !promotion_split {
        return Err(
            "OOS1 validation requires promotion split so Development, OOS1 and sealed OOS2 remain separate"
                .into(),
        );
    }
    let oos1_enabled = validation_fraction > 0.0;
    if request.mode == DiscoverMode::Continue
        && oos1_enabled
        && continued_artifact.as_ref().is_some_and(|artifact| {
            artifact
                .manifest
                .recipe
                .config
                .get("oos1_pick_enabled")
                .and_then(Value::as_bool)
                != Some(true)
        })
    {
        return Err(
            "this archive was created while OOS1 validation was disabled; start a new run to reserve and validate OOS1 without changing the historical experiment".into(),
        );
    }
    let development_dataset = (promotion_split || request.mode == DiscoverMode::Continue)
        .then(|| {
            if oos1_enabled {
                development_partition(&search_decision, validation_fraction, sealed_fraction)
            } else {
                unsealed_partition(&search_decision, validation_fraction, sealed_fraction)
            }
        })
        .transpose()?;
    let oos1_dataset = (promotion_split || request.mode == DiscoverMode::Continue)
        .then_some(())
        .filter(|_| oos1_enabled)
        .map(|_| oos1_partition(&search_decision, validation_fraction, sealed_fraction))
        .transpose()?;
    let new_dataset = development_dataset.as_ref().unwrap_or(&search_decision);
    // Development alone drives search and breeding. When reserved, OOS1 is
    // opened only after the full Development battery; OOS2 is never materialized.
    let m1_eval = &m1.dataset;
    // A pack is deliberately opt-in. Merely selecting a pack directory must
    // not add several extra full strategy replays to every scout candidate.
    // Continuations retain the immutable gate stored in the databank.
    let multi_symbol_minimum_pass =
        requested_multi_symbol_minimum_pass(&request, continued_artifact.as_ref());
    let pack = if multi_symbol_minimum_pass > 0 {
        load_fx_pack(
            request.pack_data_dir.as_deref(),
            &broker.symbol,
            validation_fraction,
            sealed_fraction,
            promotion_split || request.mode == DiscoverMode::Continue,
            oos1_enabled,
            &decision_timeframe,
            history_start_year,
        )?
    } else {
        Vec::new()
    };

    let (mut bank, continuation_recipe_hash, starting_generation) = match request.mode {
        DiscoverMode::New => {
            update_phase(
                job,
                "Evaluating initial grammar population",
                if oos1_enabled {
                    "Development fills Holding. The full battery then validates OOS1; sealed OOS2 is never loaded."
                } else {
                    "Fold-stable Development R fills Holding. Sealed holdout is never loaded."
                },
            )?;
            let mut config = new_config(&request)?;
            if !quantforge_broker::fx_multi_symbol_primary(&broker.symbol) {
                // Indices, oil, crypto and metals cannot pass the FX identical-parameter screen.
                config.multi_symbol_minimum_pass = 0;
            }
            let bank = new_databank(new_dataset, m1_eval, &broker, config)
                .map_err(|error| error.to_string())?;
            (bank, None, 0u64)
        }
        DiscoverMode::Continue => {
            let artifact = continued_artifact
                .expect("continuation always loads the databank before partitioning");
            let starting_generation = artifact.databank.completed_generations;
            (
                artifact.databank,
                Some(artifact.manifest.recipe_hash),
                starting_generation,
            )
        }
    };

    // Continuing a bank must measure this process's work, not divide every
    // historical evaluation by a timer that started a few milliseconds ago.
    // A new run keeps the zero baseline established before its initial batch.
    if request.mode == DiscoverMode::Continue {
        clock.begin_evaluation_session(bank.evaluation_count);
    }
    publish_live_databank(
        live_artifact,
        &request,
        &bank,
        &loaded,
        &quality,
        &m1_quality,
        &m1.dataset.data_hash,
        validation_fraction,
        sealed_fraction,
        starting_generation,
        continuation_recipe_hash.clone(),
        0,
        soft_budget,
        run_until_stopped,
    )?;
    update_bank(job, &bank, 0, soft_budget, run_until_stopped, &clock)?;

    let mut completed_now = 0u64;
    let mut last_checkpoint_active_seconds = clock.active_seconds();
    let mut last_holding_count = bank.holding.len();
    let mut holding_stall_generations = 0u64;

    let quota_met = |bank: &Databank| -> bool {
        bank.config
            .target_databank_elites
            .is_some_and(|target| bank.quota_progress_count() >= target)
    };
    let holding_plateaued = |bank: &Databank, stall: u64| -> bool {
        holding_plateau_should_stop(
            run_until_stopped,
            bank.config.build_to_holding,
            bank.holding.len(),
            stall,
        )
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
    let evaluation_oos1 = oos1_dataset.as_ref();
    let evaluation_m1 = if bank.execution_data_hash == m1.dataset.data_hash {
        &m1.dataset
    } else {
        m1_eval
    };
    let session =
        quantforge_discover::EvolutionSession::new(&bank.config, evaluation_dataset.bars.len())
            .map_err(|error| error.to_string())?;

    if request.mode == DiscoverMode::New && bank.evaluation_count == 0 {
        let mut on_progress =
            |evaluated: &Databank| -> Result<bool, quantforge_discover::DiscoverError> {
                update_bank(job, evaluated, 0, soft_budget, run_until_stopped, &clock)
                    .map_err(quantforge_discover::DiscoverError::InvalidConfig)?;
                Ok(!stop.load(Ordering::SeqCst))
            };
        session
            .seed_initial_population(
                &mut bank,
                evaluation_dataset,
                evaluation_oos1,
                evaluation_m1,
                quote_dataset.as_ref(),
                &broker,
                &pack,
                &broker.symbol,
                &mut on_progress,
            )
            .map_err(|error| error.to_string())?;
        publish_live_databank(
            live_artifact,
            &request,
            &bank,
            &loaded,
            &quality,
            &m1_quality,
            &m1.dataset.data_hash,
            validation_fraction,
            sealed_fraction,
            starting_generation,
            continuation_recipe_hash.clone(),
            0,
            soft_budget,
            run_until_stopped,
        )?;
        update_bank(job, &bank, 0, soft_budget, run_until_stopped, &clock)?;
        if stop.load(Ordering::SeqCst) {
            session
                .flush_promotions(&mut bank)
                .map_err(|error| error.to_string())?;
        }
    }

    loop {
        let pausing_now = paused.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst);
        if pausing_now {
            session
                .flush_promotions(&mut bank)
                .map_err(|error| error.to_string())?;
            publish_live_databank(
                live_artifact,
                &request,
                &bank,
                &loaded,
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
            )?;
            update_bank(
                job,
                &bank,
                completed_now,
                soft_budget,
                run_until_stopped,
                &clock,
            )?;
            if bank.evaluation_count > 0
                && (!bank.elites.is_empty() || !bank.accepted_pool.is_empty())
            {
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
                    Path::new(&request.databank_path),
                    true,
                )?;
                let snapshot = immutable_snapshot_path(&request.databank_path, "paused")?;
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
                    false,
                    &snapshot,
                    false,
                )?;
                if let Ok(mut view) = job.write() {
                    view.latest_immutable_snapshot_path = Some(display_path(&snapshot));
                    view.message = format!(
                        "Paused safely. Immutable snapshot: {}",
                        display_path(&snapshot)
                    );
                }
            }
        }
        wait_if_paused(job, paused, stop, &clock)?;
        if pausing_now {
            last_checkpoint_active_seconds = clock.active_seconds();
        }
        if stop.load(Ordering::SeqCst) {
            break;
        }
        if quota_met(&bank) {
            break;
        }
        // A Holding plateau is useful evidence for a finite quota recipe, but
        // it is not a valid reason to stop an explicitly open-ended Discover
        // run. The old behaviour silently handed open-ended runs to the
        // factory after 25 flat generations.
        if holding_plateaued(&bank, holding_stall_generations) {
            if let Ok(mut view) = job.write() {
                view.message = format!(
                    "Holding plateaued at {} names for {HOLDING_STALL_GENERATIONS} generations — stopping Discover so factory can run.",
                    bank.holding.len()
                );
            }
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
        let breeding = bank.pot_size() >= bank.config.mutate_after_elites;
        let status_message = if breeding {
            "Scout keeps breeding. Side workers run M1 80/130 into Holding. Fold-R, plateau, CPCV, and Monte Carlo wait for the Holding battery. OOS2 is untouched."
        } else {
            "Candidates enter the Development reservoir only. After breeding unlocks: M1 80/130 → Holding. Databank tests run from the Holding tab."
        };
        update_phase(job, &phase_label, status_message)?;

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
        if bank.config.build_to_holding {
            let holding_now = bank.holding.len();
            if holding_now > last_holding_count {
                last_holding_count = holding_now;
                holding_stall_generations = 0;
            } else {
                holding_stall_generations += 1;
            }
        }
        publish_live_databank(
            live_artifact,
            &request,
            &bank,
            &loaded,
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
        )?;
        update_bank(
            job,
            &bank,
            completed_now,
            soft_budget,
            run_until_stopped,
            &clock,
        )?;

        let resident_mb = resident_memory_mb().unwrap_or(0);
        if let Ok(mut view) = job.write() {
            view.resident_memory_mb = resident_mb;
        }
        let memory_limit_mb = request.max_memory_mb.unwrap_or(8_192).max(1_024);
        if resident_mb >= memory_limit_mb {
            if let Ok(mut view) = job.write() {
                view.phase = "Memory limit reached — saving safely".into();
                view.message = format!(
                    "Resident memory reached {resident_mb} MB of the {memory_limit_mb} MB limit. QuantForge stopped cleanly and is writing the final artifact."
                );
            }
            break;
        }

        let checkpoint_due = clock.active_seconds() - last_checkpoint_active_seconds
            >= RECOVERY_CHECKPOINT_INTERVAL.as_secs_f64();
        if checkpoint_due
            && bank.evaluation_count > 0
            && (!bank.elites.is_empty() || !bank.accepted_pool.is_empty())
        {
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
                Path::new(&request.databank_path),
                true,
            )?;
            last_checkpoint_active_seconds = clock.active_seconds();
            if let Ok(mut view) = job.write() {
                view.output_path = Some(display_path(Path::new(&request.databank_path)));
                view.message = format!(
                    "Bank growing: {} niches after {} evaluations.",
                    bank.coverage(),
                    bank.evaluation_count
                );
            }
        }

        // Deposited Holding is enough to stop. Waiting out the M1 queue after
        // every generation serialized scout behind promotion and is why
        // Looked-at crawled versus the old pipelined runs. A Holding plateau
        // (no new names for many generations) also stops so thin symbols can
        // factory instead of waiting on a quota they will never fill.
        if quota_met(&bank) {
            break;
        }
        if holding_plateaued(&bank, holding_stall_generations) {
            if let Ok(mut view) = job.write() {
                view.message = format!(
                    "Holding plateaued at {} names for {HOLDING_STALL_GENERATIONS} generations — stopping Discover so factory can run.",
                    bank.holding.len()
                );
            }
            break;
        }
    }

    session
        .flush_promotions(&mut bank)
        .map_err(|error| error.to_string())?;
    publish_live_databank(
        live_artifact,
        &request,
        &bank,
        &loaded,
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
    )?;

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
        continuation_recipe_hash.clone(),
        completed_now,
        soft_budget,
        run_until_stopped,
        stop.load(Ordering::SeqCst),
        &clock,
    )
}

fn validate_selected_symbol(selected: Option<&str>, broker_symbol: &str) -> Result<(), String> {
    let Some(selected) = selected.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if !selected.eq_ignore_ascii_case(broker_symbol) {
        return Err(format!(
            "selected symbol {selected} does not match the bound data/broker symbol {broker_symbol}; reselect {selected} to refresh all pack paths"
        ));
    }
    Ok(())
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
    stopped_by_user: bool,
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

    if bank.elites.is_empty() && bank.accepted_pool.is_empty() {
        let funnel = funnel_summary(&bank);
        let mut view = job
            .write()
            .map_err(|_| "discover job state is unavailable".to_owned())?;
        view.status = "completed";
        view.phase = "Completed with an empty bank".into();
        view.message = format!(
            "No elites passed the post-breed pipeline (Development CPCV/robustness → M1 → OOS1 validation) after {} evaluations across {} generations. {funnel} Keep searching until breeding unlocks, loosen gates, or check data.",
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
        "Writing final artifacts",
        "The recovery checkpoint and immutable final snapshot are being written atomically.",
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
        continuation_recipe_hash.clone(),
        completed_now,
        soft_budget,
        run_until_stopped,
        true,
        Path::new(&request.databank_path),
        true,
    )?;
    let snapshot = immutable_snapshot_path(
        &request.databank_path,
        if stopped_by_user {
            "stopped"
        } else {
            "completed"
        },
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
        &snapshot,
        false,
    )?;

    let mut view = job
        .write()
        .map_err(|_| "discover job state is unavailable".to_owned())?;
    view.status = "completed";
    let quota_complete = bank
        .config
        .target_databank_elites
        .is_some_and(|target| bank.quota_progress_count() >= target);
    view.phase = if quota_complete {
        format!(
            "Quota complete · {} {}",
            bank.quota_progress_count(),
            if bank.config.build_to_holding {
                "holding"
            } else {
                "databank elites"
            }
        )
    } else if stop_was_early(completed_now, soft_budget, run_until_stopped) {
        "Stopped and checkpointed".into()
    } else {
        "Discovery checkpoint complete".into()
    };
    view.output_path = Some(display_path(Path::new(&request.databank_path)));
    view.latest_immutable_snapshot_path = Some(display_path(&snapshot));
    view.message = if quota_complete {
        format!(
            "Reached {} quota ({}/{}). Saved after {} evaluations. Start a new Discover for the next family or asset.",
            if bank.config.build_to_holding {
                "Holding"
            } else {
                "databank"
            },
            bank.quota_progress_count(),
            bank.config.target_databank_elites.unwrap_or(0),
            view.evaluation_count
        )
    } else {
        format!(
            "Saved {} strategies after {} evaluations. Immutable snapshot: {}",
            view.coverage,
            view.evaluation_count,
            display_path(&snapshot)
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

fn automatic_factory_target(
    run_until_stopped: bool,
    configured_target: Option<usize>,
) -> Option<usize> {
    (!run_until_stopped)
        .then_some(configured_target.filter(|&target| target > 0))
        .flatten()
}

fn holding_plateau_should_stop(
    run_until_stopped: bool,
    build_to_holding: bool,
    holding_count: usize,
    stall_generations: u64,
) -> bool {
    !run_until_stopped
        && build_to_holding
        && stall_generations >= HOLDING_STALL_GENERATIONS
        && holding_count >= HOLDING_STALL_MIN
}

fn funnel_summary(bank: &Databank) -> String {
    let telemetry = &bank.telemetry;
    format!(
        "Rejects — scout {}, deposit {}, ambiguous {}, M1 retention {}, Development R {}, WF {}, MC {}, param {}, OOS1 {}, clone {}, corr {}, niche {}, family {}, eval {}.",
        telemetry.rejected_gate,
        telemetry.rejected_deposit_gate,
        telemetry.rejected_ambiguous,
        telemetry.rejected_m1_fidelity,
        telemetry.rejected_development_expectancy,
        telemetry.rejected_walk_forward,
        telemetry.rejected_monte_carlo,
        telemetry.rejected_param_neighborhood,
        telemetry.rejected_oos1,
        telemetry.rejected_clone,
        telemetry.rejected_correlated,
        telemetry.rejected_niche_not_improved,
        telemetry.rejected_family_not_improved,
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
    output_path: &Path,
    replace_existing: bool,
) -> Result<(), String> {
    let artifact = build_discover_artifact(
        request,
        bank,
        source,
        broker,
        metadata_hash,
        quality,
        m1_quality,
        m1_data_hash,
        validation_fraction,
        sealed_fraction,
        starting_generation,
        continuation_recipe_hash,
        completed_now,
        soft_budget,
        run_until_stopped,
        partial,
        output_path,
    )?;
    if replace_existing {
        write_json_replacing(output_path, &artifact).map_err(|error| error.to_string())?;
    } else {
        write_json_new(output_path, &artifact).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_discover_artifact(
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
    output_path: &Path,
) -> Result<EvolveArtifact, String> {
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
            "decision_timeframe".into(),
            json!(
                match request.decision_timeframe.unwrap_or(DecisionTimeframe::H1) {
                    DecisionTimeframe::H1 => "H1",
                    DecisionTimeframe::M15 => "M15",
                    DecisionTimeframe::H4 => "H4",
                }
            ),
        ),
        ("databank".into(), json!(display_path(output_path))),
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
        ("oos1_pick_enabled".into(), json!(validation_fraction > 0.0)),
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
    Ok(EvolveArtifact {
        manifest,
        source,
        broker,
        metadata_hash,
        data_quality: quality.clone(),
        coverage: bank.coverage(),
        qd_score: bank.qd_score(),
        databank: bank.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn publish_live_databank(
    live_artifact: &Arc<RwLock<Option<EvolveArtifact>>>,
    request: &DiscoverRequest,
    bank: &Databank,
    loaded: &crate::data_lab::LoadedDataSource,
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
) -> Result<(), String> {
    if bank.elites.is_empty() && bank.holding.is_empty() {
        return Ok(());
    }
    let revision = bank.telemetry.databank_accepted
        + bank.telemetry.databank_replaced
        + bank.telemetry.holding_accepted
        + bank.telemetry.holding_replaced;
    let unchanged = live_artifact
        .read()
        .map_err(|_| "discover live databank state is unavailable")?
        .as_ref()
        .is_some_and(|artifact| {
            let old = &artifact.databank.telemetry;
            let old_revision = old.databank_accepted
                + old.databank_replaced
                + old.holding_accepted
                + old.holding_replaced;
            old_revision == revision
                && artifact.databank.elites.len() == bank.elites.len()
                && artifact.databank.holding.len() == bank.holding.len()
        });
    if unchanged {
        return Ok(());
    }

    // The live inspector needs Holding + Databank elites, not the potentially
    // enormous breeding reservoir.
    let mut live_bank = bank.clone();
    live_bank.accepted_pool.clear();
    live_bank.accepted_coverage_map.clear();
    live_bank.specialist_pool.clear();
    live_bank.specialist_coverage_map.clear();
    let artifact = build_discover_artifact(
        request,
        &live_bank,
        display_path(Path::new(&request.data_path)),
        display_path(Path::new(&request.broker_path)),
        loaded
            .metadata
            .as_ref()
            .map(|value| value.metadata_hash.clone()),
        quality,
        m1_quality,
        m1_data_hash,
        validation_fraction,
        sealed_fraction,
        starting_generation,
        continuation_recipe_hash,
        completed_now,
        soft_budget,
        run_until_stopped,
        true,
        Path::new(&request.databank_path),
    )?;
    *live_artifact
        .write()
        .map_err(|_| "discover live databank state is unavailable")? = Some(artifact);
    Ok(())
}

fn immutable_snapshot_path(databank_path: &str, reason: &str) -> Result<PathBuf, String> {
    let path = Path::new(databank_path);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("quantforge-databank");
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ");
    let mut candidate = parent.join(format!("{stem}.{reason}.{stamp}.json"));
    let mut suffix = 2usize;
    while candidate.exists() {
        candidate = parent.join(format!("{stem}.{reason}.{stamp}.{suffix}.json"));
        suffix += 1;
    }
    Ok(candidate)
}

fn resident_memory_mb() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kib = String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kib.div_ceil(1_024))
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
    reserve_oos1: bool,
    decision_timeframe: &str,
    history_start_year: u16,
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
        let mut loaded = if meta_beside.is_file() {
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
        apply_history_start_year(&mut loaded.dataset, history_start_year)?;
        let dataset = if apply_promotion_split {
            if reserve_oos1 {
                development_partition(&loaded.dataset, validation_fraction, sealed_fraction)?
            } else {
                unsealed_partition(&loaded.dataset, validation_fraction, sealed_fraction)?
            }
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

/// Threads Rayon will actually use when a caller still passes the legacy
/// `0 = all CPUs` sentinel outside resolved scout/promotion helpers.
#[allow(dead_code)]
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
        .filter(|value| value.is_finite() && *value >= 0.0 && *value < 1.0)
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
        || !(0.0..=0.5).contains(&validation_fraction)
        || !(0.05..=0.4).contains(&sealed_fraction)
    {
        return Err("OOS1 reserve must be 0–50% and sealed holdout must be 5–40%".into());
    }
    let validation = validation_fraction;
    let sealed = sealed_fraction;
    if validation + sealed >= 0.9 {
        return Err(format!(
            "OOS1 ({validation:.2}) + sealed ({sealed:.2}) leaves less than 10% for Development"
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

fn unsealed_partition(
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
    let end = plan.development.bar_count + plan.validation.bar_count;
    slice_partition(dataset, 0, end)
}

#[allow(dead_code)]
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

fn sealed_partition(
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
    let start = plan.development.bar_count + plan.validation.bar_count;
    slice_partition(dataset, start, dataset.bars.len())
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
        "high_performance_islands" | "highperformanceislands" | "islands" => {
            Some(quantforge_discover::DiscoverRunMode::HighPerformanceIslands)
        }
        _ => None,
    }
}

fn broker_symbol_from_path(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".broker.json"))
        .map(str::to_ascii_uppercase)
}

fn new_config(request: &DiscoverRequest) -> Result<DiscoverConfig, String> {
    let sl_tp_only_exits = request.sl_tp_only_exits.unwrap_or(true);
    let allow_fixed_pip_stops = request.allow_fixed_pip_stops.unwrap_or(false);
    let broker = load_bound_broker(&request.broker_path, None)?;
    if allow_fixed_pip_stops && !quantforge_broker::fx_multi_symbol_primary(&broker.symbol) {
        return Err(format!(
            "fixed pip SL/TP is currently available for FX symbols only; {} should use ATR/R protection",
            broker.symbol
        ));
    }
    let fixed_pip_size_points = if matches!(broker.digits, 3 | 5) {
        10.0
    } else {
        1.0
    };
    let mut universal_grammar = request.universal_grammar.clone().unwrap_or_default();
    if sl_tp_only_exits {
        // The constrained production lane intentionally leaves at most three
        // entry conditions. Exit bounds remain part of the legacy grammar
        // contract but are erased before evaluation.
        universal_grammar.maximum_entry_conditions =
            universal_grammar.maximum_entry_conditions.min(3);
        universal_grammar.maximum_exit_conditions =
            universal_grammar.maximum_exit_conditions.min(2);
    }
    let mut commission = request
        .commission_per_lot_round_turn
        .ok_or_else(|| "commission is required for a new databank".to_owned())?;
    if let Some(symbol) = broker_symbol_from_path(&request.broker_path) {
        let expected = quantforge_broker::default_commission_per_lot_round_turn(&symbol);
        // Saved profiles and the FX default still carry $7 on zero-commission symbols.
        if (commission - 7.0).abs() < f64::EPSILON && expected == 0.0 {
            commission = 0.0;
        }
    }
    Ok(DiscoverConfig {
        initial_candidates: request.initial_candidates.unwrap_or(500),
        batch_size: request.batch_size.unwrap_or(200),
        correlation_threshold: request.correlation_threshold.unwrap_or(0.75),
        novelty_weight: request.novelty_weight.unwrap_or(10.0),
        tournament_size: 4,
        structural_mutation_probability: 0.18,
        seed: request.seed.unwrap_or(42),
        universal_grammar,
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
        minimum_development_expectancy_r: request.minimum_development_expectancy_r.unwrap_or(0.25),
        require_m1_precision: request.require_m1_precision.unwrap_or(true),
        simple_exits: request.simple_exits.unwrap_or(true),
        sl_tp_only_exits,
        allow_fixed_pip_stops,
        fixed_pip_size_points,
        allow_indicator_exit_rules: request.allow_indicator_exit_rules.unwrap_or(false),
        allow_time_stops: request.allow_time_stops.unwrap_or(false),
        allow_break_even: request.allow_break_even.unwrap_or(false),
        allow_trailing_stops: request.allow_trailing_stops.unwrap_or(false),
        allow_partial_exits: request.allow_partial_exits.unwrap_or(false),
        allow_market_entries: request.allow_market_entries.unwrap_or(true),
        allow_stop_entries: request.allow_stop_entries.unwrap_or(false),
        allow_limit_entries: request.allow_limit_entries.unwrap_or(false),
        flatten_at_22: request.flatten_at_22.unwrap_or(false) || sl_tp_only_exits,
        end_of_day_hour: request.end_of_day_hour.unwrap_or(23),
        max_one_entry_per_day: request.max_one_entry_per_day.unwrap_or(true),
        mutate_after_elites: request.mutate_after_elites.unwrap_or(300),
        random_fill_fraction: request.random_fill_fraction.unwrap_or(0.75),
        worker_threads: request.worker_threads.unwrap_or(0),
        promotion_worker_threads: request.promotion_worker_threads.unwrap_or(0),
        promotion_queue_capacity: request.promotion_queue_capacity.unwrap_or(64),
        max_accepted_pool_elites: 10_000,
        max_specialist_pool_elites: 2_000,
        max_databank_elites: 10_000,
        max_holding_elites: 10_000,
        max_elites_per_niche: 8,
        max_promoted_per_niche: 4,
        max_per_entry_family: 24,
        build_to_holding: request.build_to_holding.unwrap_or(true),
        require_m1_robustness: request.require_m1_robustness.unwrap_or(true),
        robustness_folds: request.robustness_folds.unwrap_or(8),
        robustness_monte_carlo_trials: request.robustness_monte_carlo_trials.unwrap_or(1_000),
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
        robustness_neighborhood_samples: request.robustness_neighborhood_samples.unwrap_or(200),
        robustness_perturbation_fraction: request
            .robustness_perturbation_fraction
            .unwrap_or(quantforge_discover::PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION),
        minimum_neighborhood_survival_fraction: request
            .minimum_neighborhood_survival_fraction
            .unwrap_or(0.55),
        calendar_year_folds: request.calendar_year_folds.unwrap_or(false),
        minimum_deflated_trade_sharpe: request.minimum_deflated_trade_sharpe,
        multi_symbol_minimum_pass: request.multi_symbol_minimum_pass.unwrap_or(0),
        history_start_year: quantforge_data::normalize_history_start_year(
            request
                .history_start_year
                .unwrap_or(quantforge_data::DEFAULT_HISTORY_START_YEAR),
        )
        .map_err(|error| error.to_string())?,
        // `generalIslandCount` uses 0 as an "auto / single island" sentinel in the
        // UI, so `unwrap_or(1)` (which only fires on `None`) let an explicit 0
        // through and tripped `island_count must be at least 1`. Clamp to >=1.
        island_count: request
            .general_island_count
            .filter(|&count| count > 0)
            .unwrap_or(1),
        migration_interval: request.migration_interval.unwrap_or(10),
        migration_elites: request.migration_elites.unwrap_or(2),
        general_island_count: request.general_island_count.unwrap_or(0),
        refinement_island_count: 0,
        exploration_island_count: 0,
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
    evaluation_baseline: Arc<AtomicU64>,
    throughput_samples: Arc<Mutex<VecDeque<(f64, u64)>>>,
}

impl ActiveClock {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            paused_millis: Arc::new(AtomicU64::new(0)),
            evaluation_baseline: Arc::new(AtomicU64::new(0)),
            throughput_samples: Arc::new(Mutex::new(VecDeque::from([(0.0, 0)]))),
        }
    }

    fn begin_evaluation_session(&self, evaluation_count: u64) {
        self.evaluation_baseline
            .store(evaluation_count, Ordering::SeqCst);
        if let Ok(mut samples) = self.throughput_samples.lock() {
            samples.clear();
            samples.push_back((self.active_seconds(), evaluation_count));
        }
    }

    fn add_paused(&self, span: Duration) {
        self.paused_millis
            .fetch_add(span.as_millis() as u64, Ordering::SeqCst);
    }

    fn active_seconds(&self) -> f64 {
        let paused = self.paused_millis.load(Ordering::SeqCst) as f64 / 1_000.0;
        (self.started.elapsed().as_secs_f64() - paused).max(0.0)
    }

    fn active_hours(&self) -> f64 {
        self.active_seconds().max(1.0) / 3600.0
    }

    fn lifetime_evaluations_per_hour(&self, evaluation_count: u64) -> f64 {
        let baseline = self.evaluation_baseline.load(Ordering::SeqCst);
        evaluation_count.saturating_sub(baseline) as f64 / self.active_hours()
    }

    fn rolling_evaluations_per_hour(&self, evaluation_count: u64) -> f64 {
        let now = self.active_seconds();
        let Ok(mut samples) = self.throughput_samples.lock() else {
            return 0.0;
        };
        if samples
            .back()
            .is_none_or(|(_, count)| *count != evaluation_count)
        {
            samples.push_back((now, evaluation_count));
        }
        let cutoff = (now - ROLLING_THROUGHPUT_WINDOW.as_secs_f64()).max(0.0);
        while samples.len() > 2 && samples.get(1).is_some_and(|(at, _)| *at < cutoff) {
            samples.pop_front();
        }
        let Some((then, count)) = samples.front().copied() else {
            return 0.0;
        };
        let seconds = (now - then).max(1.0);
        evaluation_count.saturating_sub(count) as f64 / seconds * 3_600.0
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
        + telemetry.rejected_family_not_improved
        + telemetry.rejected_precision
        + telemetry.rejected_ambiguous
        + telemetry.rejected_oos1
        + telemetry.rejected_development_expectancy
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
    let holding_elites = bank.holding_size();
    let databank_elites = bank.coverage();
    let quota_count = bank.quota_progress_count();
    let target_databank = bank.config.target_databank_elites;
    let breeding_active = pot_elites >= mutate_after && !bank.accepted_pool.is_empty();
    let queue_depth = telemetry.promotion_queue_depth;
    let inflight = telemetry.promotion_inflight;
    let holding_label = if bank.config.build_to_holding {
        "holding"
    } else {
        "databank"
    };
    let phase = if let Some(target) = target_databank {
        format!(
            "Quota · {holding_label} {quota_count}/{target} · pot {pot_elites} · gen {completed_now}"
        )
    } else if breeding_active {
        format!(
            "Breeding · pot {pot_elites} · holding {holding_elites} · databank {databank_elites} · promo queue {queue_depth} · gen {completed_now}"
        )
    } else {
        format!(
            "Filling initial pot · {pot_elites}/{mutate_after} · holding {holding_elites} · gen {completed_now}"
        )
    };
    let pot_message = if let Some(target) = target_databank {
        format!(
            "Quota Harvest: {holding_label} {quota_count}/{target} (stop at {target}). Pot {pot_elites} is only a breeding bag — not the goal. {}",
            funnel_summary(bank)
        )
    } else if breeding_active {
        format!(
            "Build continues; Holding pipeline on side workers (queue {queue_depth}, {inflight} in flight). Pot {pot_elites} · holding {holding_elites} · databank {databank_elites}. {}",
            funnel_summary(bank)
        )
    } else {
        format!(
            "Development reservoir {pot_elites} (breed at {mutate_after}). Holding {holding_elites} after breeding (H1+M1; battery deferred). {} more reservoir members until breeding. {}",
            mutate_after.saturating_sub(pot_elites),
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
    view.holding_elites = holding_elites;
    view.databank_elites = databank_elites;
    view.live_databank_revision = telemetry.databank_accepted
        + telemetry.databank_replaced
        + telemetry.holding_accepted
        + telemetry.holding_replaced;
    view.target_databank_elites = target_databank;
    view.mutate_after_elites = mutate_after;
    view.breeding_active = breeding_active;
    // Report resolved scout / promotion split, not the 0=auto sentinel.
    view.worker_threads = bank.config.resolved_scout_worker_threads();
    view.promotion_worker_threads = bank.config.resolved_promotion_worker_threads();
    view.promotion_queue_capacity = bank.config.promotion_queue_capacity;
    view.promotion_queue_depth = telemetry.promotion_queue_depth;
    view.promotion_inflight = telemetry.promotion_inflight;
    view.promotions_enqueued = telemetry.promotions_enqueued;
    view.promotions_completed = telemetry.promotions_completed;
    view.promotion_backpressure_events = telemetry.promotion_backpressure_events;
    view.promotions_per_hour = telemetry.promotions_completed as f64 / hours;
    view.coverage = databank_elites;
    view.qd_score = bank.qd_score();
    view.rejected_gate = telemetry.rejected_gate;
    view.rejected_deposit_gate = telemetry.rejected_deposit_gate;
    view.rejected_precision = telemetry.rejected_precision;
    view.rejected_ambiguous = telemetry.rejected_ambiguous;
    view.rejected_oos1 = telemetry.rejected_oos1;
    view.rejected_development_expectancy = telemetry.rejected_development_expectancy;
    view.rejected_m1_fidelity = telemetry.rejected_m1_fidelity;
    view.rejected_walk_forward = telemetry.rejected_walk_forward;
    view.rejected_monte_carlo = telemetry.rejected_monte_carlo;
    view.rejected_param_neighborhood = telemetry.rejected_param_neighborhood;
    view.rejected_multi_symbol = telemetry.rejected_multi_symbol;
    view.rejected_deflated_sharpe = telemetry.rejected_deflated_sharpe;
    view.rejected_clone = telemetry.rejected_clone;
    view.rejected_correlated = telemetry.rejected_correlated;
    view.rejected_niche_not_improved = telemetry.rejected_niche_not_improved;
    view.rejected_family_not_improved = telemetry.rejected_family_not_improved;
    view.rejected_evaluation = telemetry.rejected_evaluation;
    view.rejected_total = rejected_total;
    let lifetime_evaluations_per_hour = clock.lifetime_evaluations_per_hour(bank.evaluation_count);
    view.rolling_evaluations_per_hour = clock.rolling_evaluations_per_hour(bank.evaluation_count);
    view.lifetime_evaluations_per_hour = lifetime_evaluations_per_hour;
    view.evaluations_per_hour = lifetime_evaluations_per_hour;
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

    #[test]
    fn open_ended_discover_never_stops_for_a_holding_plateau_or_factory_target() {
        assert!(!holding_plateau_should_stop(true, true, 111, 25));
        assert_eq!(automatic_factory_target(true, Some(1)), None);
    }

    #[test]
    fn portfolio_lane_is_always_a_new_isolated_discover_job() {
        let mut lane = request("/tmp/portfolio-lane.json".into());
        lane.selected_symbol = Some("EURUSD".into());
        lane.worker_threads = Some(2);
        let view = portfolio_lane_job(&lane, 2);

        assert_eq!(view.mode, Some(DiscoverModeView::New));
        assert_eq!(view.status, "queued");
        assert_eq!(view.worker_threads, 2);
        assert_eq!(
            view.output_path.as_deref(),
            Some("/tmp/portfolio-lane.json")
        );
    }

    #[test]
    fn portfolio_lane_prefers_its_immutable_checkpoint_for_reopening() {
        let lane = request("/tmp/portfolio-working.json".into());
        let mut job = portfolio_lane_job(&lane, 2);
        job.latest_immutable_snapshot_path = Some("/tmp/portfolio-stopped.json".into());

        let view = portfolio_lane_view("EURUSD", &job);

        assert_eq!(view.output_path, "/tmp/portfolio-stopped.json");
    }

    #[test]
    fn finite_discover_can_use_an_explicit_plateau_or_factory_target() {
        assert!(holding_plateau_should_stop(false, true, 40, 25));
        assert_eq!(automatic_factory_target(false, Some(1)), Some(1));
        assert_eq!(automatic_factory_target(false, Some(0)), None);
    }

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
            selected_symbol: Some("EURUSD".into()),
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
            minimum_development_expectancy_r: Some(0.0),
            oos1_expectancy_retention: Some(0.7),
            require_m1_precision: Some(false),
            simple_exits: Some(true),
            sl_tp_only_exits: Some(true),
            allow_fixed_pip_stops: Some(false),
            allow_indicator_exit_rules: Some(false),
            allow_time_stops: Some(false),
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
            promotion_worker_threads: Some(1),
            promotion_queue_capacity: Some(8),
            max_memory_mb: Some(2048),
            require_m1_robustness: Some(false),
            build_to_holding: Some(false),
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
            general_island_count: None,
            refinement_island_count: None,
            exploration_island_count: None,
            migration_interval: None,
            migration_elites: None,
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
            history_start_year: None,
            factory_after_discover: None,
            factory_queue_limit: None,
            factory_target_databank: None,
            factory_max_correlation: None,
        }
    }

    #[test]
    fn cross_symbol_screen_is_opt_in_for_new_jobs() {
        let directory = tempdir().expect("temp directory");
        let mut request = request(directory.path().join("bank.json").display().to_string());
        request.pack_data_dir = Some("/unused-pack-directory".into());
        request.multi_symbol_minimum_pass = None;
        assert_eq!(requested_multi_symbol_minimum_pass(&request, None), 0);

        request.multi_symbol_minimum_pass = Some(6);
        assert_eq!(requested_multi_symbol_minimum_pass(&request, None), 6);
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
    fn new_discovery_rejects_a_stale_cross_symbol_binding() {
        let directory = tempdir().expect("temp directory");
        let mut request = request(directory.path().join("bank.json").display().to_string());
        request.selected_symbol = Some("AUDUSD".into());

        let error = validate_request(&request).expect_err("EURUSD inputs cannot run as AUDUSD");
        assert!(error.contains("selected symbol AUDUSD"));
        assert!(error.contains("EURUSD"));
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

        request.universal_grammar = Some(UniversalGrammarConfig {
            minimum_entry_conditions: 4,
            maximum_entry_conditions: 4,
            ..UniversalGrammarConfig::default()
        });
        let error = validate_request(&request).expect_err("constrained profile caps at three");
        assert!(error.contains("SL/TP-only"));
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
    fn throughput_session_excludes_historical_continuation_evaluations() {
        let clock = ActiveClock::new();
        clock.begin_evaluation_session(1_000_000);

        assert_eq!(clock.lifetime_evaluations_per_hour(1_000_000), 0.0);
        assert_eq!(clock.rolling_evaluations_per_hour(1_000_000), 0.0);
        assert!(clock.lifetime_evaluations_per_hour(1_000_100) > 0.0);
        assert!(clock.rolling_evaluations_per_hour(1_000_100) > 0.0);
    }

    #[test]
    fn immutable_snapshot_names_are_unique_and_keep_the_live_path_untouched() {
        let directory = tempdir().expect("temp directory");
        let live = directory.path().join("AUDUSD_H1_databank.json");
        let first = immutable_snapshot_path(&live.display().to_string(), "paused").unwrap();
        fs::write(&first, "snapshot").unwrap();
        let second = immutable_snapshot_path(&live.display().to_string(), "paused").unwrap();

        assert_ne!(first, second);
        assert!(
            first
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(".paused.")
        );
        assert_eq!(first.parent(), live.parent());
        assert_ne!(first, live);
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
        assert!(normalize_split_fractions(0.0, 1.0 / 3.0).is_ok());
        assert!(normalize_split_fractions(0.2, 0.1).is_ok());
        assert!(normalize_split_fractions(0.5, 0.33).is_ok());
        assert!(normalize_split_fractions(0.51, 0.2).is_err());
        let error = normalize_split_fractions(0.5, 0.45).expect_err("IS must remain");
        assert!(error.contains("less than 10%"));
    }

    #[test]
    fn unsealed_equals_development_when_oos1_is_off() {
        let loaded = load_data_source(
            &fixture("EURUSD_M15_sample.tsv"),
            Some(&fixture("EURUSD_M15_sample.metadata.csv")),
            None,
        )
        .expect("fixture should load");
        let unsealed = unsealed_partition(&loaded.dataset, 0.0, 1.0 / 3.0).expect("unsealed");
        let development =
            development_partition(&loaded.dataset, 0.0, 1.0 / 3.0).expect("development");
        assert_eq!(unsealed.data_hash, development.data_hash);
        assert_eq!(unsealed.bars.len(), development.bars.len());
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
            &Arc::new(RwLock::new(None)),
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
            &Arc::new(RwLock::new(None)),
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

    #[test]
    fn h4_is_a_four_hour_decision_lane() {
        use quantforge_data::Bar;
        let bars = vec![
            Bar {
                timestamp_ms: 0,
                open: 1.0,
                high: 1.1,
                low: 0.9,
                close: 1.05,
                tick_volume: 1,
                real_volume: 0,
                spread_points: None,
            },
            Bar {
                timestamp_ms: DecisionTimeframe::H4.interval_ms(),
                open: 1.05,
                high: 1.15,
                low: 1.0,
                close: 1.1,
                tick_volume: 1,
                real_volume: 0,
                spread_points: None,
            },
        ];
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
            bars,
        };

        assert_eq!(DecisionTimeframe::H4.interval_ms(), 14_400_000);
        assert_eq!(decision_timeframe_label(&dataset).unwrap(), "H4");
    }

    #[test]
    fn real_pack_timeframe_bakeoff_smoke_when_requested() {
        let Ok(root) = std::env::var("QF_TIMEFRAME_BAKEOFF_PACK") else {
            return;
        };
        let root = Path::new(&root);
        let request = TimeframeBakeoffRequest {
            data_path: root
                .join("ICMarketsSC-Demo_EURUSD_H1_2016_present.tsv")
                .display()
                .to_string(),
            metadata_path: Some(
                root.join("ICMarketsSC-Demo_EURUSD_H1_2016_present.metadata.csv")
                    .display()
                    .to_string(),
            ),
            source_timezone: None,
            m1_data_path: root
                .join("ICMarketsSC-Demo_EURUSD_M1_2016_present.tsv")
                .display()
                .to_string(),
            m1_metadata_path: Some(
                root.join("ICMarketsSC-Demo_EURUSD_M1_2016_present.metadata.csv")
                    .display()
                    .to_string(),
            ),
            m1_source_timezone: None,
            broker_path: root.join("EURUSD.broker.json").display().to_string(),
            draws_per_cell: 20,
            seed: 42,
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
            commission_per_lot_round_turn: 7.0,
            slippage_points_per_side: 0.0,
            fallback_spread_points: None,
            validation_fraction: 0.2,
            sealed_fraction: 1.0 / 3.0,
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 2,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 1,
            history_start_year: Some(2016),
        };
        let report = run_timeframe_bakeoff(request).expect("real pack bakeoff should run");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.pair.paired_comparisons, 20);
    }

    #[test]
    fn real_pack_timeframe_ablation_smoke_when_requested() {
        let Ok(root) = std::env::var("QF_TIMEFRAME_ABLATION_PACK") else {
            return;
        };
        let root = Path::new(&root);
        let request = TimeframeBakeoffRequest {
            data_path: root
                .join("ICMarketsSC-Demo_EURUSD_H1_2016_present.tsv")
                .display()
                .to_string(),
            metadata_path: Some(
                root.join("ICMarketsSC-Demo_EURUSD_H1_2016_present.metadata.csv")
                    .display()
                    .to_string(),
            ),
            source_timezone: None,
            m1_data_path: root
                .join("ICMarketsSC-Demo_EURUSD_M1_2016_present.tsv")
                .display()
                .to_string(),
            m1_metadata_path: Some(
                root.join("ICMarketsSC-Demo_EURUSD_M1_2016_present.metadata.csv")
                    .display()
                    .to_string(),
            ),
            m1_source_timezone: None,
            broker_path: root.join("EURUSD.broker.json").display().to_string(),
            draws_per_cell: 100,
            seed: 42,
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
            commission_per_lot_round_turn: 7.0,
            slippage_points_per_side: 0.0,
            fallback_spread_points: None,
            validation_fraction: 0.2,
            sealed_fraction: 1.0 / 3.0,
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 2,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 1,
            history_start_year: Some(2016),
        };
        let report = run_timeframe_ablation(TimeframeAblationRequest {
            base: request,
            h1_gates: None,
            h4_gates: Some(TimeframeGateConfig {
                minimum_trades: 20,
                minimum_return_percent: 0.0,
                minimum_profit_factor: 1.0,
                maximum_drawdown_percent: 25.0,
                oos1_retention: 0.7,
            }),
        })
        .expect("real pack timeframe ablation should run");
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert_eq!(report.rows.len(), 26);
        assert_eq!(report.comparisons.len(), 13);
        assert!(
            report
                .comparisons
                .iter()
                .all(|comparison| comparison.paired_comparisons == 100)
        );
    }

    #[test]
    fn real_pack_timeframe_walk_forward_selection_smoke_when_requested() {
        let Ok(root) = std::env::var("QF_TIMEFRAME_WALK_FORWARD_PACK") else {
            return;
        };
        let root = Path::new(&root);
        let symbol = std::env::var("QF_TIMEFRAME_WALK_FORWARD_SYMBOL")
            .unwrap_or_else(|_| "EURUSD".into())
            .to_ascii_uppercase();
        let seed = std::env::var("QF_TIMEFRAME_WALK_FORWARD_SEED")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(42);
        let request = TimeframeBakeoffRequest {
            data_path: root
                .join(format!("ICMarketsSC-Demo_{symbol}_H1_2016_present.tsv"))
                .display()
                .to_string(),
            metadata_path: Some(
                root.join(format!(
                    "ICMarketsSC-Demo_{symbol}_H1_2016_present.metadata.csv"
                ))
                .display()
                .to_string(),
            ),
            source_timezone: None,
            m1_data_path: root
                .join(format!("ICMarketsSC-Demo_{symbol}_M1_2016_present.tsv"))
                .display()
                .to_string(),
            m1_metadata_path: Some(
                root.join(format!(
                    "ICMarketsSC-Demo_{symbol}_M1_2016_present.metadata.csv"
                ))
                .display()
                .to_string(),
            ),
            m1_source_timezone: None,
            broker_path: root
                .join(format!("{symbol}.broker.json"))
                .display()
                .to_string(),
            draws_per_cell: 100,
            seed,
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
            commission_per_lot_round_turn: 7.0,
            slippage_points_per_side: 0.0,
            fallback_spread_points: None,
            validation_fraction: 0.2,
            sealed_fraction: 1.0 / 3.0,
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 2,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 1,
            history_start_year: Some(2016),
        };
        let data = load_timeframe_comparison_data(&request).expect("real pack should load");
        let shared_gates = timeframe_gate_from_request(&request);
        let h4_gates = TimeframeGateConfig {
            minimum_trades: 20,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 25.0,
            oos1_retention: 0.7,
        };
        let config = TimeframeAblationConfig {
            seed: request.seed,
            draws_per_cell: request.draws_per_cell,
            entry_condition_counts: vec![2],
            exit_condition_counts: vec![1],
            scout: timeframe_scout(&request),
            h1_gates: shared_gates.clone(),
            h4_gates: h4_gates.clone(),
            shared_gates,
        };
        let h1_start = data
            .h1_is
            .bars
            .first()
            .expect("H1 development bars")
            .timestamp_ms;
        let h1_len = data.h1_is.bars.len();
        let origins = [
            ("origin_1", 0.45_f64, 0.60_f64),
            ("origin_2", 0.55_f64, 0.70_f64),
            ("origin_3", 0.65_f64, 0.80_f64),
        ];
        let mut summary = Vec::new();
        for (label, train_fraction, validation_end_fraction) in origins {
            let train_end_index = (h1_len as f64 * train_fraction).round() as usize;
            let validation_end_index = (h1_len as f64 * validation_end_fraction).round() as usize;
            let validation_start = data.h1_is.bars[train_end_index].timestamp_ms;
            let validation_end = data.h1_is.bars[validation_end_index].timestamp_ms;
            let h1_train = slice_timestamp_window(&data.h1_is, h1_start, validation_start);
            let h1_validation =
                slice_timestamp_window(&data.h1_is, validation_start, validation_end);
            let h4_train = slice_timestamp_window(&data.h4_is, h1_start, validation_start);
            let h4_validation =
                slice_timestamp_window(&data.h4_is, validation_start, validation_end);
            let report = evolve_timeframe_ablation(
                &h1_train,
                &h1_validation,
                &h4_train,
                &h4_validation,
                &data.broker,
                config.clone(),
            )
            .expect("inner walk-forward ablation should run");
            let row = |mode: quantforge_discover::TimeframeSelectionMode, timeframe: &str| {
                report
                    .rows
                    .iter()
                    .find(|row| row.selection_mode == mode && row.timeframe == timeframe)
                    .expect("walk-forward row exists")
            };
            let h4_drawdown = row(
                quantforge_discover::TimeframeSelectionMode::DrawdownOnly,
                "H4",
            );
            let h4_drawdown_top_k = row(
                quantforge_discover::TimeframeSelectionMode::DrawdownTopK,
                "H4",
            );
            let h4_fold = row(
                quantforge_discover::TimeframeSelectionMode::MedianFoldExpectancyTopK,
                "H4",
            );
            let h4_expectancy = row(
                quantforge_discover::TimeframeSelectionMode::ExpectancyTopK,
                "H4",
            );
            let h4_shared = row(
                quantforge_discover::TimeframeSelectionMode::SharedGates,
                "H4",
            );
            let h4_random = row(
                quantforge_discover::TimeframeSelectionMode::RandomTopK,
                "H4",
            );
            summary.push(json!({
                "seed": request.seed,
                "origin": label,
                "trainBarsH1": h1_train.bars.len(),
                "validationBarsH1": h1_validation.bars.len(),
                "h4SharedLiftR": h4_shared.selected_future_expectancy_lift_r,
                "h4DrawdownLiftR": h4_drawdown.selected_future_expectancy_lift_r,
                "h4DrawdownTopKLiftR": h4_drawdown_top_k.selected_future_expectancy_lift_r,
                "h4FoldExpectancyTopKLiftR": h4_fold.selected_future_expectancy_lift_r,
                "h4ExpectancyTopKLiftR": h4_expectancy.selected_future_expectancy_lift_r,
                "h4RandomTopKLiftR": h4_random.selected_future_expectancy_lift_r,
                "h4DrawdownSelected": h4_drawdown.selected,
                "h4DrawdownTopKSelected": h4_drawdown_top_k.selected,
                "h4FoldSelected": h4_fold.selected,
                "h4RandomTopKSelected": h4_random.selected,
            }));
        }
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
        assert_eq!(summary.len(), 3);
    }

    #[test]
    fn real_pack_timeframe_sealed_evaluation_smoke_when_requested() {
        let Ok(root) = std::env::var("QF_TIMEFRAME_SEALED_EVAL_PACK") else {
            return;
        };
        let root = Path::new(&root);
        let symbol = std::env::var("QF_TIMEFRAME_SEALED_EVAL_SYMBOL")
            .unwrap_or_else(|_| "EURUSD".into())
            .to_ascii_uppercase();
        let seed = std::env::var("QF_TIMEFRAME_SEALED_EVAL_SEED")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(42);
        let request = TimeframeBakeoffRequest {
            data_path: root
                .join(format!("ICMarketsSC-Demo_{symbol}_H1_2016_present.tsv"))
                .display()
                .to_string(),
            metadata_path: Some(
                root.join(format!(
                    "ICMarketsSC-Demo_{symbol}_H1_2016_present.metadata.csv"
                ))
                .display()
                .to_string(),
            ),
            source_timezone: None,
            m1_data_path: root
                .join(format!("ICMarketsSC-Demo_{symbol}_M1_2016_present.tsv"))
                .display()
                .to_string(),
            m1_metadata_path: Some(
                root.join(format!(
                    "ICMarketsSC-Demo_{symbol}_M1_2016_present.metadata.csv"
                ))
                .display()
                .to_string(),
            ),
            m1_source_timezone: None,
            broker_path: root
                .join(format!("{symbol}.broker.json"))
                .display()
                .to_string(),
            draws_per_cell: 100,
            seed,
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
            commission_per_lot_round_turn: 7.0,
            slippage_points_per_side: 0.0,
            fallback_spread_points: None,
            validation_fraction: 0.2,
            sealed_fraction: 1.0 / 3.0,
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 2,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 1,
            history_start_year: Some(2016),
        };
        let data = load_timeframe_comparison_data(&request).expect("real pack should load");
        let shared_gates = timeframe_gate_from_request(&request);
        let h4_gates = TimeframeGateConfig {
            minimum_trades: 20,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 25.0,
            oos1_retention: 0.7,
        };
        let config = TimeframeAblationConfig {
            seed: request.seed,
            draws_per_cell: request.draws_per_cell,
            entry_condition_counts: vec![2],
            exit_condition_counts: vec![1],
            scout: timeframe_scout(&request),
            h1_gates: shared_gates.clone(),
            h4_gates,
            shared_gates,
        };
        let report = evolve_timeframe_ablation(
            &data.h1_is,
            &data.h1_sealed,
            &data.h4_is,
            &data.h4_sealed,
            &data.broker,
            config,
        )
        .expect("sealed evaluation should run");
        let row = |mode: quantforge_discover::TimeframeSelectionMode| {
            report
                .rows
                .iter()
                .find(|row| row.selection_mode == mode && row.timeframe == "H4")
                .expect("sealed H4 row exists")
        };
        let shared = row(quantforge_discover::TimeframeSelectionMode::SharedGates);
        let drawdown = row(quantforge_discover::TimeframeSelectionMode::DrawdownTopK);
        let random = row(quantforge_discover::TimeframeSelectionMode::RandomTopK);
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "symbol": symbol,
                "seed": request.seed,
                "sealedBarsH1": data.h1_sealed.bars.len(),
                "sealedBarsH4": data.h4_sealed.bars.len(),
                "sharedSelected": shared.selected,
                "sharedSelectedSealedExpectancyR": shared.selected_oos1_expectancy_r,
                "sharedUnselectedSealedExpectancyR": shared.unselected_oos1_expectancy_r,
                "sharedSealedLiftR": shared.selected_future_expectancy_lift_r,
                "drawdownTopKSelected": drawdown.selected,
                "drawdownTopKSelectedSealedExpectancyR": drawdown.selected_oos1_expectancy_r,
                "drawdownTopKUnselectedSealedExpectancyR": drawdown.unselected_oos1_expectancy_r,
                "drawdownTopKSealedLiftR": drawdown.selected_future_expectancy_lift_r,
                "randomTopKSelected": random.selected,
                "randomTopKSelectedSealedExpectancyR": random.selected_oos1_expectancy_r,
                "randomTopKUnselectedSealedExpectancyR": random.unselected_oos1_expectancy_r,
                "randomTopKSealedLiftR": random.selected_future_expectancy_lift_r,
            }))
            .unwrap()
        );
        assert!(drawdown.selected <= shared.selected);
        assert!(random.selected <= shared.selected);
    }

    #[test]
    fn real_pack_timeframe_rolling_benchmark_smoke_when_requested() {
        let Ok(root) = std::env::var("QF_TIMEFRAME_ROLLING_PACK") else {
            return;
        };
        let root = Path::new(&root);
        let symbol = std::env::var("QF_TIMEFRAME_ROLLING_SYMBOL")
            .unwrap_or_else(|_| "EURUSD".into())
            .to_ascii_uppercase();
        let seed = std::env::var("QF_TIMEFRAME_ROLLING_SEED")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(7);
        let draws_per_cell = std::env::var("QF_TIMEFRAME_ROLLING_DRAWS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(50);
        let request = TimeframeBakeoffRequest {
            data_path: root
                .join(format!("ICMarketsSC-Demo_{symbol}_H1_2016_present.tsv"))
                .display()
                .to_string(),
            metadata_path: Some(
                root.join(format!(
                    "ICMarketsSC-Demo_{symbol}_H1_2016_present.metadata.csv"
                ))
                .display()
                .to_string(),
            ),
            source_timezone: None,
            m1_data_path: root
                .join(format!("ICMarketsSC-Demo_{symbol}_M1_2016_present.tsv"))
                .display()
                .to_string(),
            m1_metadata_path: Some(
                root.join(format!(
                    "ICMarketsSC-Demo_{symbol}_M1_2016_present.metadata.csv"
                ))
                .display()
                .to_string(),
            ),
            m1_source_timezone: None,
            broker_path: root
                .join(format!("{symbol}.broker.json"))
                .display()
                .to_string(),
            draws_per_cell,
            seed,
            minimum_trades: 10,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            maximum_drawdown_percent: 40.0,
            oos1_retention: 0.7,
            commission_per_lot_round_turn: 7.0,
            slippage_points_per_side: 0.0,
            fallback_spread_points: None,
            validation_fraction: 0.2,
            sealed_fraction: 1.0 / 3.0,
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 2,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 1,
            history_start_year: Some(2016),
        };
        let data = load_timeframe_comparison_data(&request).expect("real pack should load");
        let shared_gates = timeframe_gate_from_request(&request);
        let config = TimeframeAblationConfig {
            seed: request.seed,
            draws_per_cell: request.draws_per_cell,
            entry_condition_counts: vec![2],
            exit_condition_counts: vec![1],
            scout: timeframe_scout(&request),
            h1_gates: shared_gates.clone(),
            h4_gates: TimeframeGateConfig {
                minimum_trades: 20,
                minimum_return_percent: 0.0,
                minimum_profit_factor: 1.0,
                maximum_drawdown_percent: 25.0,
                oos1_retention: 0.7,
            },
            shared_gates,
        };
        let h1_start = data
            .h1_is
            .bars
            .first()
            .expect("development bars")
            .timestamp_ms;
        let h1_len = data.h1_is.bars.len();
        let origins = [
            ("origin_1", 0.25_f64),
            ("origin_2", 0.35_f64),
            ("origin_3", 0.45_f64),
            ("origin_4", 0.55_f64),
            ("origin_5", 0.65_f64),
        ];
        let horizons = [("3m", 3_u32), ("6m", 6_u32), ("12m", 12_u32)];
        let mut all_rows = Vec::new();
        for (origin, train_fraction) in origins {
            let train_end_index = (h1_len as f64 * train_fraction).round() as usize;
            let validation_start = data.h1_is.bars[train_end_index].timestamp_ms;
            let h4_train = slice_timestamp_window(&data.h4_is, h1_start, validation_start);
            let mut validation_sets = Vec::new();
            for (horizon, horizon_months) in horizons {
                let validation_end_timestamp =
                    add_calendar_months(validation_start, horizon_months);
                let validation_end_index = data
                    .h1_is
                    .bars
                    .partition_point(|bar| bar.timestamp_ms < validation_end_timestamp);
                assert!(
                    validation_end_index > train_end_index,
                    "rolling validation window must contain bars"
                );
                let validation_end = if validation_end_index < h1_len {
                    data.h1_is.bars[validation_end_index].timestamp_ms
                } else {
                    data.h1_is.bars.last().unwrap().timestamp_ms + 1
                };
                let h1_validation =
                    slice_timestamp_window(&data.h1_is, validation_start, validation_end);
                let h4_validation =
                    slice_timestamp_window(&data.h4_is, validation_start, validation_end);
                validation_sets.push((format!("{origin}_{horizon}"), h1_validation, h4_validation));
            }
            let windows = validation_sets
                .iter()
                .map(|(label, h1, h4)| TimeframeRollingWindow {
                    label: label.clone(),
                    h1,
                    h4,
                })
                .collect::<Vec<_>>();
            let report = evolve_timeframe_rolling_ablation(
                &h4_train,
                &windows,
                &data.broker,
                config.clone(),
            )
            .expect("rolling benchmark should run");
            all_rows.extend(report.rows);
        }

        let modes = [
            quantforge_discover::TimeframeSelectionMode::SharedGates,
            quantforge_discover::TimeframeSelectionMode::RandomTopK,
            quantforge_discover::TimeframeSelectionMode::DrawdownTopK,
            quantforge_discover::TimeframeSelectionMode::RecoveryFactorTopK,
            quantforge_discover::TimeframeSelectionMode::TradesTopK,
            quantforge_discover::TimeframeSelectionMode::ReturnTopK,
            quantforge_discover::TimeframeSelectionMode::ProfitFactorTopK,
            quantforge_discover::TimeframeSelectionMode::SharpeTopK,
            quantforge_discover::TimeframeSelectionMode::ExpectancyTopK,
            quantforge_discover::TimeframeSelectionMode::MedianFoldExpectancyTopK,
            quantforge_discover::TimeframeSelectionMode::ExpectancyTimesTradesTopK,
            quantforge_discover::TimeframeSelectionMode::ExpectancyTimesSqrtTradesTopK,
        ];
        let mean = |values: Vec<f64>| {
            (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
        };
        let mut horizon_summary = Vec::new();
        for (horizon, _) in horizons {
            let mut mode_summary = Vec::new();
            for mode in modes {
                let rows = all_rows
                    .iter()
                    .filter(|row| row.window.ends_with(horizon) && row.selection_mode == mode)
                    .collect::<Vec<_>>();
                let lifts = rows
                    .iter()
                    .filter_map(|row| row.selected_future_expectancy_lift_r)
                    .collect::<Vec<_>>();
                let selected_future = rows
                    .iter()
                    .filter_map(|row| row.selected_future_expectancy_r)
                    .collect::<Vec<_>>();
                mode_summary.push(json!({
                    "mode": format!("{mode:?}"),
                    "windows": rows.len(),
                    "meanEligible": mean(rows.iter().map(|row| row.eligible as f64).collect()),
                    "meanLiftR": mean(lifts.clone()),
                    "positiveLiftWindows": lifts.iter().filter(|value| **value > 0.0).count(),
                    "minimumLiftR": lifts.iter().copied().reduce(f64::min),
                    "meanSelectedFutureExpectancyR": mean(selected_future),
                    "meanSelectedFutureTrades": mean(rows.iter().filter_map(|row| row.selected_future_trade_count).collect()),
                }));
            }
            let dd_rows = all_rows
                .iter()
                .filter(|row| {
                    row.window.ends_with(horizon)
                        && row.selection_mode
                            == quantforge_discover::TimeframeSelectionMode::DrawdownTopK
                })
                .collect::<Vec<_>>();
            let random_rows = all_rows
                .iter()
                .filter(|row| {
                    row.window.ends_with(horizon)
                        && row.selection_mode
                            == quantforge_discover::TimeframeSelectionMode::RandomTopK
                })
                .collect::<Vec<_>>();
            let dd_beats_random = dd_rows
                .iter()
                .zip(random_rows.iter())
                .filter(|(dd, random)| {
                    dd.selected_future_expectancy_lift_r
                        .unwrap_or(f64::NEG_INFINITY)
                        > random
                            .selected_future_expectancy_lift_r
                            .unwrap_or(f64::NEG_INFINITY)
                })
                .count();
            horizon_summary.push(json!({
                "horizon": horizon,
                "ddBeatsRandomWindows": dd_beats_random,
                "totalWindows": dd_rows.len(),
                "modes": mode_summary,
            }));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "symbol": symbol,
                "seed": request.seed,
                "drawsPerCell": request.draws_per_cell,
                "origins": origins.len(),
                "horizons": horizons.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
                "summary": horizon_summary,
            }))
            .unwrap()
        );
        assert_eq!(all_rows.len(), origins.len() * horizons.len() * modes.len());
    }

    fn add_calendar_months(timestamp_ms: i64, months: u32) -> i64 {
        use chrono::{Datelike, NaiveDate, TimeZone, Timelike, Utc};
        let timestamp = Utc
            .timestamp_millis_opt(timestamp_ms)
            .single()
            .expect("valid rolling timestamp");
        let total_month = timestamp.year() * 12 + timestamp.month0() as i32 + months as i32;
        let year = total_month.div_euclid(12);
        let month = total_month.rem_euclid(12) as u32 + 1;
        let next_month = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap()
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap()
        };
        let day = timestamp.day().min(next_month.pred_opt().unwrap().day());
        let date = NaiveDate::from_ymd_opt(year, month, day).unwrap();
        Utc.from_utc_datetime(
            &date
                .and_hms_milli_opt(
                    timestamp.hour(),
                    timestamp.minute(),
                    timestamp.second(),
                    timestamp.timestamp_subsec_millis(),
                )
                .unwrap(),
        )
        .timestamp_millis()
    }

    fn slice_timestamp_window(dataset: &BarDataset, start_ms: i64, end_ms: i64) -> BarDataset {
        let bars = dataset
            .bars
            .iter()
            .filter(|bar| bar.timestamp_ms >= start_ms && bar.timestamp_ms < end_ms)
            .cloned()
            .collect::<Vec<_>>();
        assert!(bars.len() >= 2, "walk-forward window must contain bars");
        BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: dataset.delimiter,
            source_timezone: dataset.source_timezone.clone(),
            bars,
        }
    }
}
