//! Deterministic completed-bar OHLC scout evaluator.

mod costs;
mod engine;
mod features;
mod model;

pub use costs::{ResolvedSpread, SpreadSource, SwapAccrual, accrue_swap, resolve_spread};
pub use engine::{equity_sharpe_ratio, evaluate_strategy, evaluate_strategy_from};
pub use features::{FeatureCache, calculate_indicator_series};
pub use model::{
    BacktestMetrics, CostModel, EquityPoint, EvalError, ExitReason, MANDATORY_ENTRY_WINDOW_END_HOUR,
    MANDATORY_ENTRY_WINDOW_START_HOUR, PositionSide, SameBarPolicy, ScoutConfig, ScoutResult,
    ScoutTelemetry, Trade, in_mandatory_entry_window,
};

pub const ENGINE_TIER: &str = "ohlc-scout";
