use quantforge_broker::BrokerSpecError;
use quantforge_ir::IrError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SameBarPolicy {
    /// When both protective levels are touched and path order is unknown, the
    /// stop loss wins.
    Conservative,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    /// Used only when the bar has no recorded spread.
    pub fallback_spread_points: Option<f64>,
    pub adverse_slippage_points_per_side: f64,
    pub commission_per_lot_round_turn: f64,
    pub max_spread_points: Option<f64>,
    /// Position size solves for stop loss plus estimated round-turn commission
    /// and entry/exit slippage inside the configured risk budget.
    pub include_costs_in_risk: bool,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            fallback_spread_points: None,
            adverse_slippage_points_per_side: 0.0,
            commission_per_lot_round_turn: 0.0,
            max_spread_points: None,
            include_costs_in_risk: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoutConfig {
    pub initial_balance: f64,
    pub same_bar_policy: SameBarPolicy,
    pub costs: CostModel,
}

impl Default for ScoutConfig {
    fn default() -> Self {
        Self {
            initial_balance: 100_000.0,
            same_bar_policy: SameBarPolicy::Conservative,
            costs: CostModel::default(),
        }
    }
}

impl ScoutConfig {
    pub fn validate(&self) -> Result<(), EvalError> {
        if !self.initial_balance.is_finite() || self.initial_balance <= 0.0 {
            return Err(EvalError::InvalidConfig(
                "initial_balance must be finite and greater than zero".into(),
            ));
        }
        for (name, value) in [
            (
                "adverse_slippage_points_per_side",
                self.costs.adverse_slippage_points_per_side,
            ),
            (
                "commission_per_lot_round_turn",
                self.costs.commission_per_lot_round_turn,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(EvalError::InvalidConfig(format!(
                    "{name} must be finite and non-negative"
                )));
            }
        }
        for (name, value) in [
            ("fallback_spread_points", self.costs.fallback_spread_points),
            ("max_spread_points", self.costs.max_spread_points),
        ] {
            if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
                return Err(EvalError::InvalidConfig(format!(
                    "{name} must be finite and non-negative"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositionSide {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    StopLoss,
    TakeProfit,
    Indicator,
    TimeStop,
    EndOfDay,
    PartialExit,
    EndOfData,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub side: PositionSide,
    pub entry_timestamp_ms: i64,
    pub exit_timestamp_ms: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub volume: f64,
    pub initial_stop_loss: f64,
    pub initial_take_profit: f64,
    pub gross_profit: f64,
    pub commission: f64,
    #[serde(default)]
    pub swap: f64,
    pub net_profit: f64,
    pub bars_held: usize,
    pub exit_reason: ExitReason,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquityPoint {
    pub timestamp_ms: i64,
    pub balance: f64,
    pub equity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestMetrics {
    pub initial_balance: f64,
    pub ending_balance: f64,
    pub net_profit: f64,
    pub return_percent: f64,
    pub trade_count: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub win_rate: f64,
    pub profit_factor: Option<f64>,
    pub max_drawdown: f64,
    pub max_drawdown_percent: f64,
    #[serde(default)]
    pub sharpe_ratio: Option<f64>,
    /// Mean net profit per closed trade (account currency). Zero when no trades.
    #[serde(default)]
    pub expectancy: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutTelemetry {
    pub conflicting_entry_signals: usize,
    pub skipped_outside_session: usize,
    pub skipped_for_spread: usize,
    pub skipped_for_broker_stop_level: usize,
    pub skipped_below_minimum_volume: usize,
    pub pending_orders_placed: usize,
    pub pending_orders_filled: usize,
    pub pending_orders_expired: usize,
    pub partial_exits_executed: usize,
    pub break_even_moves: usize,
    pub trailing_stop_moves: usize,
    pub end_of_day_flattens: usize,
    pub synthetic_spread_bars: usize,
    pub fallback_spread_bars: usize,
    pub swap_rollover_events: usize,
    pub swap_effective_days: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoutResult {
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
    pub metrics: BacktestMetrics,
    pub telemetry: ScoutTelemetry,
}

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("at least two bars are required")]
    InsufficientBars,
    #[error("invalid scout configuration: {0}")]
    InvalidConfig(String),
    #[error("bar at timestamp {timestamp_ms} has no spread and no fallback was configured")]
    MissingSpread { timestamp_ms: i64 },
    #[error("unsupported broker feature in scout v1: {0}")]
    UnsupportedBrokerFeature(&'static str),
    #[error("strategy side is incompatible with broker trade mode")]
    IncompatibleBrokerTradeMode,
    #[error("broker timezone is not a valid IANA timezone: {0}")]
    InvalidBrokerTimezone(String),
    #[error("indicator serialization failed: {0}")]
    IndicatorKey(#[from] serde_json::Error),
    #[error(transparent)]
    Broker(#[from] BrokerSpecError),
    #[error(transparent)]
    Ir(#[from] IrError),
}
