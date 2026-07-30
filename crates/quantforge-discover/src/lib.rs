//! Deterministic quality-diversity generation and the MAP-Elites databank.

mod archive;
mod bakeoff;
mod engine;
mod grammar;
mod methodology;
mod model;
mod multi_symbol;
mod permutation;
mod robustness;

pub use archive::niche_label;
pub use bakeoff::{
    ConditionBakeoffConfig, ConditionBakeoffReport, ConditionBakeoffRow, run_condition_bakeoff,
};
pub use engine::{
    EvolutionSession, continue_evolution, continue_evolution_with_pack, evolve_new,
    evolve_new_with_pack,
};
pub use grammar::{generate_seed, generate_seed_for_family, mutate_strategy};
pub use methodology::{
    FactorCellSummary, FactorContrast, FactorDraw, FactorRecipe, MethodologyGridConfig,
    MethodologyReport, run_methodology_grid,
};
pub use model::{
    BehaviorDescriptor, Databank, DepositDecision, DiscoverConfig, DiscoverError,
    DiscoverRunMode, DiscoverTelemetry, Elite, EvidenceComponents, FamilyStyle, GateConfig,
    GateResult, LongShortSkewBucket, M1RetentionEvidence, NicheKey,
    ParameterNeighborhoodEvidence, ParameterNeighborhoodSample, PrecisionGateConfig,
    RobustnessEvidence, SearchFamily,
    SearchRange, SearchRangeProfile, SearchFamilySpec, SymbolScreenResult, ThreeLevelBucket,
    UniversalGrammarConfig, WalkForwardEvidence, WalkForwardFold, TRIAL_BUDGET_WARNING,
};
pub use multi_symbol::{DEFAULT_FX_PACK, DISPLAY_ONLY_SYMBOLS, PackSymbol, screen_multi_symbol};
pub use permutation::{
    PermutationNullConfig, PermutationNullReport, run_permutation_null, stationary_bootstrap_bars,
};
pub use robustness::{
    MONTE_CARLO_SKIP_TRADE_PROBABILITY, PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION,
    RobustnessConfig, RobustnessOutcome, RobustnessReject, run_m1_predeposit_robustness,
};

pub const DATABANK_SCHEMA_VERSION: u16 = 6;
/// Family-free typed block grammar: completed-bar signals, next-open market
/// entry, ATR-period 14 protective exits, mandatory M1 promotion checks and
/// MAP-Elites niching on the entry-condition count.
pub const GRAMMAR_VERSION: &str = "universal-v6-condition-count";
pub const FIXED_RISK_PER_TRADE: f64 = 1_000.0;
/// Frozen ATR lookback inside institutional Search Families.
pub const FROZEN_ATR_PERIOD: u16 = 14;
