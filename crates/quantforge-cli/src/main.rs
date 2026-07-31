use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand, ValueEnum};
use quantforge_broker::{DayOfWeek, SymbolSpecification};
use quantforge_data::{
    BarDataset, DataQualityReport, Mt5ExportMetadata, QualityGrade, SourceTimezone,
    bar_content_hash, build_timeframe_from_m1, infer_median_interval_ms,
};
use quantforge_discover::{
    ConditionBakeoffConfig, Databank, DiscoverConfig, DiscoverRunMode, GateConfig,
    MethodologyGridConfig, PermutationNullConfig, UniversalGrammarConfig, continue_evolution,
    evolve_new, run_condition_bakeoff, run_methodology_grid, run_permutation_null,
};
use quantforge_eval::{CostModel, ScoutConfig, ScoutResult, evaluate_strategy};
use quantforge_export_mql5::{
    ExportEvidenceCard, ExportStyle, MetaEditorConfig, Mql5ExportConfig, TerminalConfig,
    TesterConfig, TesterRunReport, compile_with_metaeditor, generate_bundle, run_mt5_tester,
};
use quantforge_ir::StrategyIr;
use quantforge_parity::{
    DiffReport, IndicatorParityConfig, IndicatorParityReport, Mt5TesterMetadata, ParityRun,
    ParityTolerances, compare_indicator_reference, compare_runs, load_mt5_tester_metadata,
    load_mt5_tester_run_in_timezone,
};
use quantforge_portfolio::{
    PortfolioCandidate, PortfolioConfig, PortfolioObjective, PortfolioReport, pack_portfolio,
};
use quantforge_quality::{
    BoundGateEvidence, CERTIFICATION_SCHEMA_VERSION, CHALLENGE_PROTOCOL, CertificationEvidence,
    CertificationPolicy, ChallengeConfig, ChallengeReport, DataGateEvidence, DataSplitPlan,
    EVIDENCE_PROTOCOL_VERSION, EXTERNAL_PARITY_PROTOCOL, EvidenceBinding, ExternalEngine,
    ExternalParityEvidence, ILLUMINATION_PROTOCOL, INCUBATION_PROTOCOL, INDICATOR_PARITY_PROTOCOL,
    IncubationKillRules, IncubationObservation, IncubationReport, IncubationStart, JUDGE_PROTOCOL,
    SEALED_FINAL_PROTOCOL, SealedFinalConfig, SealedFinalEvidence, SealedFinalReport,
    StrategyGrade, VALIDATION_PROTOCOL, ValidationAttestation, evaluate_certification,
    run_challenge, run_incubation, run_sealed_final,
};
use quantforge_storage::{
    CertifiedVaultEntry, RunManifest, RunRecipe, VAULT_SCHEMA_VERSION, admit_certified,
    claim_sealed_access_once, sealed_final_path, write_directory_new, write_json_new,
    write_json_versioned, write_sealed_final_once, write_text_new,
};
use quantforge_tick::{JudgeConfig, JudgeResult, evaluate_strategy_m1};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "quantforge",
    version,
    about = "Systematic strategy research for MetaTrader 5"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Load an MT5 CSV/TSV file and print its deterministic identity.
    LoadCsv(DataSourceArgs),
    /// Analyze gaps, duplicates, malformed OHLC, spikes and weekend bars.
    DataQuality {
        #[command(flatten)]
        source: DataSourceArgs,
        /// Write a versioned JSON report containing its run manifest.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Run the deterministic completed-bar OHLC scout evaluator.
    Scout {
        #[command(flatten)]
        source: DataSourceArgs,
        /// Strategy IR JSON.
        #[arg(long)]
        strategy: PathBuf,
        /// Broker SymbolSpecification JSON.
        #[arg(long)]
        broker: PathBuf,
        /// Explicit account commission assumption.
        #[arg(long)]
        commission_per_lot_round_turn: f64,
        #[arg(long, default_value_t = 0.0)]
        slippage_points_per_side: f64,
        /// Used only when data rows have no spread.
        #[arg(long)]
        fallback_spread_points: Option<f64>,
        #[arg(long)]
        max_spread_points: Option<f64>,
        #[arg(long, default_value_t = 100_000.0)]
        initial_balance: f64,
        /// Broker-local hour from which entries may be placed (inclusive).
        #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_START_HOUR)]
        entry_window_start_hour: u32,
        /// Broker-local hour from which entries stop being placed (exclusive).
        #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_END_HOUR)]
        entry_window_end_hour: u32,
        /// Allow a data-quality Fail and record the override in the manifest.
        #[arg(long)]
        allow_failed_data: bool,
        /// Write a versioned result; otherwise print the manifest-bound result.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Replay an elite's decision-timeframe signals through chronological M1 execution.
    Judge(JudgeArgs),
    /// Generate, evaluate and illuminate a resumable MAP-Elites databank.
    Evolve(EvolveArgs),
    /// Export guarded MQL5, tester settings and an evidence card.
    Export(ExportArgs),
    /// Compare a Scout result with QuantForge CSVs emitted by an MT5 tester run.
    Parity(ParityArgs),
    /// Numerically compare every export-safe Rust indicator with an MT5 probe pack.
    IndicatorParity(IndicatorParityArgs),
    /// Execute a generated tester configuration in the local MT5 terminal.
    Mt5Test(Mt5TestArgs),
    /// Freeze chronological development, validation and sealed-final boundaries.
    SplitPlan(SplitPlanArgs),
    /// Evaluate all promotion evidence and admit only Certified entries to the Vault.
    Certify(CertifyArgs),
    /// Verify real gate artifacts and assemble a no-clobber certification bundle.
    AssembleEvidence(AssembleEvidenceArgs),
    /// Pack low-correlation databank elites under hard exposure caps.
    Portfolio(PortfolioArgs),
    /// Open an immutable paper-trading incubation ledger.
    IncubationStart(IncubationStartArgs),
    /// Append one daily paper-trading observation.
    IncubationRecord(IncubationRecordArgs),
    /// Seal and evaluate an incubation ledger exactly once.
    IncubationFinalize(IncubationFinalizeArgs),
    /// Materialize the exact parity-passed EA from a Certified Vault entry.
    Deploy(DeployArgs),
    /// Run the validation-only robustness battery for an Illuminated candidate.
    Challenge(ChallengeArgs),
    /// Open one shortlisted candidate's sealed partition exactly once.
    SealedFinal(SealedFinalArgs),
    /// Stationary-bootstrap noise floor for Discover gate calibration.
    PermutationNull(PermutationNullArgs),
    /// Short Fast Scout per entry-condition count, ranked by OOS1 retention.
    ConditionBakeoff(ConditionBakeoffArgs),
    /// Factor grid across entry/exit condition counts × recipes → OOS1 retention.
    MethodologyResearch(MethodologyResearchArgs),
}

#[derive(Debug, Args)]
struct PermutationNullArgs {
    #[command(flatten)]
    source: DataSourceArgs,
    /// Broker SymbolSpecification JSON for the primary symbol.
    #[arg(long)]
    broker: PathBuf,
    /// Number of synthetic Discover trials.
    #[arg(long, default_value_t = 8)]
    trials: usize,
    /// Mean stationary-bootstrap block length in M1 bars.
    #[arg(long, default_value_t = 1440)]
    mean_block_length: usize,
    #[arg(long, default_value_t = 7)]
    seed: u64,
    #[arg(long, default_value_t = 200)]
    initial_candidates: usize,
    #[arg(long, default_value_t = 0)]
    generations: u64,
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct ConditionBakeoffArgs {
    #[command(flatten)]
    source: DataSourceArgs,
    #[arg(long)]
    m1: PathBuf,
    #[arg(
        long,
        value_name = "IANA_TIMEZONE",
        required_unless_present = "m1_metadata",
        conflicts_with = "m1_metadata"
    )]
    m1_source_timezone: Option<SourceTimezone>,
    #[arg(
        long,
        value_name = "METADATA_CSV",
        required_unless_present = "m1_source_timezone",
        conflicts_with = "m1_source_timezone"
    )]
    m1_metadata: Option<PathBuf>,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long, default_value_t = 3)]
    generations: u64,
    #[arg(long, default_value_t = 60)]
    initial_candidates: usize,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// Entry-condition counts to compare (each pinned exactly). Default 2,3,4.
    #[arg(long, value_delimiter = ',', default_value = "2,3,4")]
    entry_condition_counts: Vec<usize>,
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    /// Chronological OOS1 fraction.
    #[arg(long, default_value_t = 0.2)]
    validation_fraction: f64,
    /// Chronological OOS2 / sealed fraction.
    #[arg(long, default_value_t = 0.2)]
    sealed_fraction: f64,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct MethodologyResearchArgs {
    #[command(flatten)]
    source: DataSourceArgs,
    #[arg(long)]
    m1: PathBuf,
    #[arg(
        long,
        value_name = "IANA_TIMEZONE",
        required_unless_present = "m1_metadata",
        conflicts_with = "m1_metadata"
    )]
    m1_source_timezone: Option<SourceTimezone>,
    #[arg(
        long,
        value_name = "METADATA_CSV",
        required_unless_present = "m1_source_timezone",
        conflicts_with = "m1_source_timezone"
    )]
    m1_metadata: Option<PathBuf>,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long, default_value_t = 40)]
    draws_per_cell: usize,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    #[arg(long, default_value_t = 0.2)]
    validation_fraction: f64,
    #[arg(long, default_value_t = 0.2)]
    sealed_fraction: f64,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct DataSourceArgs {
    path: PathBuf,
    /// IANA timezone of naive timestamps, for example Europe/Helsinki.
    #[arg(
        long,
        value_name = "IANA_TIMEZONE",
        required_unless_present = "metadata",
        conflicts_with = "metadata"
    )]
    source_timezone: Option<SourceTimezone>,
    /// QuantForge MT5 exporter metadata; supplies and binds the timezone.
    #[arg(
        long,
        value_name = "METADATA_CSV",
        required_unless_present = "source_timezone",
        conflicts_with = "source_timezone"
    )]
    metadata: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct EvolveArgs {
    /// Broker-local hour from which entries may be placed (inclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_START_HOUR)]
    entry_window_start_hour: u32,
    /// Broker-local hour from which entries stop being placed (exclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_END_HOUR)]
    entry_window_end_hour: u32,
    #[command(flatten)]
    source: DataSourceArgs,
    /// M1 execution CSV/TSV used for mandatory higher-precision acceptance.
    #[arg(long)]
    m1: PathBuf,
    #[arg(
        long,
        value_name = "IANA_TIMEZONE",
        required_unless_present = "m1_metadata",
        conflicts_with = "m1_metadata"
    )]
    m1_source_timezone: Option<SourceTimezone>,
    #[arg(
        long,
        value_name = "METADATA_CSV",
        required_unless_present = "m1_source_timezone",
        conflicts_with = "m1_source_timezone"
    )]
    m1_metadata: Option<PathBuf>,
    /// Broker SymbolSpecification JSON.
    #[arg(long)]
    broker: PathBuf,
    /// Databank artifact to create or continue.
    #[arg(long)]
    databank: PathBuf,
    /// Continue the existing databank with its immutable stored configuration.
    #[arg(long = "continue")]
    continue_existing: bool,
    /// Number of generations to run now (additional generations on continue).
    #[arg(long, default_value_t = 50)]
    generations: u64,
    #[arg(long)]
    initial: Option<usize>,
    #[arg(long)]
    batch: Option<usize>,
    #[arg(long)]
    correlation: Option<f64>,
    #[arg(long)]
    novelty_weight: Option<f64>,
    #[arg(long)]
    tournament_size: Option<usize>,
    #[arg(long)]
    structural_mutation_probability: Option<f64>,
    #[arg(long)]
    seed: Option<u64>,
    /// Minimum mirrored entry conditions (2..=4). Default 2.
    #[arg(long)]
    minimum_entry_conditions: Option<usize>,
    /// Maximum mirrored entry conditions (2..=4). Default 4.
    #[arg(long)]
    maximum_entry_conditions: Option<usize>,
    /// Minimum exit conditions (1..=3). Default 1.
    #[arg(long)]
    minimum_exit_conditions: Option<usize>,
    /// Maximum exit conditions (1..=3). Default 3.
    #[arg(long)]
    maximum_exit_conditions: Option<usize>,
    /// fast_scout | full_harvest | quota_harvest | mass_builder
    #[arg(long, default_value = "full_harvest")]
    run_mode: String,
    #[arg(long)]
    minimum_trades: Option<usize>,
    #[arg(long)]
    maximum_drawdown_percent: Option<f64>,
    #[arg(long)]
    minimum_return_percent: Option<f64>,
    #[arg(long)]
    minimum_profit_factor: Option<f64>,
    #[arg(long, alias = "minimum-recovery-factor")]
    minimum_return_drawdown: Option<f64>,
    #[arg(long)]
    minimum_m1_return_retention: Option<f64>,
    /// Size of the ±% jitter applied to every numeric gene when probing the
    /// local plateau, as a fraction (SQX default 0.20).
    #[arg(long)]
    robustness_perturbation_fraction: Option<f64>,
    /// Close positions, cancel pending orders and block entries from end-of-day
    /// until the next broker day.
    #[arg(long)]
    flatten_at_22: bool,
    /// Broker-local hour (0–23) for end-of-day flatten when `--flatten-at-22` is set (default 23).
    #[arg(long, default_value_t = 23)]
    end_of_day_hour: u8,
    /// Required for a new databank; a continuation uses the stored assumption.
    #[arg(long)]
    commission_per_lot_round_turn: Option<f64>,
    #[arg(long)]
    slippage_points_per_side: Option<f64>,
    /// Used only when data rows have no spread.
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    #[arg(long)]
    max_spread_points: Option<f64>,
    #[arg(long)]
    initial_balance: Option<f64>,
    /// Allow a data-quality Fail and record the override in the manifest.
    #[arg(long)]
    allow_failed_data: bool,
    /// Train on IS only and pick elites when OOS1 expectancy retains enough of IS.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    promotion_split: bool,
    #[arg(long, default_value_t = 0.2)]
    validation_fraction: f64,
    #[arg(long, default_value_t = 0.2)]
    sealed_fraction: f64,
}

#[derive(Debug, Args)]
struct JudgeArgs {
    /// Broker-local hour from which entries may be placed (inclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_START_HOUR)]
    entry_window_start_hour: u32,
    /// Broker-local hour from which entries stop being placed (exclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_END_HOUR)]
    entry_window_end_hour: u32,
    #[command(flatten)]
    decision: DataSourceArgs,
    /// M1 execution CSV/TSV covering every decision bar.
    #[arg(long)]
    m1: PathBuf,
    /// IANA timezone of naive M1 timestamps.
    #[arg(
        long,
        value_name = "IANA_TIMEZONE",
        required_unless_present = "m1_metadata",
        conflicts_with = "m1_metadata"
    )]
    m1_source_timezone: Option<SourceTimezone>,
    /// QuantForge MT5 exporter metadata for the M1 file.
    #[arg(
        long,
        value_name = "METADATA_CSV",
        required_unless_present = "m1_source_timezone",
        conflicts_with = "m1_source_timezone"
    )]
    m1_metadata: Option<PathBuf>,
    #[arg(long)]
    strategy: PathBuf,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    #[arg(long)]
    max_spread_points: Option<f64>,
    #[arg(long, default_value_t = 100_000.0)]
    initial_balance: f64,
    /// Allow failed quality on either input and record the override.
    #[arg(long)]
    allow_failed_data: bool,
    /// Research-only override for missing minutes inside a decision bar.
    #[arg(long)]
    allow_execution_gaps: bool,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct ExportArgs {
    /// Broker-local hour from which entries may be placed (inclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_START_HOUR)]
    entry_window_start_hour: u32,
    /// Broker-local hour from which entries stop being placed (exclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_END_HOUR)]
    entry_window_end_hour: u32,
    /// Strategy IR JSON.
    #[arg(long)]
    strategy: PathBuf,
    /// Broker SymbolSpecification JSON.
    #[arg(long)]
    broker: PathBuf,
    /// New directory for the generated export artifacts.
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value = "QuantForgeStrategy")]
    expert_name: String,
    /// MQL5/Experts subdirectory referenced by the tester configuration.
    #[arg(long, default_value = "QuantForge")]
    expert_directory: String,
    #[arg(long, default_value = "M15")]
    timeframe: String,
    #[arg(long, default_value_t = 42_424_242)]
    magic: u64,
    #[arg(long, default_value_t = 10)]
    deviation_points: u32,
    #[arg(long)]
    max_spread_points: Option<f64>,
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    /// Explicit account commission assumption embedded in sizing inputs.
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    #[arg(long)]
    from_date: Option<String>,
    #[arg(long)]
    to_date: Option<String>,
    #[arg(long, default_value_t = 100_000.0)]
    deposit: f64,
    #[arg(long, default_value = "USD")]
    currency: String,
    #[arg(long, default_value_t = 100)]
    leverage: u32,
    #[arg(long, default_value_t = 1)]
    tester_model: u8,
    /// Compile the generated source with MetaEditor after export.
    #[arg(long)]
    compile: bool,
    #[arg(long)]
    metaeditor: Option<PathBuf>,
    #[arg(long)]
    wine: Option<PathBuf>,
    #[arg(long)]
    wine_prefix: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ParityArgs {
    /// Manifest-bound Scout JSON produced by `quantforge scout --out`.
    /// Also accepts Judge JSON from `quantforge judge --out`.
    #[arg(long)]
    scout_result: PathBuf,
    /// Evidence card produced by `quantforge export`.
    #[arg(long)]
    evidence: PathBuf,
    /// Generated MQL5 source bound by the evidence card.
    #[arg(long)]
    mq5: PathBuf,
    /// Tester deal CSV emitted by the generated EA.
    #[arg(long)]
    mt5_deals: PathBuf,
    /// Tester equity CSV emitted by the generated EA.
    #[arg(long)]
    mt5_equity: PathBuf,
    /// Tester metadata CSV emitted by the generated EA.
    #[arg(long)]
    mt5_metadata: PathBuf,
    /// Broker timezone used to localize MT5 DEAL_TIME_MSC into UTC
    /// (same token as bar ingestion, e.g. `ICMarkets/EST+7`).
    #[arg(long)]
    broker_timezone: Option<String>,
    #[arg(long, default_value_t = 100_000.0)]
    initial_balance: f64,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 0.10)]
    trade_count_relative: f64,
    #[arg(long, default_value_t = 3)]
    trade_count_absolute: usize,
    #[arg(long, default_value_t = 0.15)]
    net_profit_relative: f64,
    #[arg(long, default_value_t = 0.15)]
    max_drawdown_relative: f64,
    #[arg(long, default_value_t = 5.0)]
    max_equity_divergence_percent: f64,
    #[arg(long, default_value_t = 0)]
    trade_timestamp_tolerance_ms: i64,
    #[arg(long, default_value_t = 0.90)]
    minimum_aligned_trade_fraction: f64,
}

#[derive(Debug, Args)]
struct Mt5TestArgs {
    #[arg(long)]
    tester_ini: PathBuf,
    #[arg(long)]
    evidence: PathBuf,
    #[arg(long)]
    out: PathBuf,
    #[arg(long)]
    terminal: Option<PathBuf>,
    #[arg(long)]
    wine: Option<PathBuf>,
    #[arg(long)]
    wine_prefix: Option<PathBuf>,
    #[arg(long)]
    common_files: Option<PathBuf>,
    #[arg(long, default_value_t = 1_800)]
    timeout_seconds: u64,
}

#[derive(Debug, Args)]
struct IndicatorParityArgs {
    /// CSV emitted by QuantForgeIndicatorParityProbeEA.
    #[arg(long)]
    reference: PathBuf,
    /// New manifest-bound JSON report.
    #[arg(long)]
    out: PathBuf,
    /// Oldest rows ignored while recursive MT5 and Rust buffers converge.
    #[arg(long, default_value_t = 1_000)]
    warmup_rows: usize,
    #[arg(long, default_value_t = 1.0e-10)]
    absolute_epsilon: f64,
    #[arg(long, default_value_t = 1.0e-9)]
    relative_epsilon: f64,
}

