use quantforge_broker::BrokerSpecError;
use quantforge_ir::IrError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SameBarPolicy {
    /// When both protective levels are touched and path order is unknown, the
    /// stop loss wins.
    #[default]
    Conservative,
    /// Walk a synthetic EveryTick OHLC path (Open → extreme1 → extreme2 → Close)
    /// to resolve same-bar stop/target collisions. Foundation for true tick files.
    EveryTickOhlc,
}

/// How open positions are accounted for during Scout / Judge replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PositionAccounting {
    /// At most one open position (current QuantForge default / MT5 hedged single).
    #[default]
    HedgedSingle,
    /// Multiple concurrent positions up to [`ScoutConfig::max_open_positions`].
    HedgedStack,
    /// One net position; an opposite signal closes the open trade before a new entry.
    Netting,
}

/// Configurable fill / requote simulation (idealized broker realism hook).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillSimulation {
    /// Fraction of intended volume filled on entry (`1.0` = full fill).
    #[serde(default = "default_fill_fraction")]
    pub fill_volume_fraction: f64,
    /// Deterministic probability `[0, 1]` that a market entry is requoted.
    #[serde(default)]
    pub requote_probability: f64,
    /// Extra adverse slippage (points) applied when a requote fires.
    #[serde(default)]
    pub requote_extra_slippage_points: f64,
    /// When true, a requote rejects the entry instead of filling worse.
    #[serde(default)]
    pub requote_rejects: bool,
}

fn default_fill_fraction() -> f64 {
    1.0
}

impl Default for FillSimulation {
    fn default() -> Self {
        Self {
            fill_volume_fraction: 1.0,
            requote_probability: 0.0,
            requote_extra_slippage_points: 0.0,
            requote_rejects: false,
        }
    }
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
    /// Optional partial-fill / requote simulation (default: ideal full fills).
    #[serde(default)]
    pub fill_simulation: FillSimulation,
}

