//! Deterministic completed-bar OHLC scout evaluator.

mod costs;
mod engine;
mod features;
mod management;
mod model;
mod position_book;
mod tick_path;

pub use costs::{ResolvedSpread, SpreadSource, SwapAccrual, accrue_swap, resolve_spread};
pub use engine::{
    calculate_metrics, equity_sharpe_ratio, evaluate_strategy, evaluate_strategy_cached,
    evaluate_strategy_from, evaluate_strategy_with_ticks,
};
pub use features::{
    DEFAULT_INDICATOR_CACHE_BYTES, FeatureCache, IndicatorBufferCache, calculate_indicator_series,
};
pub use management::{
    favorable_r, favorable_sample_from_decision_bar, favorable_sample_from_m1_window,
    normalize_price, placeable_stop_candidate, price_reaches_from_above, price_reaches_from_below,
    ratchet_favorable_peak, stop_would_trigger_at_open,
};
pub use model::{
    BacktestMetrics, CostModel, EntryWindow, EquityPoint, EvalError, ExitReason, FillSimulation,
    MANDATORY_ENTRY_WINDOW_END_HOUR, MANDATORY_ENTRY_WINDOW_START_HOUR, PositionAccounting,
    PositionSide, SameBarPolicy, ScoutConfig, ScoutResult, ScoutTelemetry, Trade,
    in_mandatory_entry_window, IndicatorEngine,
};
pub use tick_path::{
    SyntheticTick, Tick, TickDataset, everytick_stop_hit_first, load_tick_csv, parse_tick_csv,
    ohlc_everytick_path, tick_file_stop_hit_first, ticks_in_bar_window,
};

pub const ENGINE_TIER: &str = "ohlc-scout";
