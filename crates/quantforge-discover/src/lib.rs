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
    FamilyBakeoffConfig, FamilyBakeoffReport, FamilyBakeoffRow, run_family_bakeoff,
};
pub use engine::{
    continue_evolution, continue_evolution_with_pack, evolve_new, evolve_new_with_pack,
};
pub use grammar::{generate_seed, generate_seed_for_family, mutate_strategy};
pub use methodology::{
    FactorCellSummary, FactorContrast, FactorDraw, FactorRecipe, MethodologyGridConfig,
    MethodologyReport, run_methodology_grid,
};
pub use model::{
    BehaviorDescriptor, Databank, DepositDecision, DiscoverConfig, DiscoverError,
    DiscoverRunMode, DiscoverTelemetry, Elite, EvidenceComponents, FamilyStyle, GateConfig,
    GateResult, LongShortSkewBucket, NicheKey, PrecisionGateConfig, SearchFamily,
    SearchFamilySpec, SymbolScreenResult, ThreeLevelBucket, TRIAL_BUDGET_WARNING,
};
pub use multi_symbol::{DEFAULT_FX_PACK, DISPLAY_ONLY_SYMBOLS, PackSymbol, screen_multi_symbol};
pub use permutation::{
    PermutationNullConfig, PermutationNullReport, run_permutation_null, stationary_bootstrap_bars,
};

pub const DATABANK_SCHEMA_VERSION: u16 = 5;
/// Locked Search Families: completed-bar signals, next-open market entry,
/// ATR-period 14 protective exits and mandatory M1 promotion checks.
pub const GRAMMAR_VERSION: &str = "search-families-v5-selected-tf-parity";
pub const FIXED_RISK_PER_TRADE: f64 = 1_000.0;
/// Frozen ATR lookback inside institutional Search Families.
pub const FROZEN_ATR_PERIOD: u16 = 14;
