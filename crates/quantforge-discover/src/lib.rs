//! Deterministic quality-diversity generation and the MAP-Elites databank.

mod archive;
mod bakeoff;
mod engine;
mod fold_r;
mod grammar;
mod holding_corr;
mod islands;
mod meta_learning;
mod methodology;
mod model;
mod multi_symbol;
mod permutation;
mod production_bakeoff;
mod production_lane;
mod robustness;
mod timeframe_bakeoff;

pub use archive::{entry_family_key, niche_label};
pub use bakeoff::{
    ConditionBakeoffConfig, ConditionBakeoffReport, ConditionBakeoffRow, run_condition_bakeoff,
};
pub use engine::{
    EvolutionSession, HoldingBatteryAuditResult, HoldingBatteryReject, HoldingBatteryResult,
    HoldingBypassResult,
    audit_holding_battery,
    continue_evolution, continue_evolution_with_pack, evolve_new, evolve_new_with_pack,
    evolve_new_with_pack_and_quotes, holding_factory_score, new_databank,
    promote_all_holding_without_robustness, promote_selected_holding_without_robustness,
    run_holding_battery_and_promote,
};
pub use fold_r::FoldRStats;
pub use grammar::{generate_seed, generate_seed_for_family, mutate_strategy};
pub use holding_corr::{
    HoldingCorrShrinkReport, align_daily_pnl, apply_holding_daily_corr_shrink,
    daily_pnl_from_trades,
};
pub use meta_learning::{
    MetaCalibrationBin, MetaCandidate, MetaDataset, MetaDatasetRow, MetaDatasetScope,
    MetaEvaluationReport, MetaExpectancyEvaluationReport, MetaExpectancyModel,
    MetaExpectancyPrediction, MetaExpectancyWalkForwardEpisode, MetaExpectancyWalkForwardReport,
    MetaFeatureRecord, MetaFutureOutcome, MetaLabel, MetaLearningConfig, MetaLearningError,
    MetaLearningInput, MetaLogisticModel, MetaPrediction, MetaReplayCandidate, MetaReplayOrigin,
    MetaReplayWindow, MetaWalkForwardEpisode, MetaWalkForwardReport, MetaWindow, MetaWindowRole,
    build_meta_dataset, build_meta_learning_input_from_replay, run_meta_expectancy_walk_forward,
    run_meta_walk_forward,
};
pub use methodology::{
    FactorCellSummary, FactorContrast, FactorDraw, FactorRecipe, MethodologyGridConfig,
    MethodologyReport, run_methodology_grid,
};
pub use model::{
    BehaviorDescriptor, Databank, DepositDecision, DiscoverConfig, DiscoverError, DiscoverRunMode,
    DiscoverTelemetry, Elite, EvidenceComponents, FamilyStyle, GateConfig, GateResult,
    LongShortSkewBucket, M1RetentionEvidence, NicheKey, ParameterNeighborhoodEvidence,
    ParameterNeighborhoodSample, PrecisionGateConfig, RobustnessEvidence, SearchFamily,
    SearchFamilySpec, SearchRange, SearchRangeProfile, SymbolScreenResult, TRIAL_BUDGET_WARNING,
    ThreeLevelBucket, UniversalGrammarConfig, WalkForwardEvidence, WalkForwardFold,
    default_history_start_year,
};
pub use multi_symbol::{DEFAULT_FX_PACK, DISPLAY_ONLY_SYMBOLS, PackSymbol, screen_multi_symbol};
pub use permutation::{
    PermutationNullConfig, PermutationNullReport, run_permutation_null, stationary_bootstrap_bars,
};
pub use production_bakeoff::{
    ProductionBakeoffArmReport, ProductionBakeoffConfig, ProductionBakeoffDecision,
    ProductionBakeoffReport, ProductionBakeoffStrictInput, ProductionBakeoffStrictSummary,
    ProductionBakeoffSummary, SealedCandidateResult, run_production_bakeoff,
};
pub use production_lane::{
    PRODUCTION_LANE_ID, PRODUCTION_LANE_SCHEMA_VERSION, PRODUCTION_LANE_SCORE_FORMULA,
    ProductionLaneCandidateRow, ProductionLaneConfig, ProductionLaneReplay, ProductionLaneReport,
    ProductionLaneWindow, ProductionLaneWindowSummary, run_production_lane,
};
pub use robustness::{
    MONTE_CARLO_MAX_DRAWDOWN_RATIO, MONTE_CARLO_P80_PROFIT_RETENTION,
    MONTE_CARLO_SKIP_TRADE_PROBABILITY, PARAM_RECOVERY_MEDIAN_HIGH, PARAM_RECOVERY_MEDIAN_LOW,
    PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION, RobustnessAuditOutcome, RobustnessConfig,
    RobustnessOutcome, RobustnessReject, development_cpcv_diagnostic,
    passes_recovery_median_band, recovery_to_median_ratio, run_m1_holding_admission,
    run_m1_predeposit_robustness, run_m1_predeposit_robustness_audit,
};
pub use timeframe_bakeoff::{
    TimeframeAblationComparison, TimeframeAblationConfig, TimeframeAblationReport,
    TimeframeAblationRow, TimeframeBakeoffConfig, TimeframeBakeoffLaneRow, TimeframeBakeoffPair,
    TimeframeBakeoffReport, TimeframeGateConfig, TimeframeRollingReport, TimeframeRollingRow,
    TimeframeRollingWindow, TimeframeSelectionMode, run_timeframe_ablation, run_timeframe_bakeoff,
    run_timeframe_rolling_ablation,
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
