//! Deterministic quality-diversity generation and the MAP-Elites databank.

mod archive;
mod bakeoff;
mod engine;
mod fold_r;
mod grammar;
mod holding_corr;
mod islands;
mod methodology;
mod model;
mod multi_symbol;
mod permutation;
mod robustness;

pub use archive::{entry_family_key, niche_label};
pub use bakeoff::{
    ConditionBakeoffConfig, ConditionBakeoffReport, ConditionBakeoffRow, run_condition_bakeoff,
};
pub use fold_r::FoldRStats;
pub use holding_corr::{
    HoldingCorrShrinkReport, align_daily_pnl, apply_holding_daily_corr_shrink,
    daily_pnl_from_trades,
};
pub use engine::{
    EvolutionSession, HoldingBatteryReject, HoldingBatteryResult, continue_evolution,
    continue_evolution_with_pack, evolve_new, evolve_new_with_pack,
    evolve_new_with_pack_and_quotes, holding_factory_score, new_databank,
    run_holding_battery_and_promote,
};
pub use grammar::{generate_seed, generate_seed_for_family, mutate_strategy};
pub use methodology::{
    FactorCellSummary, FactorContrast, FactorDraw, FactorRecipe, MethodologyGridConfig,
    MethodologyReport, run_methodology_grid,
};
pub use model::{
    BehaviorDescriptor, Databank, DepositDecision, DiscoverConfig, DiscoverError, DiscoverRunMode,
    DiscoverTelemetry, Elite, EvidenceComponents, FamilyStyle, GateConfig, GateResult,
    LongShortSkewBucket, M1RetentionEvidence, NicheKey, ParameterNeighborhoodEvidence,
    ParameterNeighborhoodSample, PrecisionGateConfig, RobustnessEvidence, ScoutFitnessMode,
    SearchFamily,
    SearchFamilySpec, SearchRange, SearchRangeProfile, SymbolScreenResult, TRIAL_BUDGET_WARNING,
    ThreeLevelBucket, UniversalGrammarConfig, WalkForwardEvidence, WalkForwardFold,
    default_history_start_year,
};
pub use multi_symbol::{DEFAULT_FX_PACK, DISPLAY_ONLY_SYMBOLS, PackSymbol, screen_multi_symbol};
pub use permutation::{
    PermutationNullConfig, PermutationNullReport, run_permutation_null, stationary_bootstrap_bars,
};
pub use robustness::{
    MONTE_CARLO_MAX_DRAWDOWN_RATIO, MONTE_CARLO_P80_PROFIT_RETENTION,
    MONTE_CARLO_SKIP_TRADE_PROBABILITY, OPT_PROFILE_MAX_TESTS, OPT_PROFILE_PROFIT_OPT_PCT,
    OPT_PROFILE_STEPS, PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION, PARAM_RECOVERY_MEDIAN_HIGH,
    PARAM_RECOVERY_MEDIAN_LOW, RobustnessConfig, RobustnessOutcome, RobustnessReject,
    SEQUENTIAL_WALK_FORWARD_PASS_FRACTION, development_cpcv_diagnostic,
    passes_recovery_median_band, recovery_to_median_ratio, run_m1_holding_admission,
    run_m1_predeposit_robustness,
};

pub const DATABANK_SCHEMA_VERSION: u16 = 6;
/// Family-free typed block grammar: completed-bar signals, next-open entries,
/// searchable ATR/R exit geometry, mandatory M1 promotion checks and
/// MAP-Elites niching on the entry-condition count.
pub const GRAMMAR_VERSION: &str = "universal-v6-condition-count";
pub const FIXED_RISK_PER_TRADE: f64 = 1_000.0;
/// Legacy fallback used by older methodology artifacts. New Discover runs use
/// the sealed SearchRangeProfile ATR period instead of freezing this value.
pub const FROZEN_ATR_PERIOD: u16 = 14;