impl Default for CostModel {
    fn default() -> Self {
        Self {
            fallback_spread_points: None,
            adverse_slippage_points_per_side: 0.0,
            commission_per_lot_round_turn: 0.0,
            max_spread_points: None,
            include_costs_in_risk: true,
            fill_simulation: FillSimulation::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IndicatorEngine {
    /// MT5 built-in semantics (rolling-mean ATR, standard iADX).
    Mt5,
    /// StrategyQuant Sq* indicator math (default for parity).
    #[default]
    Sqx,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoutConfig {
    pub initial_balance: f64,
    pub same_bar_policy: SameBarPolicy,
    pub costs: CostModel,
    #[serde(default)]
    pub indicator_engine: IndicatorEngine,
    /// Broker-local hours in which new entries and pending orders may be placed.
    #[serde(default)]
    pub entry_window: EntryWindow,
    /// Stop replaying once equity drawdown passes this percentage.
    ///
    /// Drawdown never recovers downward, so a run past the ceiling can no longer
    /// satisfy a gate that caps drawdown at or below it. Search sets this to its
    /// drawdown gate to skip the remainder of a doomed backtest; the returned
    /// metrics are then truncated and only valid for rejecting the candidate.
    /// Leave `None` whenever the metrics themselves are the output.
    #[serde(default)]
    pub abandon_above_drawdown_percent: Option<f64>,
    /// Hedged-single / hedged-stack / netting accounting.
    #[serde(default)]
    pub position_accounting: PositionAccounting,
    /// Cap for [`PositionAccounting::HedgedStack`] (ignored for single/netting).
    #[serde(default = "default_max_open_positions")]
    pub max_open_positions: usize,
    /// When true (and a tick dataset is supplied to evaluate), protective
    /// same-bar collisions walk real tick bids/asks inside each bar window.
    #[serde(default)]
    pub enable_tick_file_replay: bool,
}

fn default_max_open_positions() -> usize {
    1
}

impl Default for ScoutConfig {
    fn default() -> Self {
        Self {
            initial_balance: 100_000.0,
            same_bar_policy: SameBarPolicy::Conservative,
            costs: CostModel::default(),
            indicator_engine: IndicatorEngine::Sqx,
            entry_window: EntryWindow::default(),
            abandon_above_drawdown_percent: None,
            position_accounting: PositionAccounting::HedgedSingle,
            max_open_positions: 1,
            enable_tick_file_replay: false,
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
        let fill = &self.costs.fill_simulation;
        let fraction = fill.fill_volume_fraction;
        let requote_p = fill.requote_probability;
        let requote_slip = fill.requote_extra_slippage_points;
        if !(0.0..=1.0).contains(&fraction) || !fraction.is_finite() {
            return Err(EvalError::InvalidConfig(
                "fill_volume_fraction must be in [0, 1]".into(),
            ));
        }
        if !(0.0..=1.0).contains(&requote_p) || !requote_p.is_finite() {
            return Err(EvalError::InvalidConfig(
                "requote_probability must be in [0, 1]".into(),
            ));
        }
        if !requote_slip.is_finite() || requote_slip < 0.0 {
            return Err(EvalError::InvalidConfig(
                "requote_extra_slippage_points must be finite and non-negative".into(),
            ));
        }
        if self.max_open_positions == 0 {
            return Err(EvalError::InvalidConfig(
                "max_open_positions must be at least 1".into(),
            ));
        }
        self.entry_window.validate()?;
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

impl BacktestMetrics {
    /// MT5 Recovery Factor: net profit ÷ absolute equity max drawdown.
    ///
    /// Matches MetaTrader 5's "Recovery Factor" (Total Net Profit / Equity
    /// Drawdown Maximal in account currency). Returns +∞ when there is profit
    /// and no drawdown, and the raw net profit when both are non-positive.
    pub fn recovery_factor(&self) -> f64 {
        if self.max_drawdown > 1.0e-12 {
            self.net_profit / self.max_drawdown
        } else if self.net_profit > 0.0 {
            f64::INFINITY
        } else {
            self.net_profit
        }
    }
}

/// Broker-local hour when new entries/pending may first be placed (inclusive).
pub const MANDATORY_ENTRY_WINDOW_START_HOUR: u32 = 2;
/// Broker-local hour when the entry window ends (exclusive). 19 = 7pm: no new
/// entries or pending from 19:00 onward.
pub const MANDATORY_ENTRY_WINDOW_END_HOUR: u32 = 19;

/// Broker-local hours during which new entries and pending orders may be placed.
///
/// Both bounds are broker local time, resolved through the symbol profile's
/// timezone, so a window survives a broker whose server day starts at a
/// different UTC offset. `start_hour` is inclusive and `end_hour` exclusive, so
/// `[2, 19)` admits 02:00 through 18:59 and rejects 19:00 onward. Brokers that
/// only accept orders a few minutes after the hour should widen the start hour
/// rather than rely on the minute-level session gate in the broker profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryWindow {
    pub start_hour: u32,
    pub end_hour: u32,
}

impl Default for EntryWindow {
    fn default() -> Self {
        Self {
            start_hour: MANDATORY_ENTRY_WINDOW_START_HOUR,
            end_hour: MANDATORY_ENTRY_WINDOW_END_HOUR,
        }
    }
}

impl EntryWindow {
    pub fn new(start_hour: u32, end_hour: u32) -> Self {
        Self {
            start_hour,
            end_hour,
        }
    }

    pub fn contains(&self, hour: u32) -> bool {
        (self.start_hour..self.end_hour).contains(&hour)
    }

    pub fn validate(&self) -> Result<(), EvalError> {
        if self.start_hour > 23 || self.end_hour > 24 {
            return Err(EvalError::InvalidConfig(
                "entry window hours must be 0-23 for the start and 0-24 for the end".into(),
            ));
        }
        if self.start_hour >= self.end_hour {
            return Err(EvalError::InvalidConfig(
                "entry window start hour must be earlier than its end hour".into(),
            ));
        }
        Ok(())
    }
}

/// Default QuantForge entry session: `[02:00, 19:00)` broker local time.
pub fn in_mandatory_entry_window(hour: u32) -> bool {
    EntryWindow::default().contains(hour)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutTelemetry {
    pub conflicting_entry_signals: usize,
    pub skipped_outside_session: usize,
    #[serde(default)]
    pub skipped_outside_entry_window: usize,
    pub skipped_for_spread: usize,
    pub skipped_for_broker_stop_level: usize,
    pub skipped_below_minimum_volume: usize,
    pub pending_orders_placed: usize,
    pub pending_orders_filled: usize,
    pub pending_orders_expired: usize,
    pub pending_orders_cancelled_opposite: usize,
    pub pending_orders_replaced: usize,
    #[serde(default)]
    pub pending_orders_modified: usize,
    #[serde(default)]
    pub entry_requotes: usize,
    #[serde(default)]
    pub entry_requote_rejects: usize,
    #[serde(default)]
    pub partial_entry_fills: usize,
    #[serde(default)]
    pub netting_closes: usize,
    /// Opens that landed while at least one other position was already open.
    #[serde(default)]
    pub stacked_opens: usize,
    pub partial_exits_executed: usize,
    pub break_even_moves: usize,
    pub trailing_stop_moves: usize,
    pub end_of_day_flattens: usize,
    #[serde(default)]
    pub skipped_max_one_entry_per_day: usize,
    #[serde(default)]
    pub skipped_weekend_entries: usize,
    #[serde(default)]
    pub friday_exits: usize,
    pub synthetic_spread_bars: usize,
    pub fallback_spread_bars: usize,
    pub swap_rollover_events: usize,
    pub swap_effective_days: u32,
    /// Replay stopped early because drawdown passed
    /// [`ScoutConfig::abandon_above_drawdown_percent`]. The accompanying metrics
    /// cover only the bars replayed and are valid solely for rejection.
    #[serde(default)]
    pub abandoned_above_drawdown: bool,
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

#[cfg(test)]
mod tests {
    use super::BacktestMetrics;

    fn metrics(net_profit: f64, max_drawdown: f64) -> BacktestMetrics {
        BacktestMetrics {
            initial_balance: 100_000.0,
            ending_balance: 100_000.0 + net_profit,
            net_profit,
            return_percent: net_profit / 100_000.0 * 100.0,
            trade_count: 1,
            winning_trades: 1,
            losing_trades: 0,
            win_rate: 100.0,
            profit_factor: None,
            max_drawdown,
            max_drawdown_percent: max_drawdown / 100_000.0 * 100.0,
            sharpe_ratio: None,
            expectancy: net_profit,
        }
    }

    #[test]
    fn recovery_factor_matches_mt5_definition() {
        // MT5: Total Net Profit / Equity Drawdown Maximal
        let value = metrics(29_706.01, 8_593.20).recovery_factor();
        assert!((value - 3.46).abs() < 0.005);
    }
}
