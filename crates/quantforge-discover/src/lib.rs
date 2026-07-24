//! Deterministic quality-diversity generation and the MAP-Elites databank.

mod archive;
mod engine;
mod grammar;
mod model;
mod robustness;

pub use archive::niche_label;
pub use engine::{continue_evolution, evolve_new};
pub use grammar::{generate_seed, mutate_strategy};
pub use model::{
    BehaviorDescriptor, Databank, DepositDecision, DiscoverConfig, DiscoverError,
    DiscoverTelemetry, Elite, EvidenceComponents, FamilyStyle, GateConfig, LongShortSkewBucket,
    NicheKey, PrecisionGateConfig, ThreeLevelBucket,
};

pub const DATABANK_SCHEMA_VERSION: u16 = 4;
pub const GRAMMAR_VERSION: &str = "m1-verified-fixed-risk-close-v5";
pub const FIXED_RISK_PER_TRADE: f64 = 1_000.0;