#[derive(Debug, Args)]
struct SplitPlanArgs {
    #[command(flatten)]
    source: DataSourceArgs,
    #[arg(long, default_value_t = 0.2)]
    validation_fraction: f64,
    #[arg(long, default_value_t = 0.2)]
    sealed_fraction: f64,
    /// Permit a research-only split plan for failed data. Such a plan cannot be Certified.
    #[arg(long)]
    allow_failed_data: bool,
    /// New immutable split-plan artifact.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct CertifyArgs {
    #[arg(long)]
    strategy: PathBuf,
    #[arg(long)]
    broker: PathBuf,
    /// Immutable artifact produced by `quantforge split-plan`.
    #[arg(long)]
    split_plan: PathBuf,
    /// CertificationEvidence JSON whose claims bind the supplied artifacts.
    #[arg(long, required_unless_present = "bundle", conflicts_with = "bundle")]
    evidence: Option<PathBuf>,
    /// Supply every gate artifact referenced by its SHA-256 hash in the evidence file.
    #[arg(
        long = "artifact",
        value_name = "EVIDENCE_FILE",
        requires = "evidence",
        conflicts_with = "bundle"
    )]
    artifacts: Vec<PathBuf>,
    /// Bundle produced by `quantforge assemble-evidence`; replaces --evidence and --artifact.
    #[arg(long, conflicts_with_all = ["evidence", "artifacts"])]
    bundle: Option<PathBuf>,
    /// Root directory of the immutable Certified-only Vault.
    #[arg(long)]
    vault: PathBuf,
    /// Require a passing incubation record before certification.
    #[arg(long)]
    require_incubation: bool,
    #[arg(long, default_value_t = 1_500)]
    selection_bias_warning_threshold: u64,
}

#[derive(Debug, Args)]
struct AssembleEvidenceArgs {
    #[arg(long)]
    strategy: PathBuf,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long)]
    split_plan: PathBuf,
    /// Promotion-grade MAP-Elites artifact containing the exact candidate.
    #[arg(long)]
    databank: PathBuf,
    #[arg(long)]
    challenge: PathBuf,
    #[arg(long)]
    judge: PathBuf,
    /// External MT5 Strategy Tester parity artifact.
    #[arg(long)]
    parity: PathBuf,
    #[arg(long)]
    indicator_parity: PathBuf,
    #[arg(long)]
    sealed_final: PathBuf,
    /// Passing final artifact from `quantforge incubation-finalize`.
    #[arg(long)]
    incubation: Option<PathBuf>,
    /// New directory containing the validation attestation, evidence and bundle.
    #[arg(long)]
    out_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PortfolioObjectiveArg {
    RiskAdjustedReturn,
    Cvar,
    MinimizeDrawdown,
}

impl From<PortfolioObjectiveArg> for PortfolioObjective {
    fn from(value: PortfolioObjectiveArg) -> Self {
        match value {
            PortfolioObjectiveArg::RiskAdjustedReturn => Self::RiskAdjustedReturn,
            PortfolioObjectiveArg::Cvar => Self::Cvar,
            PortfolioObjectiveArg::MinimizeDrawdown => Self::MinimizeDrawdown,
        }
    }
}

#[derive(Debug, Args)]
struct PortfolioArgs {
    /// Promotion-grade MAP-Elites artifact.
    databank: PathBuf,
    /// Broker profile bound to the databank; supplies the symbol identity.
    #[arg(long)]
    broker: PathBuf,
    #[arg(long, value_enum, default_value = "risk-adjusted-return")]
    objective: PortfolioObjectiveArg,
    #[arg(long, default_value_t = 0.70)]
    maximum_pairwise_correlation: f64,
    #[arg(long, default_value_t = 0.25)]
    maximum_weight_per_strategy: f64,
    #[arg(long, default_value_t = 1.0)]
    maximum_symbol_exposure: f64,
    /// Cap on exposure to any one strategy cohort (family).
    #[arg(long, alias = "maximum-family-exposure", default_value_t = 0.50)]
    maximum_cohort_exposure: f64,
    #[arg(long, default_value_t = 10)]
    maximum_strategies: usize,
    #[arg(long, default_value_t = 0.0)]
    minimum_return_percent: f64,
    #[arg(long, default_value_t = 0.05)]
    cvar_tail_fraction: f64,
    #[arg(long, default_value_t = 1_000)]
    stress_trials: usize,
    #[arg(long, default_value_t = 5)]
    stress_block_length: usize,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    /// New immutable portfolio report.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct DeployArgs {
    /// Certified Vault entry produced by `quantforge certify`.
    #[arg(long)]
    vault_entry: PathBuf,
    /// New immutable MT5 deployment-pack directory.
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct IncubationStartArgs {
    #[arg(long)]
    strategy: PathBuf,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long)]
    split_plan: PathBuf,
    /// Root of the append-only incubation store.
    #[arg(long)]
    root: PathBuf,
    #[arg(long)]
    start_date: NaiveDate,
    #[arg(long)]
    initial_balance: f64,
    #[arg(long, default_value_t = 2.0)]
    maximum_daily_loss_percent: f64,
    #[arg(long, default_value_t = 10.0)]
    maximum_total_drawdown_percent: f64,
    #[arg(long, default_value_t = 30)]
    minimum_observation_days: usize,
    #[arg(long, default_value_t = 20)]
    minimum_total_trades: usize,
    #[arg(long, default_value_t = 5)]
    maximum_consecutive_zero_trade_days: usize,
}

#[derive(Debug, Args)]
struct IncubationRecordArgs {
    /// The immutable `incubation-start.json` artifact.
    #[arg(long)]
    start: PathBuf,
    #[arg(long)]
    date: NaiveDate,
    #[arg(long)]
    ending_balance: f64,
    #[arg(long)]
    maximum_drawdown_percent: f64,
    #[arg(long)]
    trade_count: usize,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Debug, Args)]
struct IncubationFinalizeArgs {
    /// The immutable `incubation-start.json` artifact.
    #[arg(long)]
    start: PathBuf,
}

#[derive(Debug, Args)]
struct ChallengeArgs {
    /// Broker-local hour from which entries may be placed (inclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_START_HOUR)]
    entry_window_start_hour: u32,
    /// Broker-local hour from which entries stop being placed (exclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_END_HOUR)]
    entry_window_end_hour: u32,
    #[command(flatten)]
    source: DataSourceArgs,
    #[arg(long)]
    strategy: PathBuf,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long)]
    split_plan: PathBuf,
    /// Total candidates evaluated in the originating research/databank run.
    #[arg(long)]
    evaluations_touched: u64,
    #[arg(long, default_value_t = 5)]
    folds: usize,
    #[arg(long, default_value_t = 20)]
    purge_bars: usize,
    #[arg(long, default_value_t = 20)]
    embargo_bars: usize,
    #[arg(long, default_value_t = 250)]
    minimum_validation_bars: usize,
    #[arg(long, default_value_t = 20)]
    minimum_baseline_trades: usize,
    #[arg(long, default_value_t = 3)]
    minimum_fold_trades: usize,
    #[arg(long, default_value_t = 0.0)]
    minimum_return_percent: f64,
    #[arg(long, default_value_t = 1.0)]
    minimum_profit_factor: f64,
    #[arg(long, default_value_t = 30.0)]
    maximum_drawdown_percent: f64,
    #[arg(long, default_value_t = 0.6)]
    minimum_passing_fold_fraction: f64,
    #[arg(long, value_delimiter = ',', default_value = "1.0,1.25,1.5,2.0")]
    cost_multipliers: Vec<f64>,
    #[arg(long, default_value_t = 0.75)]
    minimum_cost_survival_fraction: f64,
    #[arg(long, default_value_t = 1_000)]
    monte_carlo_trials: usize,
    #[arg(long, default_value_t = 5)]
    monte_carlo_block_length: usize,
    #[arg(long, default_value_t = 0.0)]
    monte_carlo_minimum_p05_net_profit: f64,
    #[arg(long, default_value_t = 35.0)]
    monte_carlo_maximum_p95_drawdown_percent: f64,
    #[arg(long, default_value_t = 20)]
    neighborhood_samples: usize,
    #[arg(long, default_value_t = 0.1)]
    parameter_perturbation_fraction: f64,
    #[arg(long, default_value_t = 0.7)]
    minimum_neighborhood_survival_fraction: f64,
    #[arg(long, default_value_t = 0.5)]
    minimum_neighborhood_return_ratio: f64,
    #[arg(long, default_value_t = 1.5)]
    maximum_neighborhood_drawdown_ratio: f64,
    #[arg(long, default_value_t = 0.5)]
    minimum_neighborhood_trade_ratio: f64,
    #[arg(long)]
    minimum_deflated_trade_sharpe: Option<f64>,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    #[arg(long)]
    max_spread_points: Option<f64>,
    #[arg(long, default_value_t = 100_000.0)]
    initial_balance: f64,
    #[arg(long)]
    out: PathBuf,
}

#[derive(Debug, Args)]
struct SealedFinalArgs {
    /// Broker-local hour from which entries may be placed (inclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_START_HOUR)]
    entry_window_start_hour: u32,
    /// Broker-local hour from which entries stop being placed (exclusive).
    #[arg(long, default_value_t = quantforge_eval::MANDATORY_ENTRY_WINDOW_END_HOUR)]
    entry_window_end_hour: u32,
    #[command(flatten)]
    source: DataSourceArgs,
    #[arg(long)]
    strategy: PathBuf,
    #[arg(long)]
    broker: PathBuf,
    #[arg(long)]
    split_plan: PathBuf,
    /// Passing machine-readable Challenge artifact proving prior shortlisting.
    #[arg(long)]
    challenge: PathBuf,
    /// Root of the no-clobber sealed-attempt ledger.
    #[arg(long)]
    sealed_root: PathBuf,
    #[arg(long, default_value_t = 20)]
    minimum_trades: usize,
    #[arg(long, default_value_t = 1.0)]
    minimum_return_percent: f64,
    #[arg(long, default_value_t = 1.1)]
    minimum_profit_factor: f64,
    #[arg(long, default_value_t = 20.0)]
    maximum_drawdown_percent: f64,
    /// Must exactly match the Challenge cost configuration.
    #[arg(long)]
    commission_per_lot_round_turn: f64,
    /// Must exactly match the Challenge cost configuration.
    #[arg(long, default_value_t = 0.0)]
    slippage_points_per_side: f64,
    /// Must exactly match the Challenge cost configuration.
    #[arg(long)]
    fallback_spread_points: Option<f64>,
    /// Must exactly match the Challenge cost configuration.
    #[arg(long)]
    max_spread_points: Option<f64>,
    /// Must exactly match the Challenge balance configuration.
    #[arg(long, default_value_t = 100_000.0)]
    initial_balance: f64,
}

#[derive(Debug, Serialize)]
struct LoadSummary<'a> {
    source: &'a Path,
    data_hash: &'a quantforge_core::ContentHash,
    source_rows: usize,
    bars: usize,
    duplicate_rows_removed: usize,
    input_was_sorted: bool,
    delimiter: char,
    source_timezone: &'a str,
    metadata_hash: Option<&'a quantforge_core::ContentHash>,
    broker: Option<&'a str>,
    server: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct QualityArtifact<'a> {
    manifest: RunManifest,
    source: &'a Path,
    data_hash: &'a quantforge_core::ContentHash,
    metadata_hash: Option<&'a quantforge_core::ContentHash>,
    source_timezone: &'a str,
    report: &'a DataQualityReport,
}

#[derive(Debug, Serialize)]
struct ScoutArtifact<'a> {
    manifest: RunManifest,
    strategy_fingerprint: quantforge_core::ContentHash,
    source: &'a Path,
    strategy: &'a Path,
    broker: &'a Path,
    metadata_hash: Option<&'a quantforge_core::ContentHash>,
    data_quality: &'a DataQualityReport,
    result: &'a ScoutResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EvolveArtifact {
    manifest: RunManifest,
    source: String,
    broker: String,
    metadata_hash: Option<quantforge_core::ContentHash>,
    data_quality: DataQualityReport,
    coverage: usize,
    qd_score: f64,
    databank: Databank,
}

