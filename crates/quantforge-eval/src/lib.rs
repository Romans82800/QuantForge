//! Deterministic completed-bar OHLC scout evaluator.

mod costs;
mod engine;
mod features;
mod model;

pub use costs::{ResolvedSpread, SpreadSource, SwapAccrual, accrue_swap, resolve_spread};
pub use engine::{evaluate_strategy, evaluate_strategy_from};
pub use features::{FeatureCache, calculate_indicator_series};
pub use model::{
    BacktestMetrics, CostModel, EquityPoint, EvalError, ExitReason, PositionSide, SameBarPolicy,
    ScoutConfig, ScoutResult, ScoutTelemetry, Trade,
};

pub const ENGINE_TIER: &str = "ohlc-scout";