#[derive(Debug, Deserialize)]
struct ScoutArtifactInput {
    manifest: RunManifest,
    strategy_fingerprint: quantforge_core::ContentHash,
    result: ScoutResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ParityArtifact {
    manifest: RunManifest,
    evidence: ExportEvidenceCard,
    reference: ParityRun,
    external: ParityRun,
    mt5_metadata: Mt5TesterMetadata,
    report: DiffReport,
}

#[derive(Debug, Serialize)]
struct Mt5TestArtifact {
    manifest: RunManifest,
    evidence: ExportEvidenceCard,
    report: TesterRunReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IndicatorParityArtifact {
    manifest: RunManifest,
    report: IndicatorParityReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SplitPlanArtifact {
    manifest: RunManifest,
    source: String,
    metadata_hash: Option<quantforge_core::ContentHash>,
    data_quality: DataQualityReport,
    validation_fraction: f64,
    sealed_fraction: f64,
    plan: DataSplitPlan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ChallengeArtifact {
    manifest: RunManifest,
    source: String,
    metadata_hash: Option<quantforge_core::ContentHash>,
    data_quality: DataQualityReport,
    strategy_source: String,
    broker_source: String,
    split_plan_source: String,
    report: ChallengeReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct SealedFinalArtifact {
    manifest: RunManifest,
    source: String,
    metadata_hash: Option<quantforge_core::ContentHash>,
    data_quality: DataQualityReport,
    strategy_source: String,
    broker_source: String,
    split_plan_source: String,
    challenge_source: String,
    report: SealedFinalReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ValidationArtifact {
    manifest: RunManifest,
    challenge_source: String,
    attestation: ValidationAttestation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IncubationStartArtifact {
    manifest: RunManifest,
    strategy_source: String,
    broker_source: String,
    split_plan_source: String,
    start: IncubationStart,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IncubationObservationArtifact {
    manifest: RunManifest,
    start_source: String,
    start_artifact_hash: quantforge_core::ContentHash,
    observation: IncubationObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct IncubationFinalArtifact {
    manifest: RunManifest,
    start_source: String,
    observation_sources: Vec<String>,
    start: IncubationStart,
    observations: Vec<IncubationObservation>,
    report: IncubationReport,
}

const EVIDENCE_BUNDLE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct EvidenceBundle {
    schema_version: u16,
    manifest: RunManifest,
    evidence_source: String,
    evidence_hash: quantforge_core::ContentHash,
    artifacts: Vec<VaultArtifactReference>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PortfolioArtifact {
    manifest: RunManifest,
    databank_source: String,
    databank_source_hash: quantforge_core::ContentHash,
    broker_source: String,
    report: PortfolioReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct VaultArtifactReference {
    gate: String,
    path: String,
    content_hash: quantforge_core::ContentHash,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct VaultPayload {
    manifest: RunManifest,
    strategy_source: String,
    strategy_source_hash: quantforge_core::ContentHash,
    strategy: StrategyIr,
    broker_source: String,
    broker_source_hash: quantforge_core::ContentHash,
    broker: SymbolSpecification,
    split_plan_source: String,
    split_plan_source_hash: quantforge_core::ContentHash,
    split_plan: DataSplitPlan,
    evidence_source: String,
    evidence_source_hash: quantforge_core::ContentHash,
    evidence: CertificationEvidence,
    artifacts: Vec<VaultArtifactReference>,
}

const DEPLOYMENT_SCHEMA_VERSION: u16 = 1;
const DEPLOYMENT_PROTOCOL_VERSION: &str = "mt5-deployment-pack-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeploymentBrokerLimits {
    volume_min: f64,
    volume_step: f64,
    volume_max: f64,
    stops_level_points: u32,
    freeze_level_points: u32,
    filling_modes: Vec<quantforge_broker::FillingMode>,
    trade_mode: quantforge_broker::TradeMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeploymentRiskPack {
    schema_version: u16,
    protocol_version: String,
    certified_vault_entry_id: quantforge_core::ContentHash,
    certified_vault_entry_hash: quantforge_core::ContentHash,
    certification_evidence_hash: quantforge_core::ContentHash,
    incubation_artifact_hash: quantforge_core::ContentHash,
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
    content_hash: quantforge_core::ContentHash,
    byte_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeploymentManifest {
    schema_version: u16,
    protocol_version: String,
    deployment_id: quantforge_core::ContentHash,
    grade: StrategyGrade,
    run_manifest: RunManifest,
    certified_vault_entry_source: String,
    certified_vault_entry_id: quantforge_core::ContentHash,
    certified_vault_entry_hash: quantforge_core::ContentHash,
    external_parity_artifact_hash: quantforge_core::ContentHash,
    incubation_artifact_hash: quantforge_core::ContentHash,
    candidate: EvidenceBinding,
    live_trading_default: bool,
    files: Vec<DeploymentFileRecord>,
}

#[derive(Debug, Serialize)]
struct JudgeArtifact<'a> {
    manifest: RunManifest,
    strategy_fingerprint: quantforge_core::ContentHash,
    decision_source: &'a Path,
    m1_source: &'a Path,
    strategy: &'a Path,
    broker: &'a Path,
    decision_metadata_hash: Option<&'a quantforge_core::ContentHash>,
    m1_metadata_hash: Option<&'a quantforge_core::ContentHash>,
    decision_data_quality: &'a DataQualityReport,
    m1_data_quality: &'a DataQualityReport,
    result: &'a JudgeResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct JudgeArtifactInput {
    manifest: RunManifest,
    strategy_fingerprint: quantforge_core::ContentHash,
    decision_data_quality: DataQualityReport,
    m1_data_quality: DataQualityReport,
    result: JudgeResult,
}

fn main() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Command::LoadCsv(source) => {
            let (dataset, metadata) = load_source(&source)?;
            print_json(&LoadSummary {
                source: &source.path,
                data_hash: &dataset.data_hash,
                source_rows: dataset.source_rows,
                bars: dataset.bars.len(),
                duplicate_rows_removed: dataset.duplicate_rows_removed,
                input_was_sorted: dataset.input_was_sorted,
                delimiter: dataset.delimiter,
                source_timezone: &dataset.source_timezone,
                metadata_hash: metadata.as_ref().map(|value| &value.metadata_hash),
                broker: metadata
                    .as_ref()
                    .and_then(|value| value.properties.get("broker").map(String::as_str)),
                server: metadata
                    .as_ref()
                    .and_then(|value| value.properties.get("server").map(String::as_str)),
            })?;
        }
        Command::DataQuality { source, out } => {
            let (dataset, metadata) = load_source(&source)?;
            let report = DataQualityReport::analyze(&dataset);
            if let Some(out) = out {
                let mut config = BTreeMap::<String, Value>::from([
                    ("source".into(), json!(display_path(&source.path))),
                    ("quality_protocol".into(), json!("bar-quality-v1")),
                    ("source_timezone".into(), json!(&dataset.source_timezone)),
                ]);
                if let Some(metadata_path) = &source.metadata {
                    config.insert("metadata".into(), json!(display_path(metadata_path)));
                }
                if let Some(metadata) = &metadata {
                    config.insert("metadata_hash".into(), json!(metadata.metadata_hash));
                }
                let manifest = RunManifest::new(
                    "data-quality",
                    RunRecipe {
                        data_hash: Some(dataset.data_hash.clone()),
                        broker_spec_hash: None,
                        grammar_version: None,
                        seed: None,
                        config,
                        override_flags: Vec::new(),
                    },
                )?;
                let artifact = QualityArtifact {
                    manifest,
                    source: &source.path,
                    data_hash: &dataset.data_hash,
                    metadata_hash: metadata.as_ref().map(|value| &value.metadata_hash),
                    source_timezone: &dataset.source_timezone,
                    report: &report,
                };
                let backup = write_json_versioned(&out, &artifact)?;
                println!("wrote {}", out.display());
                if let Some(backup) = backup {
                    println!("preserved previous report as {}", backup.display());
                }
            } else {
                print_json(&report)?;
            }
        }
        Command::Scout {
            source,
            strategy,
            broker,
            commission_per_lot_round_turn,
            slippage_points_per_side,
            fallback_spread_points,
            max_spread_points,
            initial_balance,
            entry_window_start_hour,
            entry_window_end_hour,
            allow_failed_data,
            out,
        } => {
            let (dataset, metadata) = load_source(&source)?;
            let quality = DataQualityReport::analyze(&dataset);
            if quality.grade == QualityGrade::Fail && !allow_failed_data {
                return Err(format!(
                    "data quality failed with score {}; pass --allow-failed-data to record an explicit override",
                    quality.score
                )
                .into());
            }
            let strategy_ir: StrategyIr = read_json(&strategy)?;
            let broker_spec: SymbolSpecification = read_json(&broker)?;
            validate_metadata_broker_binding(metadata.as_ref(), &broker_spec)?;
            let broker_hash = broker_spec.content_hash()?;
            let strategy_fingerprint =
                strategy_ir.structural_fingerprint(quantforge_core::FloatPolicy::default())?;
            let config = ScoutConfig {
                initial_balance,
                same_bar_policy: quantforge_eval::SameBarPolicy::Conservative,
                costs: CostModel {
                    fallback_spread_points,
                    adverse_slippage_points_per_side: slippage_points_per_side,
                    commission_per_lot_round_turn,
                    max_spread_points,
                    include_costs_in_risk: true,
                },
                indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
                entry_window: quantforge_eval::EntryWindow::new(
                    entry_window_start_hour,
                    entry_window_end_hour,
                ),
                abandon_above_drawdown_percent: None,
            };
            let result = evaluate_strategy(&strategy_ir, &dataset, &broker_spec, &config)?;

            let mut manifest_config = BTreeMap::<String, Value>::from([
                ("source".into(), json!(display_path(&source.path))),
                ("strategy".into(), json!(display_path(&strategy))),
                ("broker".into(), json!(display_path(&broker))),
                ("strategy_fingerprint".into(), json!(&strategy_fingerprint)),
                ("scout_config".into(), serde_json::to_value(&config)?),
                ("engine_tier".into(), json!(quantforge_eval::ENGINE_TIER)),
                ("data_quality_grade".into(), json!(quality.grade)),
                ("data_quality_score".into(), json!(quality.score)),
            ]);
            if let Some(metadata) = &metadata {
                manifest_config.insert("metadata_hash".into(), json!(&metadata.metadata_hash));
            }
            let manifest = RunManifest::new(
                "scout",
                RunRecipe {
                    data_hash: Some(dataset.data_hash.clone()),
                    broker_spec_hash: Some(broker_hash),
                    grammar_version: Some("export-safe-v1".into()),
                    seed: None,
                    config: manifest_config,
                    override_flags: if allow_failed_data {
                        vec!["allow_failed_data".into()]
                    } else {
                        Vec::new()
                    },
                },
            )?;
            let artifact = ScoutArtifact {
                manifest,
                strategy_fingerprint,
                source: &source.path,
                strategy: &strategy,
                broker: &broker,
                metadata_hash: metadata.as_ref().map(|value| &value.metadata_hash),
                data_quality: &quality,
                result: &result,
            };
            if let Some(out) = out {
                let backup = write_json_versioned(&out, &artifact)?;
                println!("wrote {}", out.display());
                if let Some(backup) = backup {
                    println!("preserved previous result as {}", backup.display());
                }
            } else {
                print_json(&artifact)?;
            }
        }
        Command::Evolve(args) => evolve_command(args)?,
        Command::Judge(args) => judge_command(args)?,
        Command::Export(args) => export_command(args)?,
        Command::Parity(args) => parity_command(args)?,
        Command::IndicatorParity(args) => indicator_parity_command(args)?,
        Command::Mt5Test(args) => mt5_test_command(args)?,
        Command::SplitPlan(args) => split_plan_command(args)?,
        Command::Certify(args) => certify_command(args)?,
        Command::AssembleEvidence(args) => assemble_evidence_command(args)?,
        Command::Portfolio(args) => portfolio_command(args)?,
        Command::IncubationStart(args) => incubation_start_command(args)?,
        Command::IncubationRecord(args) => incubation_record_command(args)?,
        Command::IncubationFinalize(args) => incubation_finalize_command(args)?,
        Command::Deploy(args) => deploy_command(args)?,
        Command::Challenge(args) => challenge_command(args)?,
        Command::SealedFinal(args) => sealed_final_command(args)?,
        Command::PermutationNull(args) => permutation_null_command(args)?,
        Command::ConditionBakeoff(args) => condition_bakeoff_command(args)?,
        Command::MethodologyResearch(args) => methodology_research_command(args)?,
    }
    Ok(())
}

fn parse_cli_run_mode(value: &str) -> Result<DiscoverRunMode, Box<dyn Error>> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fast_scout" | "scout" => Ok(DiscoverRunMode::FastScout),
        "full_harvest" | "harvest" => Ok(DiscoverRunMode::FullHarvest),
        "quota_harvest" | "quota" => Ok(DiscoverRunMode::QuotaHarvest),
        "mass_builder" | "builder" | "mass" => Ok(DiscoverRunMode::MassBuilder),
        other => Err(format!("unknown run mode: {other}").into()),
    }
}

fn condition_bakeoff_command(args: ConditionBakeoffArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "condition-bakeoff artifact already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    if args.entry_condition_counts.is_empty() {
        return Err("entry-condition-counts must include at least one count".into());
    }
    for count in &args.entry_condition_counts {
        if !(2..=UniversalGrammarConfig::MAX_ENTRY_CONDITIONS).contains(count) {
            return Err(format!(
                "entry-condition-counts must be within 2..={}: got {count}",
                UniversalGrammarConfig::MAX_ENTRY_CONDITIONS
            )
            .into());
        }
    }
    let (dataset, _) = load_source(&args.source)?;
    let m1_source = DataSourceArgs {
        path: args.m1.clone(),
        source_timezone: args.m1_source_timezone,
        metadata: args.m1_metadata.clone(),
    };
    let (m1, _) = load_source(&m1_source)?;
    let broker: SymbolSpecification = serde_json::from_slice(&fs::read(&args.broker)?)?;
    broker.validate()?;
    let search_dataset = development_partition(&dataset, args.validation_fraction, args.sealed_fraction)?;
    let oos1 = oos1_partition(&dataset, args.validation_fraction, args.sealed_fraction)?;
    let mut discover = DiscoverConfig {
        run_mode: DiscoverRunMode::FastScout,
        initial_candidates: args.initial_candidates,
        batch_size: args.initial_candidates.min(30),
        seed: args.seed,
        require_m1_robustness: false,
        require_m1_precision: false,
        worker_threads: 0,
        ..DiscoverConfig::default()
    };
    discover.scout.costs.commission_per_lot_round_turn = args.commission_per_lot_round_turn;
    discover.scout.costs.adverse_slippage_points_per_side = args.slippage_points_per_side;
    discover.scout.costs.fallback_spread_points = args.fallback_spread_points;
    let config = ConditionBakeoffConfig {
        discover,
        generations: args.generations,
        entry_condition_counts: args.entry_condition_counts,
    };
    let report = run_condition_bakeoff(
        &search_dataset,
        Some(&oos1),
        &m1,
        &broker,
        &[],
        &broker.symbol,
        config,
    )?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&args.out, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "condition-bakeoff wrote {} (recommended={:?})",
        args.out.display(),
        report.recommended
    );
    for row in &report.rows {
        println!(
            "  entry_conditions={} retention={:.3} oos1_E={:.4} pass={:.0}% pot={} evals={}",
            row.entry_conditions,
            row.median_retention,
            row.median_oos1_expectancy_r,
            row.pass_rate * 100.0,
            row.pot_elites,
            row.evaluations
        );
    }
    Ok(())
}

fn methodology_research_command(args: MethodologyResearchArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "methodology-research artifact already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let (dataset, _) = load_source(&args.source)?;
    let m1_source = DataSourceArgs {
        path: args.m1.clone(),
        source_timezone: args.m1_source_timezone,
        metadata: args.m1_metadata.clone(),
    };
    let (_m1, _) = load_source(&m1_source)?;
    let broker: SymbolSpecification = serde_json::from_slice(&fs::read(&args.broker)?)?;
    broker.validate()?;
    let search_dataset = development_partition(&dataset, args.validation_fraction, args.sealed_fraction)?;
    let oos1 = oos1_partition(&dataset, args.validation_fraction, args.sealed_fraction)?;
    let mut scout = ScoutConfig::default();
    scout.costs.commission_per_lot_round_turn = args.commission_per_lot_round_turn;
    scout.costs.adverse_slippage_points_per_side = args.slippage_points_per_side;
    scout.costs.fallback_spread_points = args.fallback_spread_points;
    let config = MethodologyGridConfig {
        seed: args.seed,
        draws_per_cell: args.draws_per_cell,
        scout,
        ..MethodologyGridConfig::default()
    };
    let report = run_methodology_grid(&search_dataset, &oos1, &broker, config)?;
    if let Some(parent) = args.out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&args.out, serde_json::to_vec_pretty(&report)?)?;
    println!(
        "methodology-research wrote {} ({} evaluations)",
        args.out.display(),
        report.evaluations
    );
    for line in &report.recommendations {
        println!("  • {line}");
    }
    println!("Top cells:");
    for cell in report.cells.iter().take(12) {
        println!(
            "  entry={} exit={} {} screened={} oos_pass={:.0}% ret={}",
            cell.entry_conditions,
            cell.exit_conditions,
            cell.recipe.label(),
            cell.screened,
            cell.oos1_pass_rate * 100.0,
            cell.median_retention
                .map(|value| format!("{value:.3}"))
                .unwrap_or_else(|| "n/a".into())
        );
    }
    println!("Contrasts:");
    for contrast in &report.contrasts {
        println!(
            "  {} n={}/{} ret_lift={} pass_lift={:+.1}pp p={:.3} q={:.3}{}",
            contrast.name,
            contrast.baseline_n,
            contrast.treatment_n,
            contrast
                .retention_lift
                .map(|value| format!("{value:+.3}"))
                .unwrap_or_else(|| "n/a".into()),
            contrast.pass_rate_lift * 100.0,
            contrast.p_value,
            contrast.q_value,
            if contrast.significant_fdr_10 {
                " *"
            } else {
                ""
            }
        );
    }
    Ok(())
}

fn permutation_null_command(args: PermutationNullArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "permutation-null artifact already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let (m1, metadata) = load_source(&args.source)?;
    let broker: SymbolSpecification = serde_json::from_slice(&fs::read(&args.broker)?)?;
    broker.validate()?;
    validate_metadata_broker_binding(metadata.as_ref(), &broker)?;
    let mut discover = DiscoverConfig {
        initial_candidates: args.initial_candidates,
        batch_size: args.initial_candidates.min(100),
        require_m1_precision: false,
        require_m1_robustness: false,
        calendar_year_folds: false,
        multi_symbol_minimum_pass: 0,
        minimum_deflated_trade_sharpe: None,
        worker_threads: 0,
        ..DiscoverConfig::default()
    };
    discover.scout.costs.commission_per_lot_round_turn = args.commission_per_lot_round_turn;
    discover.scout.costs.adverse_slippage_points_per_side = args.slippage_points_per_side;
    discover.scout.costs.fallback_spread_points = args.fallback_spread_points;
    let config = PermutationNullConfig {
        trials: args.trials,
        mean_block_length: args.mean_block_length,
        seed: args.seed,
        discover,
        generations: args.generations,
    };
    let report = run_permutation_null(&m1, &broker, &[], &config)?;
    let report_hash = quantforge_core::stable_json_hash(&report)?;
    let manifest = RunManifest::new(
        "permutation-null",
        RunRecipe {
            data_hash: Some(m1.data_hash.clone()),
            broker_spec_hash: Some(broker.content_hash()?),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: Some(args.seed),
            config: BTreeMap::from([
                ("source".into(), json!(display_path(&args.source.path))),
                ("broker".into(), json!(display_path(&args.broker))),
                ("trials".into(), json!(args.trials)),
                ("mean_block_length".into(), json!(args.mean_block_length)),
                ("report_hash".into(), json!(&report_hash)),
                ("p95_profit_factor".into(), json!(report.p95_profit_factor)),
                (
                    "p95_return_drawdown".into(),
                    json!(report.p95_return_drawdown),
                ),
                ("p95_expectancy".into(), json!(report.p95_expectancy)),
                ("p95_trade_sharpe".into(), json!(report.p95_trade_sharpe)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    write_json_new(
        &args.out,
        &json!({
            "manifest": manifest,
            "report": report,
        }),
    )?;
    print_json(&report)?;
    Ok(())
}

fn split_plan_command(args: SplitPlanArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "split-plan artifact already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let (dataset, metadata) = load_source(&args.source)?;
    let quality = DataQualityReport::analyze(&dataset);
    if quality.grade == QualityGrade::Fail && !args.allow_failed_data {
        return Err(format!(
            "data quality failed with score {}; pass --allow-failed-data only for a non-certifiable research split",
            quality.score
        )
        .into());
    }
    let plan =
        DataSplitPlan::chronological(&dataset, args.validation_fraction, args.sealed_fraction)?;
    let manifest = RunManifest::new(
        "split-plan",
        RunRecipe {
            data_hash: Some(dataset.data_hash.clone()),
            broker_spec_hash: None,
            grammar_version: None,
            seed: None,
            config: BTreeMap::from([
                ("source".into(), json!(display_path(&args.source.path))),
                ("source_timezone".into(), json!(&dataset.source_timezone)),
                (
                    "validation_fraction".into(),
                    json!(args.validation_fraction),
                ),
                ("sealed_fraction".into(), json!(args.sealed_fraction)),
                ("data_quality_grade".into(), json!(quality.grade)),
                ("data_quality_score".into(), json!(quality.score)),
            ]),
            override_flags: args
                .allow_failed_data
                .then_some("allow_failed_data".into())
                .into_iter()
                .collect(),
        },
    )?;
    let artifact = SplitPlanArtifact {
        manifest,
        source: display_path(&args.source.path),
        metadata_hash: metadata.map(|value| value.metadata_hash),
        data_quality: quality,
        validation_fraction: args.validation_fraction,
        sealed_fraction: args.sealed_fraction,
        plan,
    };
    write_json_new(&args.out, &artifact)?;
    println!(
        "wrote {} (development={} validation={} sealed={})",
        args.out.display(),
        artifact.plan.development.bar_count,
        artifact.plan.validation.bar_count,
        artifact.plan.sealed_final.bar_count
    );
    Ok(())
}

fn challenge_command(args: ChallengeArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "Challenge artifact already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let (dataset, metadata) = load_source(&args.source)?;
    let data_quality = DataQualityReport::analyze(&dataset);
    if data_quality.grade == QualityGrade::Fail {
        return Err(
            "Challenge refuses failed-quality data; research overrides are not promotion-grade"
                .into(),
        );
    }
    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    validate_metadata_broker_binding(metadata.as_ref(), &broker)?;
    let split_artifact: SplitPlanArtifact = read_json(&args.split_plan)?;
    split_artifact.manifest.validate()?;
    if split_artifact.manifest.command != "split-plan"
        || split_artifact.manifest.recipe.data_hash.as_ref()
            != Some(&split_artifact.plan.full_data_hash)
        || !split_artifact.manifest.recipe.override_flags.is_empty()
        || split_artifact.data_quality.grade == QualityGrade::Fail
    {
        return Err("Challenge requires an intact, promotion-grade split-plan artifact".into());
    }
    if dataset.data_hash != split_artifact.plan.full_data_hash {
        return Err("Challenge source data does not match the split plan".into());
    }

    let config = ChallengeConfig {
        scout: ScoutConfig {
            initial_balance: args.initial_balance,
            same_bar_policy: quantforge_eval::SameBarPolicy::Conservative,
            costs: CostModel {
                fallback_spread_points: args.fallback_spread_points,
                adverse_slippage_points_per_side: args.slippage_points_per_side,
                commission_per_lot_round_turn: args.commission_per_lot_round_turn,
                max_spread_points: args.max_spread_points,
                include_costs_in_risk: true,
            },
            indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
            entry_window: quantforge_eval::EntryWindow::new(
                args.entry_window_start_hour,
                args.entry_window_end_hour,
            ),
            abandon_above_drawdown_percent: None,
        },
        folds: args.folds,
        purge_bars: args.purge_bars,
        embargo_bars: args.embargo_bars,
        minimum_validation_bars: args.minimum_validation_bars,
        minimum_baseline_trades: args.minimum_baseline_trades,
        minimum_fold_trades: args.minimum_fold_trades,
        minimum_return_percent: args.minimum_return_percent,
        minimum_profit_factor: args.minimum_profit_factor,
        maximum_drawdown_percent: args.maximum_drawdown_percent,
        minimum_passing_fold_fraction: args.minimum_passing_fold_fraction,
        cost_multipliers: args.cost_multipliers,
        minimum_cost_survival_fraction: args.minimum_cost_survival_fraction,
        monte_carlo_trials: args.monte_carlo_trials,
        monte_carlo_block_length: args.monte_carlo_block_length,
        monte_carlo_minimum_p05_net_profit: args.monte_carlo_minimum_p05_net_profit,
        monte_carlo_maximum_p95_drawdown_percent: args.monte_carlo_maximum_p95_drawdown_percent,
        neighborhood_samples: args.neighborhood_samples,
        parameter_perturbation_fraction: args.parameter_perturbation_fraction,
        minimum_neighborhood_survival_fraction: args.minimum_neighborhood_survival_fraction,
        minimum_neighborhood_return_ratio: args.minimum_neighborhood_return_ratio,
        maximum_neighborhood_drawdown_ratio: args.maximum_neighborhood_drawdown_ratio,
        minimum_neighborhood_trade_ratio: args.minimum_neighborhood_trade_ratio,
        minimum_deflated_trade_sharpe: args.minimum_deflated_trade_sharpe,
        evaluations_touched: args.evaluations_touched,
        seed: args.seed,
    };
    let report = run_challenge(&strategy, &dataset, &broker, &split_artifact.plan, config)?;
    let manifest = RunManifest::new(
        "challenge",
        RunRecipe {
            data_hash: Some(report.validation_data_hash.clone()),
            broker_spec_hash: Some(report.binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: Some(report.config.seed),
            config: BTreeMap::from([
                ("source".into(), json!(display_path(&args.source.path))),
                ("strategy".into(), json!(display_path(&args.strategy))),
                ("broker".into(), json!(display_path(&args.broker))),
                ("split_plan".into(), json!(display_path(&args.split_plan))),
                ("split_plan_hash".into(), json!(&report.split_plan_hash)),
                (
                    "full_data_hash".into(),
                    json!(&split_artifact.plan.full_data_hash),
                ),
                (
                    "strategy_fingerprint".into(),
                    json!(&report.binding.strategy_fingerprint),
                ),
                (
                    "protocol".into(),
                    json!(quantforge_quality::CHALLENGE_PROTOCOL),
                ),
                ("report_passed".into(), json!(report.passed)),
                ("report_blockers".into(), json!(&report.blockers)),
                (
                    "evaluations_touched".into(),
                    json!(report.config.evaluations_touched),
                ),
                (
                    "challenge_config".into(),
                    serde_json::to_value(&report.config)?,
                ),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = ChallengeArtifact {
        manifest,
        source: display_path(&args.source.path),
        metadata_hash: metadata.map(|value| value.metadata_hash),
        data_quality,
        strategy_source: display_path(&args.strategy),
        broker_source: display_path(&args.broker),
        split_plan_source: display_path(&args.split_plan),
        report,
    };
    write_json_new(&args.out, &artifact)?;
    println!(
        "wrote {} (Challenge {}, {} fold(s) passed, {}/{} cost shocks survived)",
        args.out.display(),
        if artifact.report.passed {
            "passed"
        } else {
            "failed"
        },
        artifact
            .report
            .purged_folds
            .iter()
            .filter(|fold| fold.passed)
            .count(),
        artifact.report.cost_shocks.passing_points,
        artifact.report.cost_shocks.points.len()
    );
    if !artifact.report.passed {
        return Err(format!(
            "Challenge failed with {} blocker(s); inspect the written report",
            artifact.report.blockers.len()
        )
        .into());
    }
    Ok(())
}

fn sealed_final_command(args: SealedFinalArgs) -> Result<(), Box<dyn Error>> {
    // Validate every shortlist and policy input before claiming access. Market
    // bars are deliberately loaded only after the durable one-shot claim.
    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    let split_artifact: SplitPlanArtifact = read_json(&args.split_plan)?;
    let challenge_bytes = fs::read(&args.challenge)?;
    let challenge_artifact_hash = quantforge_core::ContentHash::sha256(&challenge_bytes);
    let challenge_artifact: ChallengeArtifact = serde_json::from_slice(&challenge_bytes)?;
    split_artifact.manifest.validate()?;
    challenge_artifact.manifest.validate()?;
    split_artifact.plan.validate()?;
    challenge_artifact.report.validate_integrity()?;

    if split_artifact.manifest.command != "split-plan"
        || split_artifact.manifest.recipe.data_hash.as_ref()
            != Some(&split_artifact.plan.full_data_hash)
        || !split_artifact.manifest.recipe.override_flags.is_empty()
        || split_artifact.data_quality.grade == QualityGrade::Fail
    {
        return Err("sealed-final requires an intact, promotion-grade split plan".into());
    }
    if challenge_artifact.manifest.command != "challenge" {
        return Err("shortlist artifact was not produced by the Challenge command".into());
    }
    if challenge_artifact.manifest.recipe.data_hash.as_ref()
        != Some(&challenge_artifact.report.validation_data_hash)
        || challenge_artifact.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&challenge_artifact.report.binding.broker_spec_hash)
    {
        return Err("Challenge manifest does not bind its validation data or broker".into());
    }
    if !challenge_artifact.manifest.recipe.override_flags.is_empty()
        || challenge_artifact.data_quality.grade == QualityGrade::Fail
    {
        return Err("research overrides or failed data make the Challenge ineligible".into());
    }
    if challenge_artifact
        .manifest
        .recipe
        .config
        .get("challenge_config")
        != Some(&serde_json::to_value(&challenge_artifact.report.config)?)
        || challenge_artifact
            .manifest
            .recipe
            .config
            .get("report_passed")
            != Some(&json!(challenge_artifact.report.passed))
        || challenge_artifact
            .manifest
            .recipe
            .config
            .get("report_blockers")
            != Some(&json!(&challenge_artifact.report.blockers))
    {
        return Err("Challenge manifest does not bind its configuration and outcome".into());
    }
    if !challenge_artifact.report.passed {
        return Err("sealed-final cannot open data for a candidate that failed Challenge".into());
    }

    let binding = EvidenceBinding {
        strategy_fingerprint: strategy
            .structural_fingerprint(quantforge_core::FloatPolicy::default())?,
        broker_spec_hash: broker.content_hash()?,
    };
    let split_plan_hash = split_artifact.plan.content_hash()?;
    if challenge_artifact.report.binding != binding
        || challenge_artifact.report.split_plan_hash != split_plan_hash
        || challenge_artifact.report.validation_data_hash
            != split_artifact.plan.validation.data_hash
    {
        return Err(
            "strategy, broker, Challenge and split plan do not describe one candidate".into(),
        );
    }
    let config = SealedFinalConfig {
        scout: ScoutConfig {
            initial_balance: args.initial_balance,
            same_bar_policy: quantforge_eval::SameBarPolicy::Conservative,
            costs: CostModel {
                fallback_spread_points: args.fallback_spread_points,
                adverse_slippage_points_per_side: args.slippage_points_per_side,
                commission_per_lot_round_turn: args.commission_per_lot_round_turn,
                max_spread_points: args.max_spread_points,
                include_costs_in_risk: true,
            },
            indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
            entry_window: quantforge_eval::EntryWindow::new(
                args.entry_window_start_hour,
                args.entry_window_end_hour,
            ),
            abandon_above_drawdown_percent: None,
        },
        minimum_trades: args.minimum_trades,
        minimum_return_percent: args.minimum_return_percent,
        minimum_profit_factor: args.minimum_profit_factor,
        maximum_drawdown_percent: args.maximum_drawdown_percent,
    };
    config.validate(&challenge_artifact.report)?;

    let final_path = sealed_final_path(
        &args.sealed_root,
        &binding.strategy_fingerprint,
        &split_plan_hash,
    );
    if final_path.exists() {
        return Err(format!(
            "sealed-final was already evaluated for this strategy and split: {}",
            final_path.display()
        )
        .into());
    }
    let access_path = claim_sealed_access_once(
        &args.sealed_root,
        &binding.strategy_fingerprint,
        &split_plan_hash,
        &challenge_artifact_hash,
    )?;

    let (dataset, metadata) = load_source(&args.source)?;
    let data_quality = DataQualityReport::analyze(&dataset);
    if data_quality.grade == QualityGrade::Fail {
        return Err(format!(
            "sealed data access was claimed at {}, but source quality failed; the final test cannot be retried",
            access_path.display()
        )
        .into());
    }
    validate_metadata_broker_binding(metadata.as_ref(), &broker)?;
    let report = run_sealed_final(
        &strategy,
        &dataset,
        &broker,
        &split_artifact.plan,
        &challenge_artifact.report,
        challenge_artifact_hash,
        config,
    )?;
    let report_hash = quantforge_core::stable_json_hash(&report)?;
    let manifest = RunManifest::new(
        "sealed-final",
        RunRecipe {
            data_hash: Some(report.sealed_data_hash.clone()),
            broker_spec_hash: Some(report.binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("source".into(), json!(display_path(&args.source.path))),
                ("strategy".into(), json!(display_path(&args.strategy))),
                ("broker".into(), json!(display_path(&args.broker))),
                ("split_plan".into(), json!(display_path(&args.split_plan))),
                ("challenge".into(), json!(display_path(&args.challenge))),
                ("access_claim".into(), json!(display_path(&access_path))),
                ("split_plan_hash".into(), json!(&report.split_plan_hash)),
                (
                    "challenge_artifact_hash".into(),
                    json!(&report.challenge_artifact_hash),
                ),
                (
                    "strategy_fingerprint".into(),
                    json!(&report.binding.strategy_fingerprint),
                ),
                (
                    "protocol".into(),
                    json!(quantforge_quality::SEALED_FINAL_PROTOCOL),
                ),
                ("report_hash".into(), json!(&report_hash)),
                (
                    "sealed_config".into(),
                    serde_json::to_value(&report.config)?,
                ),
                ("report_passed".into(), json!(report.passed)),
                ("report_blockers".into(), json!(&report.blockers)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = SealedFinalArtifact {
        manifest,
        source: display_path(&args.source.path),
        metadata_hash: metadata.map(|value| value.metadata_hash),
        data_quality,
        strategy_source: display_path(&args.strategy),
        broker_source: display_path(&args.broker),
        split_plan_source: display_path(&args.split_plan),
        challenge_source: display_path(&args.challenge),
        report,
    };
    let written = write_sealed_final_once(&args.sealed_root, &artifact.report, &artifact)?;
    println!(
        "wrote {} (sealed final {}, {} trades, {:.4}% return)",
        written.display(),
        if artifact.report.passed {
            "passed"
        } else {
            "failed; candidate demoted to Illuminated"
        },
        artifact.report.result.metrics.trade_count,
        artifact.report.result.metrics.return_percent
    );
    if !artifact.report.passed {
        return Err(format!(
            "sealed final failed with {} blocker(s); the recorded attempt cannot be retried",
            artifact.report.blockers.len()
        )
        .into());
    }
    Ok(())
}

fn incubation_start_command(args: IncubationStartArgs) -> Result<(), Box<dyn Error>> {
    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    broker.validate()?;
    let split_artifact: SplitPlanArtifact = read_json(&args.split_plan)?;
    verify_split_artifact(&split_artifact)?;
    let binding = EvidenceBinding {
        strategy_fingerprint: strategy
            .structural_fingerprint(quantforge_core::FloatPolicy::default())?,
        broker_spec_hash: broker.content_hash()?,
    };
    let start = IncubationStart {
        schema_version: quantforge_quality::INCUBATION_SCHEMA_VERSION,
        protocol_version: INCUBATION_PROTOCOL.into(),
        binding: binding.clone(),
        split_plan_hash: split_artifact.plan.content_hash()?,
        started_on: args.start_date,
        initial_balance: args.initial_balance,
        kill_rules: IncubationKillRules {
            maximum_daily_loss_percent: args.maximum_daily_loss_percent,
            maximum_total_drawdown_percent: args.maximum_total_drawdown_percent,
            minimum_observation_days: args.minimum_observation_days,
            minimum_total_trades: args.minimum_total_trades,
            maximum_consecutive_zero_trade_days: args.maximum_consecutive_zero_trade_days,
        },
    };
    start.validate()?;
    let path = args
        .root
        .join(binding.strategy_fingerprint.as_str())
        .join(start.split_plan_hash.as_str())
        .join("incubation-start.json");
    let strategy_source = display_path(&args.strategy);
    let broker_source = display_path(&args.broker);
    let split_plan_source = display_path(&args.split_plan);
    let manifest = RunManifest::new(
        "incubation-start",
        RunRecipe {
            data_hash: Some(split_artifact.plan.full_data_hash),
            broker_spec_hash: Some(binding.broker_spec_hash),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("strategy".into(), json!(&strategy_source)),
                ("broker".into(), json!(&broker_source)),
                ("split_plan".into(), json!(&split_plan_source)),
                ("split_plan_hash".into(), json!(&start.split_plan_hash)),
                ("protocol".into(), json!(INCUBATION_PROTOCOL)),
                ("start".into(), serde_json::to_value(&start)?),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = IncubationStartArtifact {
        manifest,
        strategy_source,
        broker_source,
        split_plan_source,
        start,
    };
    write_json_new(&path, &artifact)?;
    println!("opened immutable incubation ledger at {}", path.display());
    Ok(())
}

fn incubation_record_command(args: IncubationRecordArgs) -> Result<(), Box<dyn Error>> {
    let final_path = incubation_final_path(&args.start)?;
    if final_path.exists() {
        return Err(
            "the incubation ledger is already finalized and cannot accept observations".into(),
        );
    }
    let ledger = load_incubation_ledger(&args.start)?;
    if let Some(previous) = ledger.observations.last() {
        if args.date <= previous.observation.date {
            return Err("the new observation date must be later than every recorded date".into());
        }
    } else if args.date < ledger.start.start.started_on {
        return Err("the first observation cannot predate incubation".into());
    }
    let starting_balance = ledger
        .observations
        .last()
        .map_or(ledger.start.start.initial_balance, |value| {
            value.observation.ending_balance
        });
    let observation = IncubationObservation {
        date: args.date,
        starting_balance,
        ending_balance: args.ending_balance,
        maximum_drawdown_percent: args.maximum_drawdown_percent,
        trade_count: args.trade_count,
        note: args.note,
    };
    observation.validate()?;
    let start_source = display_path(&args.start);
    let manifest = RunManifest::new(
        "incubation-record",
        RunRecipe {
            data_hash: ledger.start.manifest.recipe.data_hash.clone(),
            broker_spec_hash: Some(ledger.start.start.binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("start".into(), json!(&start_source)),
                ("start_artifact_hash".into(), json!(&ledger.start_hash)),
                ("protocol".into(), json!(INCUBATION_PROTOCOL)),
                ("observation".into(), serde_json::to_value(&observation)?),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = IncubationObservationArtifact {
        manifest,
        start_source,
        start_artifact_hash: ledger.start_hash,
        observation,
    };
    let path = args
        .start
        .parent()
        .ok_or("incubation start path has no parent")?
        .join("observations")
        .join(format!("{}.json", artifact.observation.date));
    write_json_new(&path, &artifact)?;
    println!("appended incubation observation {}", path.display());
    Ok(())
}

fn incubation_finalize_command(args: IncubationFinalizeArgs) -> Result<(), Box<dyn Error>> {
    let final_path = incubation_final_path(&args.start)?;
    if final_path.exists() {
        return Err(format!(
            "incubation final already exists and will not be replaced: {}",
            final_path.display()
        )
        .into());
    }
    let ledger = load_incubation_ledger(&args.start)?;
    let observations: Vec<_> = ledger
        .observations
        .iter()
        .map(|artifact| artifact.observation.clone())
        .collect();
    let report = run_incubation(
        &ledger.start.start,
        &observations,
        ledger.start_hash.clone(),
        ledger.observation_hashes.clone(),
    )?;
    let report_hash = quantforge_core::stable_json_hash(&report)?;
    let observation_sources: Vec<_> = ledger
        .observation_paths
        .iter()
        .map(|path| display_path(path))
        .collect();
    let start_source = display_path(&args.start);
    let manifest = RunManifest::new(
        "incubation-final",
        RunRecipe {
            data_hash: ledger.start.manifest.recipe.data_hash.clone(),
            broker_spec_hash: Some(ledger.start.start.binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("start".into(), json!(&start_source)),
                ("start_artifact_hash".into(), json!(&ledger.start_hash)),
                ("observation_sources".into(), json!(&observation_sources)),
                (
                    "observation_artifact_hashes".into(),
                    json!(&ledger.observation_hashes),
                ),
                ("protocol".into(), json!(INCUBATION_PROTOCOL)),
                ("report_hash".into(), json!(&report_hash)),
                ("report_passed".into(), json!(report.passed)),
                ("report_blockers".into(), json!(&report.blockers)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = IncubationFinalArtifact {
        manifest,
        start_source,
        observation_sources,
        start: ledger.start.start,
        observations,
        report,
    };
    write_json_new(&final_path, &artifact)?;
    println!(
        "sealed incubation {} at {} ({} days, {} trades)",
        if artifact.report.passed {
            "passed"
        } else {
            "failed"
        },
        final_path.display(),
        artifact.report.observation_days,
        artifact.report.total_trades
    );
    if !artifact.report.passed {
        return Err(format!(
            "incubation failed with {} blocker(s); the recorded final cannot be retried",
            artifact.report.blockers.len()
        )
        .into());
    }
    Ok(())
}

struct LoadedIncubationLedger {
    start: IncubationStartArtifact,
    start_hash: quantforge_core::ContentHash,
    observations: Vec<IncubationObservationArtifact>,
    observation_hashes: Vec<quantforge_core::ContentHash>,
    observation_paths: Vec<PathBuf>,
}

fn incubation_final_path(start_path: &Path) -> Result<PathBuf, Box<dyn Error>> {
    Ok(start_path
        .parent()
        .ok_or("incubation start path has no parent")?
        .join("incubation-final.json"))
}

fn load_incubation_ledger(start_path: &Path) -> Result<LoadedIncubationLedger, Box<dyn Error>> {
    let (start, start_hash) = read_json_hashed::<IncubationStartArtifact>(start_path)?;
    verify_incubation_start_artifact(&start)?;
    let ledger_dir = start_path
        .parent()
        .ok_or("incubation start path has no parent")?;
    if start_path.file_name().and_then(|value| value.to_str()) != Some("incubation-start.json")
        || ledger_dir.file_name().and_then(|value| value.to_str())
            != Some(start.start.split_plan_hash.as_str())
        || ledger_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            != Some(start.start.binding.strategy_fingerprint.as_str())
    {
        return Err("incubation start is outside its deterministic strategy/split location".into());
    }
    let observations_dir = ledger_dir.join("observations");
    let mut paths = Vec::new();
    if observations_dir.exists() {
        for entry in fs::read_dir(&observations_dir)? {
            let path = entry?.path();
            if !path.is_file() || path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                return Err(format!(
                    "unexpected entry in incubation observation store: {}",
                    path.display()
                )
                .into());
            }
            paths.push(path);
        }
    }
    paths.sort();
    let mut observations = Vec::with_capacity(paths.len());
    let mut hashes = Vec::with_capacity(paths.len());
    for path in &paths {
        let (artifact, hash) = read_json_hashed::<IncubationObservationArtifact>(path)?;
        verify_incubation_observation_artifact(&artifact, &start, &start_hash, start_path)?;
        if path.file_name().and_then(|value| value.to_str())
            != Some(&format!("{}.json", artifact.observation.date))
        {
            return Err(format!(
                "incubation observation filename does not match its date: {}",
                path.display()
            )
            .into());
        }
        observations.push(artifact);
        hashes.push(hash);
    }
    if !observations.is_empty() {
        let values: Vec<_> = observations
            .iter()
            .map(|artifact| artifact.observation.clone())
            .collect();
        run_incubation(&start.start, &values, start_hash.clone(), hashes.clone())?;
    }
    Ok(LoadedIncubationLedger {
        start,
        start_hash,
        observations,
        observation_hashes: hashes,
        observation_paths: paths,
    })
}

fn verify_incubation_start_artifact(
    artifact: &IncubationStartArtifact,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact.start.validate()?;
    if artifact.manifest.command != "incubation-start"
        || artifact.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&artifact.start.binding.broker_spec_hash)
        || artifact.manifest.recipe.data_hash.is_none()
        || artifact.manifest.recipe.grammar_version.as_deref()
            != Some(quantforge_discover::GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.manifest.recipe.config.get("strategy")
            != Some(&json!(&artifact.strategy_source))
        || artifact.manifest.recipe.config.get("broker") != Some(&json!(&artifact.broker_source))
        || artifact.manifest.recipe.config.get("split_plan")
            != Some(&json!(&artifact.split_plan_source))
        || artifact.manifest.recipe.config.get("split_plan_hash")
            != Some(&json!(&artifact.start.split_plan_hash))
        || artifact.manifest.recipe.config.get("protocol") != Some(&json!(INCUBATION_PROTOCOL))
        || artifact.manifest.recipe.config.get("start")
            != Some(&serde_json::to_value(&artifact.start)?)
    {
        return Err("incubation start artifact is invalid or internally unbound".into());
    }
    Ok(())
}

fn verify_incubation_observation_artifact(
    artifact: &IncubationObservationArtifact,
    start: &IncubationStartArtifact,
    start_hash: &quantforge_core::ContentHash,
    start_path: &Path,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact.observation.validate()?;
    if artifact.manifest.command != "incubation-record"
        || artifact.start_artifact_hash != *start_hash
        || artifact.start_source != display_path(start_path)
        || artifact.manifest.recipe.data_hash != start.manifest.recipe.data_hash
        || artifact.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&start.start.binding.broker_spec_hash)
        || artifact.manifest.recipe.grammar_version.as_deref()
            != Some(quantforge_discover::GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.manifest.recipe.config.get("start") != Some(&json!(&artifact.start_source))
        || artifact.manifest.recipe.config.get("start_artifact_hash") != Some(&json!(start_hash))
        || artifact.manifest.recipe.config.get("protocol") != Some(&json!(INCUBATION_PROTOCOL))
        || artifact.manifest.recipe.config.get("observation")
            != Some(&serde_json::to_value(&artifact.observation)?)
    {
        return Err("incubation observation artifact is invalid or internally unbound".into());
    }
    Ok(())
}

fn verify_incubation_final_artifact(
    artifact: &IncubationFinalArtifact,
    binding: &EvidenceBinding,
    split_plan: &DataSplitPlan,
    split_plan_hash: &quantforge_core::ContentHash,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact
        .report
        .validate_integrity(&artifact.start, &artifact.observations)?;
    if artifact.manifest.command != "incubation-final"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&split_plan.full_data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&binding.broker_spec_hash)
        || artifact.manifest.recipe.grammar_version.as_deref()
            != Some(quantforge_discover::GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.start.binding != *binding
        || artifact.start.split_plan_hash != *split_plan_hash
        || artifact.report.binding != *binding
        || artifact.report.split_plan_hash != *split_plan_hash
        || !artifact.report.passed
        || !artifact.report.blockers.is_empty()
    {
        return Err("incubation final is failed, mismatched or internally unbound".into());
    }

    let start_path = PathBuf::from(&artifact.start_source);
    let (source_start, start_hash) = read_json_hashed::<IncubationStartArtifact>(&start_path)?;
    verify_incubation_start_artifact(&source_start)?;
    if source_start.start != artifact.start || start_hash != artifact.report.start_artifact_hash {
        return Err("incubation final does not match its source start artifact".into());
    }
    if artifact.observation_sources.len() != artifact.observations.len() {
        return Err("incubation final has incomplete observation source bindings".into());
    }
    let mut source_hashes = Vec::with_capacity(artifact.observation_sources.len());
    for (index, source) in artifact.observation_sources.iter().enumerate() {
        let path = PathBuf::from(source);
        let (observation, hash) = read_json_hashed::<IncubationObservationArtifact>(&path)?;
        verify_incubation_observation_artifact(
            &observation,
            &source_start,
            &start_hash,
            &start_path,
        )?;
        if observation.observation != artifact.observations[index] {
            return Err("incubation final observation differs from its source artifact".into());
        }
        source_hashes.push(hash);
    }
    let report_hash = quantforge_core::stable_json_hash(&artifact.report)?;
    if source_hashes != artifact.report.observation_artifact_hashes
        || artifact.manifest.recipe.config.get("start") != Some(&json!(&artifact.start_source))
        || artifact.manifest.recipe.config.get("start_artifact_hash")
            != Some(&json!(&artifact.report.start_artifact_hash))
        || artifact.manifest.recipe.config.get("observation_sources")
            != Some(&json!(&artifact.observation_sources))
        || artifact
            .manifest
            .recipe
            .config
            .get("observation_artifact_hashes")
            != Some(&json!(&artifact.report.observation_artifact_hashes))
        || artifact.manifest.recipe.config.get("protocol") != Some(&json!(INCUBATION_PROTOCOL))
        || artifact.manifest.recipe.config.get("report_hash") != Some(&json!(&report_hash))
        || artifact.manifest.recipe.config.get("report_passed")
            != Some(&json!(artifact.report.passed))
        || artifact.manifest.recipe.config.get("report_blockers")
            != Some(&json!(&artifact.report.blockers))
    {
        return Err("incubation final manifest does not bind its source ledger and report".into());
    }
    Ok(())
}

fn assemble_evidence_command(args: AssembleEvidenceArgs) -> Result<(), Box<dyn Error>> {
    let validation_path = args.out_dir.join("validation-attestation.json");
    let evidence_path = args.out_dir.join("certification-evidence.json");
    let bundle_path = args.out_dir.join("certification-bundle.json");
    if let Some(existing) = [&validation_path, &evidence_path, &bundle_path]
        .into_iter()
        .find(|path| path.exists())
    {
        return Err(format!(
            "evidence assembly output already exists and will not be replaced: {}",
            existing.display()
        )
        .into());
    }

    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    broker.validate()?;
    let binding = EvidenceBinding {
        strategy_fingerprint: strategy
            .structural_fingerprint(quantforge_core::FloatPolicy::default())?,
        broker_spec_hash: broker.content_hash()?,
    };

    let (split_artifact, _) = read_json_hashed::<SplitPlanArtifact>(&args.split_plan)?;
    verify_split_artifact(&split_artifact)?;
    let split_plan_hash = split_artifact.plan.content_hash()?;

    let (databank, databank_hash) = read_json_hashed::<EvolveArtifact>(&args.databank)?;
    let (challenge, challenge_hash) = read_json_hashed::<ChallengeArtifact>(&args.challenge)?;
    let (judge, judge_hash) = read_json_hashed::<JudgeArtifactInput>(&args.judge)?;
    let (parity, parity_hash) = read_json_hashed::<ParityArtifact>(&args.parity)?;
    let (indicator, indicator_hash) =
        read_json_hashed::<IndicatorParityArtifact>(&args.indicator_parity)?;
    let (sealed, sealed_hash) = read_json_hashed::<SealedFinalArtifact>(&args.sealed_final)?;
    let incubation = args
        .incubation
        .as_ref()
        .map(|path| read_json_hashed::<IncubationFinalArtifact>(path))
        .transpose()?;

    verify_challenge_artifact(&challenge, &binding, &split_artifact.plan, &split_plan_hash)?;
    verify_databank_artifact(
        &databank,
        &strategy,
        &binding,
        &split_artifact.plan,
        &challenge.report,
    )?;
    verify_judge_artifact(&judge, &binding, &split_artifact.plan, &challenge.report)?;
    verify_parity_artifact(
        &parity,
        &binding,
        &broker,
        &split_artifact.plan,
        &challenge.report,
    )?;
    verify_indicator_artifact(&indicator, &broker)?;
    verify_sealed_artifact(
        &sealed,
        &binding,
        &split_artifact.plan,
        &split_plan_hash,
        &challenge,
        &challenge_hash,
    )?;
    if let Some((incubation, _)) = &incubation {
        verify_incubation_final_artifact(
            incubation,
            &binding,
            &split_artifact.plan,
            &split_plan_hash,
        )?;
    }

    let attestation =
        ValidationAttestation::from_challenge(&challenge.report, challenge_hash.clone())?;
    if !attestation.passed {
        return Err("the Challenge validation baseline did not pass".into());
    }
    let attestation_hash = quantforge_core::stable_json_hash(&attestation)?;
    let validation_manifest = RunManifest::new(
        "validation-attestation",
        RunRecipe {
            data_hash: Some(attestation.validation_data_hash.clone()),
            broker_spec_hash: Some(attestation.binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("challenge".into(), json!(display_path(&args.challenge))),
                (
                    "source_challenge_artifact_hash".into(),
                    json!(&challenge_hash),
                ),
                ("attestation_hash".into(), json!(&attestation_hash)),
                ("split_plan_hash".into(), json!(&split_plan_hash)),
                (
                    "strategy_fingerprint".into(),
                    json!(&binding.strategy_fingerprint),
                ),
                ("protocol".into(), json!(VALIDATION_PROTOCOL)),
                ("passed".into(), json!(attestation.passed)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let validation_artifact = ValidationArtifact {
        manifest: validation_manifest,
        challenge_source: display_path(&args.challenge),
        attestation,
    };
    write_json_new(&validation_path, &validation_artifact)?;
    let validation_hash = hash_file(&validation_path)?;

    let evidence = CertificationEvidence {
        schema_version: CERTIFICATION_SCHEMA_VERSION,
        protocol_version: EVIDENCE_PROTOCOL_VERSION.into(),
        candidate: binding.clone(),
        split_plan_hash: split_plan_hash.clone(),
        validation: DataGateEvidence {
            gate: passing_gate(&binding, validation_hash.clone(), VALIDATION_PROTOCOL),
            data_hash: split_artifact.plan.validation.data_hash.clone(),
        },
        illumination: passing_gate(&binding, databank_hash.clone(), ILLUMINATION_PROTOCOL),
        challenge: passing_gate(&binding, challenge_hash.clone(), CHALLENGE_PROTOCOL),
        judge: passing_gate(&binding, judge_hash.clone(), JUDGE_PROTOCOL),
        external_parity: ExternalParityEvidence {
            gate: passing_gate(&binding, parity_hash.clone(), EXTERNAL_PARITY_PROTOCOL),
            engine: ExternalEngine::Mt5StrategyTester,
            protective_orders_present: parity.report.protective_orders_present,
        },
        indicator_parity: passing_gate(&binding, indicator_hash.clone(), INDICATOR_PARITY_PROTOCOL),
        sealed_final: SealedFinalEvidence {
            gate: passing_gate(&binding, sealed_hash.clone(), SEALED_FINAL_PROTOCOL),
            split_plan_hash: sealed.report.split_plan_hash.clone(),
            sealed_data_hash: sealed.report.sealed_data_hash.clone(),
            shortlisted_before_open: sealed.report.shortlisted_before_open,
            used_in_selection_score: sealed.report.used_in_selection_score,
        },
        incubation: incubation
            .as_ref()
            .map(|(_, hash)| passing_gate(&binding, hash.clone(), INCUBATION_PROTOCOL)),
        evaluations_touched: databank.databank.evaluation_count,
        research_override_flags: Vec::new(),
    };
    let decision = evaluate_certification(
        &evidence,
        &split_artifact.plan,
        &CertificationPolicy {
            require_incubation: incubation.is_some(),
            ..CertificationPolicy::default()
        },
    )?;
    if !decision.passed {
        return Err(format!(
            "assembled evidence failed its own certification check with {} blocker(s)",
            decision.blockers.len()
        )
        .into());
    }
    write_json_new(&evidence_path, &evidence)?;
    let evidence_hash = hash_file(&evidence_path)?;

    let mut artifacts = vec![
        artifact_reference("validation", &validation_path, validation_hash),
        artifact_reference("illumination", &args.databank, databank_hash),
        artifact_reference("challenge", &args.challenge, challenge_hash),
        artifact_reference("judge", &args.judge, judge_hash),
        artifact_reference("external_parity", &args.parity, parity_hash),
        artifact_reference("indicator_parity", &args.indicator_parity, indicator_hash),
        artifact_reference("sealed_final", &args.sealed_final, sealed_hash),
    ];
    if let (Some(path), Some((_, hash))) = (&args.incubation, &incubation) {
        artifacts.push(artifact_reference("incubation", path, hash.clone()));
    }
    let gate_count = artifacts.len();
    let bundle_manifest = RunManifest::new(
        "assemble-evidence",
        RunRecipe {
            data_hash: Some(split_artifact.plan.full_data_hash.clone()),
            broker_spec_hash: Some(binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("evidence_hash".into(), json!(&evidence_hash)),
                ("artifacts".into(), serde_json::to_value(&artifacts)?),
                ("split_plan_hash".into(), json!(&split_plan_hash)),
                (
                    "strategy_fingerprint".into(),
                    json!(&binding.strategy_fingerprint),
                ),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let bundle = EvidenceBundle {
        schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION,
        manifest: bundle_manifest,
        evidence_source: display_path(&evidence_path),
        evidence_hash,
        artifacts,
    };
    write_json_new(&bundle_path, &bundle)?;
    println!(
        "wrote {} ({} verified gates; ready for `quantforge certify --bundle {}`)",
        bundle_path.display(),
        gate_count,
        bundle_path.display()
    );
    Ok(())
}

fn read_json_hashed<T: DeserializeOwned>(
    path: &Path,
) -> Result<(T, quantforge_core::ContentHash), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let hash = quantforge_core::ContentHash::sha256(&bytes);
    Ok((serde_json::from_slice(&bytes)?, hash))
}

fn hash_file(path: &Path) -> Result<quantforge_core::ContentHash, Box<dyn Error>> {
    Ok(quantforge_core::ContentHash::sha256(fs::read(path)?))
}

fn passing_gate(
    binding: &EvidenceBinding,
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

fn artifact_reference(
    gate: &str,
    path: &Path,
    content_hash: quantforge_core::ContentHash,
) -> VaultArtifactReference {
    VaultArtifactReference {
        gate: gate.into(),
        path: display_path(path),
        content_hash,
    }
}

fn verify_split_artifact(artifact: &SplitPlanArtifact) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact.plan.validate()?;
    if artifact.manifest.command != "split-plan"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&artifact.plan.full_data_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || artifact.manifest.recipe.config.get("data_quality_grade")
            != Some(&json!(artifact.data_quality.grade))
        || artifact.manifest.recipe.config.get("data_quality_score")
            != Some(&json!(artifact.data_quality.score))
        || artifact.manifest.recipe.config.get("validation_fraction")
            != Some(&json!(artifact.validation_fraction))
        || artifact.manifest.recipe.config.get("sealed_fraction")
            != Some(&json!(artifact.sealed_fraction))
    {
        return Err("split-plan artifact is not intact and promotion-grade".into());
    }
    Ok(())
}

fn verify_challenge_artifact(
    artifact: &ChallengeArtifact,
    binding: &EvidenceBinding,
    plan: &DataSplitPlan,
    split_plan_hash: &quantforge_core::ContentHash,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact.report.validate_integrity()?;
    if artifact.manifest.command != "challenge"
        || artifact.manifest.recipe.data_hash.as_ref()
            != Some(&artifact.report.validation_data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&artifact.report.binding.broker_spec_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || artifact.report.binding != *binding
        || artifact.report.split_plan_hash != *split_plan_hash
        || artifact.report.validation_data_hash != plan.validation.data_hash
        || artifact.report.validation_bar_count != plan.validation.bar_count
        || !artifact.report.passed
        || !artifact.report.blockers.is_empty()
        || !artifact.report.baseline_passed()
        || artifact.manifest.recipe.config.get("challenge_config")
            != Some(&serde_json::to_value(&artifact.report.config)?)
        || artifact.manifest.recipe.config.get("report_passed")
            != Some(&json!(artifact.report.passed))
        || artifact.manifest.recipe.config.get("report_blockers")
            != Some(&json!(&artifact.report.blockers))
        || artifact.manifest.recipe.config.get("evaluations_touched")
            != Some(&json!(artifact.report.config.evaluations_touched))
        || artifact.manifest.recipe.config.get("strategy_fingerprint")
            != Some(&json!(&binding.strategy_fingerprint))
        || artifact.manifest.recipe.config.get("split_plan_hash") != Some(&json!(split_plan_hash))
    {
        return Err(
            "Challenge artifact is failed, overridden, mismatched or internally unbound".into(),
        );
    }
    Ok(())
}

fn verify_databank_artifact(
    artifact: &EvolveArtifact,
    strategy: &StrategyIr,
    binding: &EvidenceBinding,
    plan: &DataSplitPlan,
    challenge: &ChallengeReport,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    let bank = &artifact.databank;
    bank.validate_integrity()?;
    if artifact.manifest.command != "evolve"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&bank.data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&bank.broker_spec_hash)
        || artifact.manifest.recipe.grammar_version.as_deref()
            != Some(quantforge_discover::GRAMMAR_VERSION)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || bank.data_hash != plan.development.data_hash
        || bank.broker_spec_hash != binding.broker_spec_hash
        || bank.evaluation_count != challenge.config.evaluations_touched
        || bank.config.scout != challenge.config.scout
        || artifact.coverage != bank.coverage()
        || (artifact.qd_score - bank.qd_score()).abs() > 1.0e-9
        || artifact.manifest.recipe.config.get("discover_config")
            != Some(&serde_json::to_value(&bank.config)?)
    {
        return Err(
            "MAP-Elites databank is overridden, mismatched or internally inconsistent".into(),
        );
    }
    if !bank.elites.iter().any(|elite| {
        elite.structural_fingerprint == binding.strategy_fingerprint && elite.strategy == *strategy
    }) {
        return Err("candidate is not an exact elite in the development-only databank".into());
    }
    Ok(())
}

fn verify_judge_artifact(
    artifact: &JudgeArtifactInput,
    binding: &EvidenceBinding,
    plan: &DataSplitPlan,
    challenge: &ChallengeReport,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    let config_value = artifact
        .manifest
        .recipe
        .config
        .get("judge_config")
        .ok_or("Judge manifest is missing judge_config")?
        .clone();
    let config: JudgeConfig = serde_json::from_value(config_value)?;
    config.validate()?;
    let decision_data_hash: quantforge_core::ContentHash = serde_json::from_value(
        artifact
            .manifest
            .recipe
            .config
            .get("decision_data_hash")
            .ok_or("Judge manifest is missing decision_data_hash")?
            .clone(),
    )?;
    let m1_data_hash: quantforge_core::ContentHash = serde_json::from_value(
        artifact
            .manifest
            .recipe
            .config
            .get("m1_data_hash")
            .ok_or("Judge manifest is missing m1_data_hash")?
            .clone(),
    )?;
    let combined_hash = quantforge_core::stable_json_hash(&BTreeMap::from([
        ("decision", &decision_data_hash),
        ("m1", &m1_data_hash),
    ]))?;
    if artifact.manifest.command != "judge"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&combined_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&binding.broker_spec_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.strategy_fingerprint != binding.strategy_fingerprint
        || decision_data_hash != plan.validation.data_hash
        || artifact.decision_data_quality.grade == QualityGrade::Fail
        || artifact.m1_data_quality.grade == QualityGrade::Fail
        || artifact.manifest.recipe.config.get("strategy_fingerprint")
            != Some(&json!(&binding.strategy_fingerprint))
        || artifact.manifest.recipe.config.get("decision_quality")
            != Some(&json!(artifact.decision_data_quality.grade))
        || artifact.manifest.recipe.config.get("m1_quality")
            != Some(&json!(artifact.m1_data_quality.grade))
        || config.allow_execution_gaps
        || config.initial_balance != challenge.config.scout.initial_balance
        || config.costs != challenge.config.scout.costs
        || artifact.result.engine != quantforge_tick::ENGINE_TIER
        || artifact.result.execution_interval_ms != 60_000
        || artifact.result.decision_interval_ms <= artifact.result.execution_interval_ms
        || artifact.result.decision_interval_ms % artifact.result.execution_interval_ms != 0
        || artifact.result.telemetry.m1_gap_events != 0
        || !challenge.metrics_pass_baseline(&artifact.result.metrics)
    {
        return Err(
            "M1 Judge artifact is overridden, mismatched or below the Challenge baseline gates"
                .into(),
        );
    }
    Ok(())
}

fn verify_parity_artifact(
    artifact: &ParityArtifact,
    binding: &EvidenceBinding,
    broker: &SymbolSpecification,
    plan: &DataSplitPlan,
    challenge: &ChallengeReport,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact
        .mt5_metadata
        .validate_evidence(&artifact.evidence)?;
    let recomputed = compare_runs(
        &artifact.reference,
        &artifact.external,
        &artifact.evidence,
        artifact.report.tolerances.clone(),
    )?;
    if artifact.manifest.command != "parity"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&plan.validation.data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&binding.broker_spec_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.evidence.schema_version != quantforge_export_mql5::EXPORT_SCHEMA_VERSION
        || artifact.evidence.target != quantforge_export_mql5::EXPORT_TARGET
        || artifact.evidence.strategy_fingerprint != binding.strategy_fingerprint
        || artifact.evidence.broker_spec_hash != binding.broker_spec_hash
        || artifact.evidence.live_trading_default
        || artifact.evidence.config.allow_live_trading_default
        || !artifact.evidence.mandatory_stop_loss
        || !artifact.evidence.mandatory_take_profit
        || artifact.evidence.symbol != broker.symbol
        || artifact.evidence.config.tester.currency != broker.account_currency
        || artifact.evidence.config.tester.deposit != challenge.config.scout.initial_balance
        || artifact.evidence.config.commission_per_lot_round_turn
            != challenge.config.scout.costs.commission_per_lot_round_turn
        || artifact.evidence.config.estimated_slippage_points_per_side
            != challenge
                .config
                .scout
                .costs
                .adverse_slippage_points_per_side
        || artifact.evidence.config.max_spread_points
            != challenge.config.scout.costs.max_spread_points
        || artifact.reference != ParityRun::from_scout(&challenge.baseline)
        || artifact.reference.engine != quantforge_eval::ENGINE_TIER
        || artifact.external.engine != "mt5-strategy-tester"
        || artifact.report.protocol_version != quantforge_parity::PARITY_PROTOCOL_VERSION
        || artifact.report != recomputed
        || !artifact.report.passed
        || !artifact.report.protective_orders_present
        || artifact.manifest.recipe.config.get("strategy_fingerprint")
            != Some(&json!(&binding.strategy_fingerprint))
        || artifact.manifest.recipe.config.get("source_hash")
            != Some(&json!(&artifact.evidence.source_hash))
        || artifact.manifest.recipe.config.get("protocol")
            != Some(&json!(quantforge_parity::PARITY_PROTOCOL_VERSION))
    {
        return Err(
            "MT5 parity artifact is failed, mismatched, internal-only or lacks protective orders"
                .into(),
        );
    }
    Ok(())
}

fn verify_indicator_artifact(
    artifact: &IndicatorParityArtifact,
    broker: &SymbolSpecification,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    let report = &artifact.report;
    let required_fields = BTreeSet::from([
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
    ]);
    let actual_fields: BTreeSet<_> = report.indicators.keys().map(String::as_str).collect();
    let fields_are_intact = actual_fields == required_fields
        && report.indicators.values().all(|field| {
            field.passed
                && field.compared_rows == report.compared_rows
                && field.mismatch_count == 0
                && field.first_mismatch_row.is_none()
                && field.first_mismatch_timestamp_ms.is_none()
        });
    if artifact.manifest.command != "indicator-parity"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&report.reference_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || report.protocol_version != quantforge_parity::INDICATOR_PARITY_PROTOCOL_VERSION
        || !report.passed
        || !fields_are_intact
        || report.metadata.terminal_build == 0
        || report.metadata.server.is_empty()
        || report.metadata.broker.is_empty()
        || report.metadata.symbol != broker.symbol
        || report.source_rows <= report.config.warmup_rows
        || report.compared_rows != report.source_rows - report.config.warmup_rows
        || artifact.manifest.recipe.config.get("protocol")
            != Some(&json!(quantforge_parity::INDICATOR_PARITY_PROTOCOL_VERSION))
        || artifact.manifest.recipe.config.get("terminal_build")
            != Some(&json!(report.metadata.terminal_build))
        || artifact.manifest.recipe.config.get("tolerances")
            != Some(&serde_json::to_value(&report.config)?)
    {
        return Err(
            "MT5 indicator parity artifact is failed, malformed or for another symbol".into(),
        );
    }
    Ok(())
}

fn verify_sealed_artifact(
    artifact: &SealedFinalArtifact,
    binding: &EvidenceBinding,
    plan: &DataSplitPlan,
    split_plan_hash: &quantforge_core::ContentHash,
    challenge: &ChallengeArtifact,
    challenge_artifact_hash: &quantforge_core::ContentHash,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact.report.validate_integrity(&challenge.report)?;
    let report = &artifact.report;
    if artifact.manifest.command != "sealed-final"
        || artifact.manifest.recipe.data_hash.as_ref() != Some(&report.sealed_data_hash)
        || artifact.manifest.recipe.broker_spec_hash.as_ref() != Some(&binding.broker_spec_hash)
        || !artifact.manifest.recipe.override_flags.is_empty()
        || artifact.data_quality.grade == QualityGrade::Fail
        || report.binding != *binding
        || report.split_plan_hash != *split_plan_hash
        || report.challenge_artifact_hash != *challenge_artifact_hash
        || report.sealed_data_hash != plan.sealed_final.data_hash
        || report.sealed_bar_count != plan.sealed_final.bar_count
        || report.sealed_start_timestamp_ms != plan.sealed_final.start_timestamp_ms
        || report.sealed_end_timestamp_ms_exclusive != plan.sealed_final.end_timestamp_ms_exclusive
        || !report.shortlisted_before_open
        || report.used_in_selection_score
        || !report.passed
        || !report.blockers.is_empty()
        || artifact.manifest.recipe.config.get("sealed_config")
            != Some(&serde_json::to_value(&report.config)?)
        || artifact.manifest.recipe.config.get("report_passed") != Some(&json!(report.passed))
        || artifact.manifest.recipe.config.get("report_blockers") != Some(&json!(&report.blockers))
        || artifact
            .manifest
            .recipe
            .config
            .get("challenge_artifact_hash")
            != Some(&json!(challenge_artifact_hash))
        || artifact.manifest.recipe.config.get("split_plan_hash") != Some(&json!(split_plan_hash))
    {
        return Err("sealed-final artifact is failed, mismatched, reused in selection or internally unbound".into());
    }
    Ok(())
}

fn certify_command(args: CertifyArgs) -> Result<(), Box<dyn Error>> {
    let inputs = resolve_certification_inputs(&args)?;
    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    let split_artifact: SplitPlanArtifact = read_json(&args.split_plan)?;
    let evidence: CertificationEvidence = read_json(&inputs.evidence_source)?;

    split_artifact.manifest.validate()?;
    if split_artifact.manifest.command != "split-plan"
        || split_artifact.manifest.recipe.data_hash.as_ref()
            != Some(&split_artifact.plan.full_data_hash)
    {
        return Err("split-plan artifact manifest does not bind its full data hash".into());
    }
    if !split_artifact.manifest.recipe.override_flags.is_empty()
        || split_artifact.data_quality.grade == QualityGrade::Fail
    {
        return Err(
            "a failed-data or research-override split plan cannot enter certification".into(),
        );
    }

    let actual_binding = EvidenceBinding {
        strategy_fingerprint: strategy
            .structural_fingerprint(quantforge_core::FloatPolicy::default())?,
        broker_spec_hash: broker.content_hash()?,
    };
    if evidence.candidate != actual_binding {
        return Err(
            "strategy or broker content does not match the certification candidate binding".into(),
        );
    }
    if let Some(bundle) = &inputs.bundle
        && (bundle.manifest.recipe.data_hash.as_ref() != Some(&split_artifact.plan.full_data_hash)
            || bundle.manifest.recipe.broker_spec_hash.as_ref()
                != Some(&actual_binding.broker_spec_hash))
    {
        return Err("assembled bundle manifest does not bind this split plan and broker".into());
    }
    let policy = CertificationPolicy {
        require_incubation: args.require_incubation,
        selection_bias_warning_threshold: args.selection_bias_warning_threshold,
    };
    let decision = evaluate_certification(&evidence, &split_artifact.plan, &policy)?;
    if !decision.passed {
        print_json(&decision)?;
        return Err(format!(
            "certification denied with {} blocker(s); no Vault entry was written",
            decision.blockers.len()
        )
        .into());
    }

    let artifact_references = bind_gate_artifacts(&evidence, &inputs.artifacts)?;
    let strategy_source_hash = quantforge_core::ContentHash::sha256(fs::read(&args.strategy)?);
    let broker_source_hash = quantforge_core::ContentHash::sha256(fs::read(&args.broker)?);
    let split_plan_source_hash = quantforge_core::ContentHash::sha256(fs::read(&args.split_plan)?);
    let evidence_source_hash =
        quantforge_core::ContentHash::sha256(fs::read(&inputs.evidence_source)?);
    let manifest = RunManifest::new(
        "certify",
        RunRecipe {
            data_hash: Some(split_artifact.plan.full_data_hash.clone()),
            broker_spec_hash: Some(actual_binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("strategy".into(), json!(display_path(&args.strategy))),
                ("broker".into(), json!(display_path(&args.broker))),
                ("split_plan".into(), json!(display_path(&args.split_plan))),
                (
                    "evidence".into(),
                    json!(display_path(&inputs.evidence_source)),
                ),
                ("vault".into(), json!(display_path(&args.vault))),
                ("policy".into(), serde_json::to_value(&policy)?),
                (
                    "strategy_fingerprint".into(),
                    json!(&actual_binding.strategy_fingerprint),
                ),
                (
                    "split_plan_hash".into(),
                    json!(split_artifact.plan.content_hash()?),
                ),
                ("evidence_hash".into(), json!(&decision.evidence_hash)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let payload = VaultPayload {
        manifest,
        strategy_source: display_path(&args.strategy),
        strategy_source_hash,
        strategy,
        broker_source: display_path(&args.broker),
        broker_source_hash,
        broker,
        split_plan_source: display_path(&args.split_plan),
        split_plan_source_hash,
        split_plan: split_artifact.plan.clone(),
        evidence_source: display_path(&inputs.evidence_source),
        evidence_source_hash,
        evidence: evidence.clone(),
        artifacts: artifact_references,
    };
    let admission = admit_certified(
        &args.vault,
        &evidence,
        &split_artifact.plan,
        &policy,
        payload,
    )?;
    println!(
        "admitted Certified entry {} to {}",
        admission.entry_id,
        admission.path.display()
    );
    if !admission.decision.warnings.is_empty() {
        println!(
            "recorded {} certification warning(s)",
            admission.decision.warnings.len()
        );
    }
    Ok(())
}

struct CertificationInputs {
    evidence_source: PathBuf,
    artifacts: Vec<PathBuf>,
    bundle: Option<EvidenceBundle>,
}

fn resolve_certification_inputs(args: &CertifyArgs) -> Result<CertificationInputs, Box<dyn Error>> {
    if let Some(bundle_path) = &args.bundle {
        let bundle: EvidenceBundle = read_json(bundle_path)?;
        bundle.manifest.validate()?;
        if bundle.schema_version != EVIDENCE_BUNDLE_SCHEMA_VERSION
            || bundle.manifest.command != "assemble-evidence"
            || !bundle.manifest.recipe.override_flags.is_empty()
            || bundle.manifest.recipe.config.get("evidence_hash")
                != Some(&json!(&bundle.evidence_hash))
            || bundle.manifest.recipe.config.get("artifacts")
                != Some(&serde_json::to_value(&bundle.artifacts)?)
        {
            return Err(
                "certification bundle manifest is invalid or does not bind its contents".into(),
            );
        }
        let evidence_source = PathBuf::from(&bundle.evidence_source);
        let actual_evidence_hash =
            quantforge_core::ContentHash::sha256(fs::read(&evidence_source)?);
        if actual_evidence_hash != bundle.evidence_hash {
            return Err("certification evidence bytes do not match the assembled bundle".into());
        }
        let mut gates = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        let mut artifacts = Vec::with_capacity(bundle.artifacts.len());
        for reference in &bundle.artifacts {
            if !gates.insert(reference.gate.clone())
                || !hashes.insert(reference.content_hash.clone())
            {
                return Err(
                    "certification bundle contains duplicate gates or artifact hashes".into(),
                );
            }
            let path = PathBuf::from(&reference.path);
            let actual_hash = quantforge_core::ContentHash::sha256(fs::read(&path)?);
            if actual_hash != reference.content_hash {
                return Err(format!(
                    "{} artifact bytes do not match the assembled bundle",
                    reference.gate
                )
                .into());
            }
            artifacts.push(path);
        }
        return Ok(CertificationInputs {
            evidence_source,
            artifacts,
            bundle: Some(bundle),
        });
    }

    let evidence_source = args
        .evidence
        .clone()
        .ok_or("provide either --bundle or --evidence with --artifact inputs")?;
    if args.artifacts.is_empty() {
        return Err("manual certification requires every referenced --artifact".into());
    }
    Ok(CertificationInputs {
        evidence_source,
        artifacts: args.artifacts.clone(),
        bundle: None,
    })
}

fn bind_gate_artifacts(
    evidence: &CertificationEvidence,
    paths: &[PathBuf],
) -> Result<Vec<VaultArtifactReference>, Box<dyn Error>> {
    let mut available = BTreeMap::<quantforge_core::ContentHash, &Path>::new();
    for path in paths {
        let hash = quantforge_core::ContentHash::sha256(fs::read(path)?);
        if let Some(previous) = available.insert(hash.clone(), path) {
            return Err(format!(
                "duplicate artifact content supplied by {} and {} ({hash})",
                previous.display(),
                path.display()
            )
            .into());
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
    for (gate, required_hash) in required {
        let path = available.get(required_hash).ok_or_else(|| {
            format!(
                "the {gate} gate references {required_hash}, but no supplied --artifact has that content hash"
            )
        })?;
        used.insert(required_hash.clone());
        references.push(VaultArtifactReference {
            gate: gate.into(),
            path: display_path(path),
            content_hash: required_hash.clone(),
        });
    }
    if let Some(unused) = available
        .iter()
        .find(|(hash, _)| !used.contains(*hash))
        .map(|(_, path)| path)
    {
        return Err(format!(
            "supplied artifact {} is not referenced by any certification gate",
            unused.display()
        )
        .into());
    }
    Ok(references)
}

fn judge_command(args: JudgeArgs) -> Result<(), Box<dyn Error>> {
    let (decision_dataset, decision_metadata) = load_source(&args.decision)?;
    let m1_source = DataSourceArgs {
        path: args.m1.clone(),
        source_timezone: args.m1_source_timezone,
        metadata: args.m1_metadata.clone(),
    };
    let (m1_dataset, m1_metadata) = load_source(&m1_source)?;
    let interval_ms = infer_median_interval_ms(&decision_dataset.bars).unwrap_or(3_600_000);
    let grid: Vec<i64> = decision_dataset
        .bars
        .iter()
        .map(|bar| bar.timestamp_ms)
        .collect();
    let decision_dataset = build_timeframe_from_m1(&m1_dataset, interval_ms, Some(&grid))?;
    let decision_quality = DataQualityReport::analyze(&decision_dataset);
    let m1_quality = DataQualityReport::analyze(&m1_dataset);
    if (decision_quality.grade == QualityGrade::Fail || m1_quality.grade == QualityGrade::Fail)
        && !args.allow_failed_data
    {
        return Err(format!(
            "judge input quality failed (decision={:?}, m1={:?}); pass --allow-failed-data to record an explicit override",
            decision_quality.grade, m1_quality.grade
        )
        .into());
    }

    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    validate_metadata_broker_binding(decision_metadata.as_ref(), &broker)?;
    validate_metadata_broker_binding(m1_metadata.as_ref(), &broker)?;
    let broker_hash = broker.content_hash()?;
    let strategy_fingerprint =
        strategy.structural_fingerprint(quantforge_core::FloatPolicy::default())?;
    let config = JudgeConfig {
        initial_balance: args.initial_balance,
        costs: CostModel {
            fallback_spread_points: args.fallback_spread_points,
            adverse_slippage_points_per_side: args.slippage_points_per_side,
            commission_per_lot_round_turn: args.commission_per_lot_round_turn,
            max_spread_points: args.max_spread_points,
            include_costs_in_risk: true,
        },
        allow_execution_gaps: args.allow_execution_gaps,
        indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
        entry_window: quantforge_eval::EntryWindow::new(
            args.entry_window_start_hour,
            args.entry_window_end_hour,
        ),
    };
    let result = evaluate_strategy_m1(&strategy, &decision_dataset, &m1_dataset, &broker, &config)?;
    let combined_data_hash = quantforge_core::stable_json_hash(&BTreeMap::from([
        ("decision", &decision_dataset.data_hash),
        ("m1", &m1_dataset.data_hash),
    ]))?;
    let manifest = RunManifest::new(
        "judge",
        RunRecipe {
            data_hash: Some(combined_data_hash),
            broker_spec_hash: Some(broker_hash),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                (
                    "decision_source".into(),
                    json!(display_path(&args.decision.path)),
                ),
                ("m1_source".into(), json!(display_path(&args.m1))),
                ("strategy".into(), json!(display_path(&args.strategy))),
                ("broker".into(), json!(display_path(&args.broker))),
                ("strategy_fingerprint".into(), json!(&strategy_fingerprint)),
                ("judge_config".into(), serde_json::to_value(&config)?),
                (
                    "decision_data_hash".into(),
                    json!(&decision_dataset.data_hash),
                ),
                ("m1_data_hash".into(), json!(&m1_dataset.data_hash)),
                ("decision_quality".into(), json!(decision_quality.grade)),
                ("m1_quality".into(), json!(m1_quality.grade)),
            ]),
            override_flags: [
                args.allow_failed_data.then_some("allow_failed_data".into()),
                args.allow_execution_gaps
                    .then_some("allow_execution_gaps".into()),
            ]
            .into_iter()
            .flatten()
            .collect(),
        },
    )?;
    let artifact = JudgeArtifact {
        manifest,
        strategy_fingerprint,
        decision_source: &args.decision.path,
        m1_source: &args.m1,
        strategy: &args.strategy,
        broker: &args.broker,
        decision_metadata_hash: decision_metadata
            .as_ref()
            .map(|metadata| &metadata.metadata_hash),
        m1_metadata_hash: m1_metadata.as_ref().map(|metadata| &metadata.metadata_hash),
        decision_data_quality: &decision_quality,
        m1_data_quality: &m1_quality,
        result: &result,
    };
    let backup = write_json_versioned(&args.out, &artifact)?;
    println!(
        "wrote {} ({} trades, {} M1 bars replayed)",
        args.out.display(),
        result.metrics.trade_count,
        result.telemetry.m1_bars_replayed
    );
    if let Some(backup) = backup {
        println!("preserved previous judge result as {}", backup.display());
    }
    Ok(())
}

fn indicator_parity_command(args: IndicatorParityArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "indicator parity report already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let report = compare_indicator_reference(
        &args.reference,
        IndicatorParityConfig {
            warmup_rows: args.warmup_rows,
            absolute_epsilon: args.absolute_epsilon,
            relative_epsilon: args.relative_epsilon,
        },
    )?;
    let manifest = RunManifest::new(
        "indicator-parity",
        RunRecipe {
            data_hash: Some(report.reference_hash.clone()),
            broker_spec_hash: None,
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("reference".into(), json!(display_path(&args.reference))),
                (
                    "protocol".into(),
                    json!(quantforge_parity::INDICATOR_PARITY_PROTOCOL_VERSION),
                ),
                (
                    "terminal_build".into(),
                    json!(report.metadata.terminal_build),
                ),
                ("broker".into(), json!(&report.metadata.broker)),
                ("server".into(), json!(&report.metadata.server)),
                ("symbol".into(), json!(&report.metadata.symbol)),
                ("timeframe".into(), json!(&report.metadata.timeframe)),
                ("period".into(), json!(report.metadata.period)),
                ("tolerances".into(), serde_json::to_value(&report.config)?),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = IndicatorParityArtifact { manifest, report };
    write_json_new(&args.out, &artifact)?;
    println!(
        "wrote {} (indicator parity {})",
        args.out.display(),
        if artifact.report.passed {
            "passed"
        } else {
            "failed"
        }
    );
    if !artifact.report.passed {
        return Err("one or more MT5 indicator buffers exceeded epsilon".into());
    }
    Ok(())
}

fn export_command(args: ExportArgs) -> Result<(), Box<dyn Error>> {
    let compiler = args.compile.then(|| metaeditor_config(&args)).transpose()?;
    let strategy: StrategyIr = read_json(&args.strategy)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    let config = Mql5ExportConfig {
        expert_name: args.expert_name.clone(),
        expert_directory: args.expert_directory,
        timeframe: args.timeframe,
        magic: args.magic,
        deviation_points: args.deviation_points,
        max_spread_points: args.max_spread_points,
        estimated_slippage_points_per_side: args.slippage_points_per_side,
        commission_per_lot_round_turn: args.commission_per_lot_round_turn,
        allow_live_trading_default: false,
        export_style: ExportStyle::Sqx,
        entry_window_start_hour: args.entry_window_start_hour,
        entry_window_end_hour: args.entry_window_end_hour,
        tester: TesterConfig {
            from_date: args.from_date,
            to_date: args.to_date,
            deposit: args.deposit,
            currency: args.currency,
            leverage: args.leverage,
            model: args.tester_model,
        },
    };
    let bundle = generate_bundle(&strategy, &broker, &config)?;
    let source_path = args.out.join(format!("{}.mq5", args.expert_name));
    let set_path = args.out.join(format!("{}.set", args.expert_name));
    let tester_path = args.out.join(format!("{}.tester.ini", args.expert_name));
    let evidence_path = args.out.join(format!("{}.evidence.json", args.expert_name));
    let compile_path = args.out.join(format!("{}.compile.json", args.expert_name));
    let mut targets = vec![&source_path, &set_path, &tester_path, &evidence_path];
    if args.compile {
        targets.push(&compile_path);
    }
    if let Some(existing) = targets.into_iter().find(|path| path.exists()) {
        return Err(format!(
            "export target already exists and will not be replaced: {}",
            existing.display()
        )
        .into());
    }

    write_text_new(&source_path, &bundle.source)?;
    write_text_new(&set_path, &bundle.set_file)?;
    write_text_new(&tester_path, &bundle.tester_ini)?;
    write_json_new(&evidence_path, &bundle.evidence)?;
    for support in &bundle.support_files {
        let path = args.out.join(&support.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        write_text_new(&path, &support.contents)?;
    }
    println!("wrote guarded export to {}", args.out.display());

    if args.compile {
        let report = compile_with_metaeditor(
            &source_path,
            compiler
                .as_ref()
                .expect("compiler configuration was built for --compile"),
        )?;
        write_json_new(&compile_path, &report)?;
        println!(
            "MetaEditor: {} errors, {} warnings ({})",
            report
                .errors
                .map_or_else(|| "unknown".into(), |value| value.to_string()),
            report
                .warnings
                .map_or_else(|| "unknown".into(), |value| value.to_string()),
            if report.success { "passed" } else { "failed" }
        );
        if !report.success {
            return Err(format!(
                "generated source did not compile; inspect {}",
                compile_path.display()
            )
            .into());
        }
    }
    Ok(())
}

fn parity_command(args: ParityArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "parity report already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let mut evidence: ExportEvidenceCard = read_json(&args.evidence)?;
    let (reference, data_hash, grammar_version, strategy_fingerprint, broker_spec_hash) =
        if let Ok(judge) = read_json::<JudgeArtifactInput>(&args.scout_result) {
            if judge.strategy_fingerprint != evidence.strategy_fingerprint {
                return Err(
                    "Judge result and MQL5 evidence reference different strategies".into(),
                );
            }
            if judge.manifest.recipe.broker_spec_hash.as_ref()
                != Some(&evidence.broker_spec_hash)
            {
                return Err(
                    "Judge result and MQL5 evidence reference different broker profiles".into(),
                );
            }
            (
                ParityRun::from_judge(&judge.result),
                judge.manifest.recipe.data_hash.clone(),
                judge.manifest.recipe.grammar_version.clone(),
                judge.strategy_fingerprint,
                evidence.broker_spec_hash.clone(),
            )
        } else {
            let scout: ScoutArtifactInput = read_json(&args.scout_result)?;
            if scout.strategy_fingerprint != evidence.strategy_fingerprint {
                return Err(
                    "Scout result and MQL5 evidence reference different strategies".into(),
                );
            }
            if scout.manifest.recipe.broker_spec_hash.as_ref()
                != Some(&evidence.broker_spec_hash)
            {
                return Err(
                    "Scout result and MQL5 evidence reference different broker profiles".into(),
                );
            }
            (
                ParityRun::from_scout(&scout.result),
                scout.manifest.recipe.data_hash.clone(),
                scout.manifest.recipe.grammar_version.clone(),
                scout.strategy_fingerprint,
                evidence.broker_spec_hash.clone(),
            )
        };
    let _ = (strategy_fingerprint, broker_spec_hash);
    let source = fs::read(&args.mq5)?;
    if quantforge_core::ContentHash::sha256(&source) != evidence.source_hash {
        return Err("MQL5 source hash does not match the evidence card".into());
    }
    let source_text = String::from_utf8(source)?;
    let protective_calls = source_text.contains("g_trade.Buy(volume,_Symbol,0.0,stop,target")
        && source_text.contains("g_trade.Sell(volume,_Symbol,0.0,stop,target");
    evidence.mandatory_stop_loss &= protective_calls;
    evidence.mandatory_take_profit &= protective_calls;
    let mt5_metadata = load_mt5_tester_metadata(&args.mt5_metadata)?;
    mt5_metadata.validate_evidence(&evidence)?;
    let broker_timezone = args.broker_timezone.clone().or_else(|| {
        mt5_metadata
            .properties
            .get("broker_timezone")
            .cloned()
    });
    let external = load_mt5_tester_run_in_timezone(
        &args.mt5_deals,
        &args.mt5_equity,
        args.initial_balance,
        broker_timezone.as_deref(),
    )?;
    let tolerances = ParityTolerances {
        trade_count_relative: args.trade_count_relative,
        trade_count_absolute: args.trade_count_absolute,
        net_profit_relative: args.net_profit_relative,
        max_drawdown_relative: args.max_drawdown_relative,
        max_equity_divergence_percent: args.max_equity_divergence_percent,
        trade_timestamp_tolerance_ms: args.trade_timestamp_tolerance_ms,
        minimum_aligned_trade_fraction: args.minimum_aligned_trade_fraction,
    };
    let report = compare_runs(&reference, &external, &evidence, tolerances)?;
    let manifest = RunManifest::new(
        "parity",
        RunRecipe {
            data_hash,
            broker_spec_hash: Some(evidence.broker_spec_hash.clone()),
            grammar_version,
            seed: None,
            config: BTreeMap::from([
                (
                    "scout_result".into(),
                    json!(display_path(&args.scout_result)),
                ),
                ("evidence".into(), json!(display_path(&args.evidence))),
                ("mq5".into(), json!(display_path(&args.mq5))),
                ("mt5_deals".into(), json!(display_path(&args.mt5_deals))),
                ("mt5_equity".into(), json!(display_path(&args.mt5_equity))),
                (
                    "mt5_metadata".into(),
                    json!(display_path(&args.mt5_metadata)),
                ),
                (
                    "broker_timezone".into(),
                    json!(broker_timezone),
                ),
                (
                    "reference_engine".into(),
                    json!(reference.engine),
                ),
                (
                    "strategy_fingerprint".into(),
                    json!(&evidence.strategy_fingerprint),
                ),
                ("source_hash".into(), json!(&evidence.source_hash)),
                (
                    "protocol".into(),
                    json!(quantforge_parity::PARITY_PROTOCOL_VERSION),
                ),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = ParityArtifact {
        manifest,
        evidence,
        reference,
        external,
        mt5_metadata,
        report,
    };
    write_json_new(&args.out, &artifact)?;
    println!(
        "wrote {} ({})",
        args.out.display(),
        if artifact.report.passed {
            "parity passed"
        } else {
            "parity failed"
        }
    );
    Ok(())
}

fn mt5_test_command(args: Mt5TestArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "tester run report already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let evidence: ExportEvidenceCard = read_json(&args.evidence)?;
    let (terminal, common_files) = terminal_config(&args)?;
    let deals_path = join_windows_relative(&common_files, &evidence.parity_deals_file);
    let equity_path = join_windows_relative(&common_files, &evidence.parity_equity_file);
    let metadata_path = join_windows_relative(&common_files, &evidence.parity_metadata_file);
    let report = run_mt5_tester(
        &args.tester_ini,
        &deals_path,
        &equity_path,
        &metadata_path,
        &terminal,
    )?;
    let manifest = RunManifest::new(
        "mt5-test",
        RunRecipe {
            data_hash: None,
            broker_spec_hash: Some(evidence.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("tester_ini".into(), json!(display_path(&args.tester_ini))),
                ("terminal".into(), json!(display_path(&terminal.executable))),
                (
                    "strategy_fingerprint".into(),
                    json!(&evidence.strategy_fingerprint),
                ),
                ("source_hash".into(), json!(&evidence.source_hash)),
                ("timeout_seconds".into(), json!(args.timeout_seconds)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let artifact = Mt5TestArtifact {
        manifest,
        evidence,
        report,
    };
    write_json_new(&args.out, &artifact)?;
    println!(
        "wrote {} ({})",
        args.out.display(),
        if artifact.report.success {
            "tester output ready"
        } else {
            "tester failed"
        }
    );
    if !artifact.report.success {
        return Err("MT5 tester did not produce fresh deal and equity outputs".into());
    }
    Ok(())
}

fn metaeditor_config(args: &ExportArgs) -> Result<MetaEditorConfig, Box<dyn Error>> {
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is unavailable; provide --metaeditor, --wine and --wine-prefix")?;
    let default_prefix =
        user_home.join("Library/Application Support/net.metaquotes.wine.metatrader5");
    let executable = args.metaeditor.clone().unwrap_or_else(|| {
        default_prefix.join("drive_c/Program Files/MetaTrader 5/metaeditor64.exe")
    });
    let wine_binary = args.wine.clone().or_else(|| {
        let candidate =
            PathBuf::from("/Applications/MetaTrader 5.app/Contents/SharedSupport/wine/bin/wine");
        candidate.is_file().then_some(candidate)
    });
    let wine_prefix = args
        .wine_prefix
        .clone()
        .or_else(|| (wine_binary.is_some() && default_prefix.is_dir()).then_some(default_prefix));
    Ok(MetaEditorConfig {
        executable,
        wine_binary,
        wine_prefix,
    })
}

fn terminal_config(args: &Mt5TestArgs) -> Result<(TerminalConfig, PathBuf), Box<dyn Error>> {
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is unavailable; provide explicit MT5 terminal paths")?;
    let user_name = std::env::var_os("USER")
        .map(PathBuf::from)
        .ok_or("USER is unavailable; provide --common-files")?;
    let default_prefix =
        user_home.join("Library/Application Support/net.metaquotes.wine.metatrader5");
    let executable = args.terminal.clone().unwrap_or_else(|| {
        default_prefix.join("drive_c/Program Files/MetaTrader 5/terminal64.exe")
    });
    let wine_binary = args.wine.clone().or_else(|| {
        let candidate =
            PathBuf::from("/Applications/MetaTrader 5.app/Contents/SharedSupport/wine/bin/wine");
        candidate.is_file().then_some(candidate)
    });
    let wine_prefix = args.wine_prefix.clone().or_else(|| {
        (wine_binary.is_some() && default_prefix.is_dir()).then_some(default_prefix.clone())
    });
    let common_files = args.common_files.clone().unwrap_or_else(|| {
        default_prefix
            .join("drive_c/users")
            .join(user_name)
            .join("AppData/Roaming/MetaQuotes/Terminal/Common/Files")
    });
    Ok((
        TerminalConfig {
            executable,
            wine_binary,
            wine_prefix,
            timeout_seconds: args.timeout_seconds,
        },
        common_files,
    ))
}

fn join_windows_relative(base: &Path, relative: &str) -> PathBuf {
    relative
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
        .fold(base.to_path_buf(), |path, component| path.join(component))
}

fn evolve_command(args: EvolveArgs) -> Result<(), Box<dyn Error>> {
    let (dataset, metadata) = load_source(&args.source)?;
    let m1_source = DataSourceArgs {
        path: args.m1.clone(),
        source_timezone: args.m1_source_timezone,
        metadata: args.m1_metadata.clone(),
    };
    let (m1_dataset, m1_metadata) = load_source(&m1_source)?;
    let quality = DataQualityReport::analyze(&dataset);
    if quality.grade == QualityGrade::Fail && !args.allow_failed_data {
        return Err(format!(
            "data quality failed with score {}; pass --allow-failed-data to record an explicit override",
            quality.score
        )
        .into());
    }
    let broker_spec: SymbolSpecification = read_json(&args.broker)?;
    validate_metadata_broker_binding(metadata.as_ref(), &broker_spec)?;
    validate_metadata_broker_binding(m1_metadata.as_ref(), &broker_spec)?;
    let m1_quality = DataQualityReport::analyze(&m1_dataset);
    if m1_quality.grade == QualityGrade::Fail && !args.allow_failed_data {
        return Err(format!(
            "M1 data quality failed with score {}; pass --allow-failed-data to record an explicit override",
            m1_quality.score
        )
        .into());
    }

    // SQX-style: synthesize decision bars from M1 (exported H1 is only the open grid).
    let interval_ms = infer_median_interval_ms(&dataset.bars).unwrap_or(3_600_000);
    let grid: Vec<i64> = dataset.bars.iter().map(|bar| bar.timestamp_ms).collect();
    let dataset = build_timeframe_from_m1(&m1_dataset, interval_ms, Some(&grid))?;
    eprintln!(
        "built {} decision bars from M1 (SQX-style, interval {}ms)",
        dataset.bars.len(),
        interval_ms
    );

    let development = args
        .promotion_split
        .then(|| development_partition(&dataset, args.validation_fraction, args.sealed_fraction))
        .transpose()?;
    let oos1 = args
        .promotion_split
        .then(|| oos1_partition(&dataset, args.validation_fraction, args.sealed_fraction))
        .transpose()?;
    let search_dataset = development.as_ref().unwrap_or(&dataset);
    let oos1_ref = oos1.as_ref();
    let m1_is = args
        .promotion_split
        .then(|| clip_dataset_to_window(&m1_dataset, search_dataset))
        .transpose()?;
    let m1_eval = m1_is.as_ref().unwrap_or(&m1_dataset);

    let (bank, continuation_recipe_hash, starting_generation) = if args.continue_existing {
        reject_continuation_overrides(&args)?;
        let previous: EvolveArtifact = read_json(&args.databank)?;
        let recipe_hash = previous.manifest.recipe_hash.clone();
        let starting_generation = previous.databank.completed_generations;
        let evaluation_dataset = if previous.databank.data_hash == dataset.data_hash {
            &dataset
        } else {
            development.as_ref().ok_or(
                "this databank was built from an IS partition; enable --promotion-split to continue it",
            )?
        };
        let evaluation_oos1 = if previous.databank.data_hash == dataset.data_hash {
            None
        } else {
            oos1_ref
        };
        let evaluation_m1 = if previous.databank.execution_data_hash == m1_dataset.data_hash {
            &m1_dataset
        } else {
            m1_eval
        };
        (
            continue_evolution(
                previous.databank,
                evaluation_dataset,
                evaluation_oos1,
                evaluation_m1,
                &broker_spec,
                args.generations,
            )?,
            Some(recipe_hash),
            starting_generation,
        )
    } else {
        if args.databank.exists() {
            return Err(format!(
                "databank {} already exists; pass --continue to resume it",
                args.databank.display()
            )
            .into());
        }
        let config = new_discover_config(&args)?;
        (
            evolve_new(
                search_dataset,
                oos1_ref,
                m1_eval,
                &broker_spec,
                config,
                args.generations,
            )?,
            None,
            0,
        )
    };

    let mut manifest_config = BTreeMap::<String, Value>::from([
        ("source".into(), json!(display_path(&args.source.path))),
        ("broker".into(), json!(display_path(&args.broker))),
        ("databank".into(), json!(display_path(&args.databank))),
        ("engine_tier".into(), json!(quantforge_tick::ENGINE_TIER)),
        ("m1_source".into(), json!(display_path(&args.m1))),
        ("m1_data_hash".into(), json!(&m1_dataset.data_hash)),
        ("m1_quality_grade".into(), json!(m1_quality.grade)),
        ("m1_quality_score".into(), json!(m1_quality.score)),
        (
            "discover_config".into(),
            serde_json::to_value(&bank.config)?,
        ),
        ("generations_requested".into(), json!(args.generations)),
        ("starting_generation".into(), json!(starting_generation)),
        ("continued".into(), json!(args.continue_existing)),
        ("data_quality_grade".into(), json!(quality.grade)),
        ("data_quality_score".into(), json!(quality.score)),
        ("promotion_split".into(), json!(args.promotion_split)),
        (
            "validation_fraction".into(),
            json!(args.validation_fraction),
        ),
        ("sealed_fraction".into(), json!(args.sealed_fraction)),
        ("is_label".into(), json!("in_sample")),
        ("oos1_label".into(), json!("out_of_sample_1_pick")),
        ("oos2_label".into(), json!("out_of_sample_2_display")),
    ]);
    if let Some(metadata) = &metadata {
        manifest_config.insert("metadata_hash".into(), json!(&metadata.metadata_hash));
    }
    if let Some(recipe_hash) = continuation_recipe_hash {
        manifest_config.insert("continued_recipe_hash".into(), json!(recipe_hash));
    }
    let manifest = RunManifest::new(
        "evolve",
        RunRecipe {
            // Discovery is fitted against the persisted databank partition
            // (IS when promotion splitting is enabled), not the full source.
            data_hash: Some(bank.data_hash.clone()),
            broker_spec_hash: Some(bank.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: Some(bank.config.seed),
            config: manifest_config,
            override_flags: if args.allow_failed_data {
                vec!["allow_failed_data".into()]
            } else {
                Vec::new()
            },
        },
    )?;
    let artifact = EvolveArtifact {
        manifest,
        source: display_path(&args.source.path),
        broker: display_path(&args.broker),
        metadata_hash: metadata.map(|value| value.metadata_hash),
        data_quality: quality,
        coverage: bank.coverage(),
        qd_score: bank.qd_score(),
        databank: bank,
    };

    if args.continue_existing {
        let backup = write_json_versioned(&args.databank, &artifact)?;
        println!(
            "continued {} to generation {} ({} niches, {} evaluations)",
            args.databank.display(),
            artifact.databank.completed_generations,
            artifact.coverage,
            artifact.databank.evaluation_count
        );
        if let Some(backup) = backup {
            println!("preserved previous databank as {}", backup.display());
        }
    } else {
        write_json_new(&args.databank, &artifact)?;
        println!(
            "wrote {} at generation {} ({} niches, {} evaluations)",
            args.databank.display(),
            artifact.databank.completed_generations,
            artifact.coverage,
            artifact.databank.evaluation_count
        );
    }
    Ok(())
}

fn portfolio_command(args: PortfolioArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "portfolio artifact already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let (artifact, databank_source_hash) = read_json_hashed::<EvolveArtifact>(&args.databank)?;
    let broker: SymbolSpecification = read_json(&args.broker)?;
    broker.validate()?;
    let broker_spec_hash = broker.content_hash()?;
    verify_portfolio_databank(&artifact, &broker_spec_hash)?;
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
    let config = PortfolioConfig {
        objective: args.objective.into(),
        maximum_pairwise_correlation: args.maximum_pairwise_correlation,
        maximum_weight_per_strategy: args.maximum_weight_per_strategy,
        maximum_symbol_exposure: args.maximum_symbol_exposure,
        maximum_cohort_exposure: args.maximum_cohort_exposure,
        maximum_strategies: args.maximum_strategies,
        minimum_return_percent: args.minimum_return_percent,
        cvar_tail_fraction: args.cvar_tail_fraction,
        stress_trials: args.stress_trials,
        stress_block_length: args.stress_block_length,
        seed: args.seed,
    };
    let report = pack_portfolio(
        &candidates,
        artifact.databank.data_hash.clone(),
        broker_spec_hash.clone(),
        config,
    )?;
    let manifest = RunManifest::new(
        "portfolio",
        RunRecipe {
            data_hash: Some(artifact.databank.data_hash.clone()),
            broker_spec_hash: Some(broker_spec_hash),
            grammar_version: Some(artifact.databank.grammar_version.clone()),
            seed: Some(report.config.seed),
            config: BTreeMap::from([
                ("databank".into(), json!(display_path(&args.databank))),
                ("databank_source_hash".into(), json!(&databank_source_hash)),
                ("broker".into(), json!(display_path(&args.broker))),
                ("protocol".into(), json!(&report.protocol_version)),
                ("portfolio_id".into(), json!(&report.portfolio_id)),
                (
                    "portfolio_config".into(),
                    serde_json::to_value(&report.config)?,
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
            override_flags: Vec::new(),
        },
    )?;
    let output = PortfolioArtifact {
        manifest,
        databank_source: display_path(&args.databank),
        databank_source_hash,
        broker_source: display_path(&args.broker),
        report,
    };
    write_json_new(&args.out, &output)?;
    println!(
        "wrote {} ({} strategies, {:.4}% expected return, {:.4}% path drawdown, {:.4} max pairwise correlation)",
        args.out.display(),
        output.report.selected.len(),
        output.report.expected_return_percent,
        output.report.path_maximum_drawdown_percent,
        output.report.maximum_observed_pairwise_correlation
    );
    Ok(())
}

fn verify_portfolio_databank(
    artifact: &EvolveArtifact,
    broker_spec_hash: &quantforge_core::ContentHash,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    let bank = &artifact.databank;
    bank.validate_integrity()?;
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
            != Some(&serde_json::to_value(&bank.config)?)
    {
        return Err("portfolio requires an intact, promotion-grade MAP-Elites databank".into());
    }
    Ok(())
}

/// Diversification group for the portfolio exposure cap: entry-condition count
/// plus the trade-frequency and hold-time buckets.
fn behavior_cohort(elite: &quantforge_discover::Elite) -> String {
    format!(
        "e{}/{:?}/{:?}",
        elite.niche.entry_conditions, elite.niche.trade_frequency, elite.niche.hold_time
    )
    .to_ascii_lowercase()
}

#[derive(Serialize)]
struct VaultIdentityCheck<'a> {
    schema_version: u16,
    strategy_fingerprint: &'a quantforge_core::ContentHash,
    evidence_hash: &'a quantforge_core::ContentHash,
}

#[derive(Serialize)]
struct DeploymentIdentity<'a> {
    protocol_version: &'a str,
    certified_vault_entry_id: &'a quantforge_core::ContentHash,
    certified_vault_entry_hash: &'a quantforge_core::ContentHash,
    external_parity_artifact_hash: &'a quantforge_core::ContentHash,
    incubation_artifact_hash: &'a quantforge_core::ContentHash,
    candidate: &'a EvidenceBinding,
    files: &'a [DeploymentFileRecord],
}

fn deploy_command(args: DeployArgs) -> Result<(), Box<dyn Error>> {
    if args.out.exists() {
        return Err(format!(
            "deployment pack already exists and will not be replaced: {}",
            args.out.display()
        )
        .into());
    }
    let (entry, vault_entry_hash) =
        read_json_hashed::<CertifiedVaultEntry<VaultPayload>>(&args.vault_entry)?;
    let binding = verify_certified_vault_entry(&entry)?;

    let artifact_paths: Vec<_> = entry
        .payload
        .artifacts
        .iter()
        .map(|reference| PathBuf::from(&reference.path))
        .collect();
    let rebound = bind_gate_artifacts(&entry.payload.evidence, &artifact_paths)?;
    if rebound != entry.payload.artifacts {
        return Err("Certified Vault artifact references changed after admission".into());
    }
    let incubation_reference = entry
        .payload
        .artifacts
        .iter()
        .find(|reference| reference.gate == "incubation")
        .ok_or("Certified Vault entry has no paper incubation artifact")?;
    let (incubation, incubation_hash) =
        read_json_hashed::<IncubationFinalArtifact>(Path::new(&incubation_reference.path))?;
    if incubation_hash != incubation_reference.content_hash
        || Some(&incubation_hash)
            != entry
                .payload
                .evidence
                .incubation
                .as_ref()
                .map(|gate| &gate.artifact_hash)
    {
        return Err("paper incubation artifact does not match the Certified evidence".into());
    }
    verify_incubation_final_artifact(
        &incubation,
        &binding,
        &entry.payload.split_plan,
        &entry.payload.evidence.split_plan_hash,
    )?;
    let parity_reference = entry
        .payload
        .artifacts
        .iter()
        .find(|reference| reference.gate == "external_parity")
        .ok_or("Certified Vault entry has no external parity artifact")?;
    let (parity, parity_hash) =
        read_json_hashed::<ParityArtifact>(Path::new(&parity_reference.path))?;
    if parity_hash != parity_reference.content_hash
        || parity_hash != entry.payload.evidence.external_parity.gate.artifact_hash
    {
        return Err("external parity artifact does not match the Certified evidence".into());
    }
    verify_deployment_parity(
        &parity,
        &binding,
        &entry.payload.broker,
        &entry.payload.split_plan,
    )?;
    let generated = generate_bundle(
        &entry.payload.strategy,
        &entry.payload.broker,
        &parity.evidence.config,
    )?;
    if generated.evidence != parity.evidence
        || quantforge_core::ContentHash::sha256(generated.source.as_bytes())
            != parity.evidence.source_hash
    {
        return Err(
            "regenerated EA does not exactly match the source that passed external parity".into(),
        );
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
        operator_notice: "Research certification and passed paper incubation are not a profitability guarantee. AllowLiveTrading remains false; require independent operational review before enabling live orders.".into(),
    };
    let changelog = deployment_changelog(&entry, &parity_hash, &generated.evidence);
    let expert_name = &generated.evidence.expert_name;
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
        (PathBuf::from("CHANGELOG.md"), changelog.into_bytes()),
    ]);
    let file_records: Vec<_> = files
        .iter()
        .map(|(path, bytes)| DeploymentFileRecord {
            relative_path: path.to_string_lossy().into_owned(),
            content_hash: quantforge_core::ContentHash::sha256(bytes),
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
    })?;
    let run_manifest = RunManifest::new(
        "deploy",
        RunRecipe {
            data_hash: Some(entry.payload.split_plan.full_data_hash.clone()),
            broker_spec_hash: Some(binding.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("vault_entry".into(), json!(display_path(&args.vault_entry))),
                ("vault_entry_hash".into(), json!(&vault_entry_hash)),
                ("vault_entry_id".into(), json!(&entry.entry_id)),
                ("external_parity_hash".into(), json!(&parity_hash)),
                ("incubation_hash".into(), json!(&incubation_hash)),
                ("deployment_id".into(), json!(&deployment_id)),
                ("export_config".into(), json!(&generated.evidence.config)),
                ("files".into(), json!(&file_records)),
                ("live_trading_default".into(), json!(false)),
            ]),
            override_flags: Vec::new(),
        },
    )?;
    let manifest = DeploymentManifest {
        schema_version: DEPLOYMENT_SCHEMA_VERSION,
        protocol_version: DEPLOYMENT_PROTOCOL_VERSION.into(),
        deployment_id: deployment_id.clone(),
        grade: StrategyGrade::Deployed,
        run_manifest,
        certified_vault_entry_source: display_path(&args.vault_entry),
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
        pretty_json_bytes(&manifest)?,
    );
    write_directory_new(&args.out, &files)?;
    println!(
        "wrote Deployed pack {} to {} (live trading disabled)",
        deployment_id,
        args.out.display()
    );
    Ok(())
}

fn verify_certified_vault_entry(
    entry: &CertifiedVaultEntry<VaultPayload>,
) -> Result<EvidenceBinding, Box<dyn Error>> {
    entry.payload.manifest.validate()?;
    entry.payload.split_plan.validate()?;
    let strategy_fingerprint = entry
        .payload
        .strategy
        .structural_fingerprint(quantforge_core::FloatPolicy::default())?;
    let broker_spec_hash = entry.payload.broker.content_hash()?;
    let binding = EvidenceBinding {
        strategy_fingerprint,
        broker_spec_hash,
    };
    let policy: CertificationPolicy = serde_json::from_value(
        entry
            .payload
            .manifest
            .recipe
            .config
            .get("policy")
            .ok_or("Certified Vault manifest is missing its policy")?
            .clone(),
    )?;
    let decision =
        evaluate_certification(&entry.payload.evidence, &entry.payload.split_plan, &policy)?;
    let expected_entry_id = quantforge_core::stable_json_hash(&VaultIdentityCheck {
        schema_version: VAULT_SCHEMA_VERSION,
        strategy_fingerprint: &decision.candidate.strategy_fingerprint,
        evidence_hash: &decision.evidence_hash,
    })?;
    if entry.schema_version != VAULT_SCHEMA_VERSION
        || entry.payload.manifest.command != "certify"
        || !entry.payload.manifest.recipe.override_flags.is_empty()
        || entry.payload.manifest.recipe.data_hash.as_ref()
            != Some(&entry.payload.split_plan.full_data_hash)
        || entry.payload.manifest.recipe.broker_spec_hash.as_ref()
            != Some(&binding.broker_spec_hash)
        || entry.payload.evidence.candidate != binding
        || entry.payload.evidence.split_plan_hash != entry.payload.split_plan.content_hash()?
        || !policy.require_incubation
        || !entry
            .payload
            .evidence
            .incubation
            .as_ref()
            .is_some_and(|gate| gate.passed && gate.override_flags.is_empty())
        || entry.strategy_fingerprint != binding.strategy_fingerprint
        || entry.payload_hash != quantforge_core::stable_json_hash(&entry.payload)?
        || entry.entry_id != expected_entry_id
        || entry.certification != decision
        || !entry.certification.passed
        || entry.certification.resulting_grade != StrategyGrade::Certified
    {
        return Err("Vault entry is not an intact Certified artifact".into());
    }
    Ok(binding)
}

fn verify_deployment_parity(
    artifact: &ParityArtifact,
    binding: &EvidenceBinding,
    broker: &SymbolSpecification,
    split_plan: &DataSplitPlan,
) -> Result<(), Box<dyn Error>> {
    artifact.manifest.validate()?;
    artifact
        .mt5_metadata
        .validate_evidence(&artifact.evidence)?;
    let recomputed = compare_runs(
        &artifact.reference,
        &artifact.external,
        &artifact.evidence,
        artifact.report.tolerances.clone(),
    )?;
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
        return Err(
            "Certified external parity artifact is unsafe or internally inconsistent".into(),
        );
    }
    Ok(())
}

fn deployment_changelog(
    entry: &CertifiedVaultEntry<VaultPayload>,
    parity_hash: &quantforge_core::ContentHash,
    evidence: &ExportEvidenceCard,
) -> String {
    format!(
        "# QuantForge Deployment Changelog\n\n## Initial certified build\n\n- Vault entry: `{}`\n- Strategy fingerprint: `{}`\n- Broker specification: `{}`\n- External MT5 parity artifact: `{}`\n- Parity-passed EA source: `{}`\n- Expert: `{}`\n- Symbol/timeframe: `{}` / `{}`\n- Magic: `{}`\n- Paper incubation: `passed and reverified`\n- Live trading default: `false`\n\nThis pack reproduces the exact source and settings that passed external parity. Research certification and paper incubation are not guarantees of future performance. Complete independent operational review before enabling live orders.\n",
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

fn pretty_json_bytes(value: &impl Serialize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn new_discover_config(args: &EvolveArgs) -> Result<DiscoverConfig, Box<dyn Error>> {
    let commission = args
        .commission_per_lot_round_turn
        .ok_or("--commission-per-lot-round-turn is required when creating a new databank")?;
    let defaults = UniversalGrammarConfig::default();
    let universal_grammar = UniversalGrammarConfig {
        minimum_entry_conditions: args
            .minimum_entry_conditions
            .unwrap_or(defaults.minimum_entry_conditions),
        maximum_entry_conditions: args
            .maximum_entry_conditions
            .unwrap_or(defaults.maximum_entry_conditions),
        minimum_exit_conditions: args
            .minimum_exit_conditions
            .unwrap_or(defaults.minimum_exit_conditions),
        maximum_exit_conditions: args
            .maximum_exit_conditions
            .unwrap_or(defaults.maximum_exit_conditions),
        minimum_shift: defaults.minimum_shift,
        maximum_shift: defaults.maximum_shift,
    };
    Ok(DiscoverConfig {
        initial_candidates: args.initial.unwrap_or(500),
        batch_size: args.batch.unwrap_or(200),
        correlation_threshold: args.correlation.unwrap_or(0.85),
        novelty_weight: args.novelty_weight.unwrap_or(10.0),
        tournament_size: args.tournament_size.unwrap_or(4),
        structural_mutation_probability: args.structural_mutation_probability.unwrap_or(0.18),
        seed: args.seed.unwrap_or(42),
        universal_grammar,
        run_mode: parse_cli_run_mode(&args.run_mode)?,
        early_stop_pot_elites: None,
        target_databank_elites: None,
        trial_budget_warning: quantforge_discover::TRIAL_BUDGET_WARNING,
        gates: GateConfig {
            minimum_trades: args.minimum_trades.unwrap_or(10),
            maximum_drawdown_percent: args.maximum_drawdown_percent.unwrap_or(40.0),
            minimum_return_percent: args.minimum_return_percent.unwrap_or(0.0),
            minimum_profit_factor: args.minimum_profit_factor.unwrap_or(1.0),
            minimum_recovery_factor: args.minimum_return_drawdown.unwrap_or(0.0),
        },
        deposit_gates: GateConfig {
            minimum_trades: args.minimum_trades.unwrap_or(20),
            maximum_drawdown_percent: args.maximum_drawdown_percent.unwrap_or(30.0),
            minimum_return_percent: args.minimum_return_percent.unwrap_or(0.0),
            minimum_profit_factor: args.minimum_profit_factor.unwrap_or(1.0),
            minimum_recovery_factor: args.minimum_return_drawdown.unwrap_or(0.0),
        },
        precision: quantforge_discover::PrecisionGateConfig {
            minimum_return_retention: args.minimum_m1_return_retention.unwrap_or(0.80),
        },
        search_ranges: quantforge_discover::SearchRangeProfile::default(),
        oos1_expectancy_retention: 0.7,
        require_m1_precision: false,
        simple_exits: true,
        allow_break_even: false,
        allow_trailing_stops: false,
        allow_partial_exits: false,
        allow_market_entries: true,
        allow_stop_entries: false,
        allow_limit_entries: false,
        allow_stop_limit_entries: false,
        flatten_at_22: args.flatten_at_22,
        end_of_day_hour: args.end_of_day_hour,
        max_one_entry_per_day: true,
        mutate_after_elites: 300,
        random_fill_fraction: 0.4,
        worker_threads: 0,
        require_m1_robustness: true,
        robustness_folds: 3,
        robustness_monte_carlo_trials: 250,
        robustness_neighborhood_samples: 8,
        robustness_perturbation_fraction: args
            .robustness_perturbation_fraction
            .unwrap_or(quantforge_discover::PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION),
        minimum_neighborhood_survival_fraction: 0.7,
        calendar_year_folds: false,
        minimum_deflated_trade_sharpe: None,
        multi_symbol_minimum_pass: 0,
        enable_cheap_prefilter: false,
        prefilter_bar_fraction: 0.25,
        prefilter_gates: GateConfig::prefilter_defaults(),
        island_count: 1,
        migration_interval: 0,
        migration_elites: 2,
        scout: ScoutConfig {
            initial_balance: args.initial_balance.unwrap_or(100_000.0),
            same_bar_policy: quantforge_eval::SameBarPolicy::Conservative,
            costs: CostModel {
                fallback_spread_points: args.fallback_spread_points,
                adverse_slippage_points_per_side: args.slippage_points_per_side.unwrap_or(0.0),
                commission_per_lot_round_turn: commission,
                max_spread_points: args.max_spread_points,
                include_costs_in_risk: true,
            },
            indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
            entry_window: quantforge_eval::EntryWindow::new(
                args.entry_window_start_hour,
                args.entry_window_end_hour,
            ),
            // Search overrides this per batch; the CLI default keeps full metrics.
            abandon_above_drawdown_percent: None,
        },
    })
}

fn development_partition(
    dataset: &BarDataset,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<BarDataset, Box<dyn Error>> {
    let plan = DataSplitPlan::chronological(dataset, validation_fraction, sealed_fraction)?;
    slice_partition(dataset, 0, plan.development.bar_count)
}

fn oos1_partition(
    dataset: &BarDataset,
    validation_fraction: f64,
    sealed_fraction: f64,
) -> Result<BarDataset, Box<dyn Error>> {
    let plan = DataSplitPlan::chronological(dataset, validation_fraction, sealed_fraction)?;
    let start = plan.development.bar_count;
    let end = start + plan.validation.bar_count;
    slice_partition(dataset, start, end)
}

fn slice_partition(
    dataset: &BarDataset,
    start: usize,
    end: usize,
) -> Result<BarDataset, Box<dyn Error>> {
    if end <= start || end > dataset.bars.len() {
        return Err(format!(
            "invalid partition slice {start}..{end} for {} bars",
            dataset.bars.len()
        )
        .into());
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

fn clip_dataset_to_window(
    dataset: &BarDataset,
    window: &BarDataset,
) -> Result<BarDataset, Box<dyn Error>> {
    let (Some(first), Some(last)) = (window.bars.first(), window.bars.last()) else {
        return Err("cannot clip M1: IS window is empty".into());
    };
    let start_ms = first.timestamp_ms;
    let end_ms = last.timestamp_ms;
    let bars: Vec<_> = dataset
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms >= start_ms && bar.timestamp_ms <= end_ms)
        .cloned()
        .collect();
    if bars.len() < 2 {
        return Err(format!(
            "M1 has fewer than 2 bars inside the IS window [{start_ms}..{end_ms}]"
        )
        .into());
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

fn reject_continuation_overrides(args: &EvolveArgs) -> Result<(), Box<dyn Error>> {
    if args.initial.is_some()
        || args.batch.is_some()
        || args.correlation.is_some()
        || args.novelty_weight.is_some()
        || args.tournament_size.is_some()
        || args.structural_mutation_probability.is_some()
        || args.seed.is_some()
        || args.minimum_trades.is_some()
        || args.maximum_drawdown_percent.is_some()
        || args.minimum_return_percent.is_some()
        || args.minimum_profit_factor.is_some()
        || args.minimum_return_drawdown.is_some()
        || args.minimum_m1_return_retention.is_some()
        || args.flatten_at_22
        || args.commission_per_lot_round_turn.is_some()
        || args.slippage_points_per_side.is_some()
        || args.fallback_spread_points.is_some()
        || args.max_spread_points.is_some()
        || args.initial_balance.is_some()
    {
        return Err(
            "a continuation uses the databank's immutable stored search, gate and cost configuration; pass only --generations"
                .into(),
        );
    }
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, Box<dyn Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn validate_metadata_broker_binding(
    metadata: Option<&Mt5ExportMetadata>,
    broker: &SymbolSpecification,
) -> Result<(), Box<dyn Error>> {
    let Some(metadata) = metadata else {
        return Ok(());
    };
    if metadata.required("symbol")? != broker.symbol {
        return Err(format!(
            "metadata symbol {} does not match broker profile {}",
            metadata.required("symbol")?,
            broker.symbol
        )
        .into());
    }
    if metadata.source_timezone()?.name() != broker.timezone {
        return Err(format!(
            "metadata timezone {} does not match broker profile {}",
            metadata.source_timezone()?,
            broker.timezone
        )
        .into());
    }
    for (property, expected) in [
        ("account_currency", broker.account_currency.as_str()),
        ("currency_base", broker.base_currency.as_str()),
        ("currency_profit", broker.profit_currency.as_str()),
        ("currency_margin", broker.margin_currency.as_str()),
    ] {
        if let Some(actual) = metadata.properties.get(property)
            && actual != expected
        {
            return Err(format!(
                "metadata {property} {actual} does not match broker profile {expected}"
            )
            .into());
        }
    }
    for (property, day) in [
        ("swap_multiplier_sunday", DayOfWeek::Sunday),
        ("swap_multiplier_monday", DayOfWeek::Monday),
        ("swap_multiplier_tuesday", DayOfWeek::Tuesday),
        ("swap_multiplier_wednesday", DayOfWeek::Wednesday),
        ("swap_multiplier_thursday", DayOfWeek::Thursday),
        ("swap_multiplier_friday", DayOfWeek::Friday),
        ("swap_multiplier_saturday", DayOfWeek::Saturday),
    ] {
        if let Some(actual) = metadata.properties.get(property) {
            let actual = actual
                .parse::<f64>()
                .map_err(|_| format!("metadata {property} is not numeric"))?;
            if (actual - f64::from(broker.swap_multiplier(day))).abs() > 1.0e-9 {
                return Err(format!(
                    "metadata {property} {actual} does not match broker profile {}",
                    broker.swap_multiplier(day)
                )
                .into());
            }
        }
    }
    Ok(())
}

fn load_source(
    source: &DataSourceArgs,
) -> Result<(BarDataset, Option<Mt5ExportMetadata>), Box<dyn Error>> {
    let metadata = source
        .metadata
        .as_ref()
        .map(Mt5ExportMetadata::load)
        .transpose()?;
    let timezone = match (&metadata, source.source_timezone) {
        (Some(metadata), None) => metadata.source_timezone()?,
        (None, Some(timezone)) => timezone,
        _ => return Err("provide exactly one of --metadata or --source-timezone".into()),
    };
    let dataset = BarDataset::load_mt5(&source.path, timezone)?;
    if let Some(metadata) = &metadata {
        metadata.validate_dataset(&dataset)?;
    }
    Ok((dataset, metadata))
}

fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn print_json(value: &impl Serialize) -> Result<(), Box<dyn Error>> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    serde_json::to_writer_pretty(&mut output, value)?;
    output.write_all(b"\n")?;
    Ok(())
}
