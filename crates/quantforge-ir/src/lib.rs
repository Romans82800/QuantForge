//! Typed strategy intermediate representation shared by generation, execution,
//! fingerprinting and MQL5 export.

use quantforge_core::{
    ContentHash, FloatPolicy, HashError, STRATEGY_IR_VERSION, quantize, stable_json_hash,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceField {
    Open,
    High,
    Low,
    Close,
}

/// Every field is integer-valued, so `Eq`/`Hash` are exact and this type can key
/// an indicator buffer cache directly instead of via a serialized string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "operator", rename_all = "snake_case")]
pub enum IndicatorExpr {
    Sma {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    Ema {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    Wma {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    Rsi {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    Atr {
        period: u16,
        shift: u16,
    },
    /// Wilder's Average Directional Index (MT5 iADX buffer 0).
    Adx {
        period: u16,
        shift: u16,
    },
    /// Positive directional indicator (MT5 iADX buffer 1).
    PlusDi {
        period: u16,
        shift: u16,
    },
    /// Negative directional indicator (MT5 iADX buffer 2).
    MinusDi {
        period: u16,
        shift: u16,
    },
    DonchianHigh {
        period: u16,
        shift: u16,
    },
    DonchianLow {
        period: u16,
        shift: u16,
    },
    Highest {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    Lowest {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    StandardDeviation {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    ZScore {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    PercentileInRange {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    RateOfChange {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    /// Broker-local opening-range high formed over `range_bars` from `start_hour`.
    SessionRangeHigh {
        start_hour: u8,
        range_bars: u16,
        shift: u16,
    },
    /// Broker-local opening-range low formed over `range_bars` from `start_hour`.
    SessionRangeLow {
        start_hour: u8,
        range_bars: u16,
        shift: u16,
    },
    /// Candle body / range ratio `|C-O|/(H-L)` on the shifted bar.
    BodyRangeRatio {
        shift: u16,
    },
    /// Close location in the candle range `(C-L)/(H-L)` on the shifted bar.
    CloseLocationInBar {
        shift: u16,
    },
    /// Percentile rank of current ATR among the prior `lookback` ATR values.
    AtrPercentile {
        atr_period: u16,
        lookback: u16,
        shift: u16,
    },
    /// High edge of the most recent confirmed swing-low base zone.
    SwingBaseZoneHigh {
        swing_left: u16,
        swing_right: u16,
        base_bars: u16,
        shift: u16,
    },
    /// Low edge of the most recent confirmed swing-high base zone.
    SwingBaseZoneLow {
        swing_left: u16,
        swing_right: u16,
        base_bars: u16,
        shift: u16,
    },
    /// `+1` bullish liquidity sweep, `-1` bearish, else `0`.
    LiquiditySweepScore {
        period: u16,
        shift: u16,
    },
    /// MACD main line: `EMA(fast) - EMA(slow)`.
    MacdMain {
        source: PriceField,
        fast_period: u16,
        slow_period: u16,
        shift: u16,
    },
    /// MACD signal line: EMA of the main line.
    MacdSignal {
        source: PriceField,
        fast_period: u16,
        slow_period: u16,
        signal_period: u16,
        shift: u16,
    },
    /// MACD histogram: `main - signal`.
    MacdHistogram {
        source: PriceField,
        fast_period: u16,
        slow_period: u16,
        signal_period: u16,
        shift: u16,
    },
    /// Bollinger middle band (simple moving average).
    BollingerMid {
        source: PriceField,
        period: u16,
        shift: u16,
    },
    /// Bollinger upper band. Deviation is carried in tenths so the IR stays
    /// integer-valued and fingerprints cannot drift on float formatting.
    BollingerUpper {
        source: PriceField,
        period: u16,
        deviation_tenths: u16,
        shift: u16,
    },
    /// Bollinger lower band.
    BollingerLower {
        source: PriceField,
        period: u16,
        deviation_tenths: u16,
        shift: u16,
    },
    /// `(upper - lower) / mid * 100` — squeeze / expansion detector.
    BollingerBandwidth {
        source: PriceField,
        period: u16,
        deviation_tenths: u16,
        shift: u16,
    },
    /// Ichimoku conversion line: midpoint of the last `period` bars.
    IchimokuTenkan {
        period: u16,
        shift: u16,
    },
    /// Ichimoku base line: midpoint of the last `period` bars.
    IchimokuKijun {
        period: u16,
        shift: u16,
    },
    /// Ichimoku leading span A, displaced forward by `kijun_period` bars so the
    /// value read at a bar is the cloud edge visible on that bar.
    IchimokuSenkouA {
        tenkan_period: u16,
        kijun_period: u16,
        shift: u16,
    },
    /// Ichimoku leading span B, displaced forward by `kijun_period` bars.
    IchimokuSenkouB {
        period: u16,
        kijun_period: u16,
        shift: u16,
    },
    /// QQE smoothed RSI line: `EMA(RSI(rsi_period), smoothing_period)`.
    QqeLine {
        rsi_period: u16,
        smoothing_period: u16,
        shift: u16,
    },
    /// QQE trailing level derived from the smoothed RSI's average true range.
    /// The Wilder factor is carried in tenths (42 = 4.2).
    QqeTrail {
        rsi_period: u16,
        smoothing_period: u16,
        factor_tenths: u16,
        shift: u16,
    },
    /// Rolling volume-weighted average price over `period` bars.
    Vwap {
        period: u16,
        shift: u16,
    },
    /// Commodity Channel Index on typical price.
    Cci {
        period: u16,
        shift: u16,
    },
}

impl IndicatorExpr {
    /// Mutable access to the bar shift carried by every indicator variant.
    pub fn shift_mut(&mut self) -> &mut u16 {
        match self {
            Self::Sma { shift, .. }
            | Self::Ema { shift, .. }
            | Self::Wma { shift, .. }
            | Self::Rsi { shift, .. }
            | Self::Atr { shift, .. }
            | Self::Adx { shift, .. }
            | Self::PlusDi { shift, .. }
            | Self::MinusDi { shift, .. }
            | Self::DonchianHigh { shift, .. }
            | Self::DonchianLow { shift, .. }
            | Self::Highest { shift, .. }
            | Self::Lowest { shift, .. }
            | Self::StandardDeviation { shift, .. }
            | Self::ZScore { shift, .. }
            | Self::PercentileInRange { shift, .. }
            | Self::RateOfChange { shift, .. }
            | Self::SessionRangeHigh { shift, .. }
            | Self::SessionRangeLow { shift, .. }
            | Self::BodyRangeRatio { shift, .. }
            | Self::CloseLocationInBar { shift, .. }
            | Self::AtrPercentile { shift, .. }
            | Self::SwingBaseZoneHigh { shift, .. }
            | Self::SwingBaseZoneLow { shift, .. }
            | Self::LiquiditySweepScore { shift, .. }
            | Self::MacdMain { shift, .. }
            | Self::MacdSignal { shift, .. }
            | Self::MacdHistogram { shift, .. }
            | Self::BollingerMid { shift, .. }
            | Self::BollingerUpper { shift, .. }
            | Self::BollingerLower { shift, .. }
            | Self::BollingerBandwidth { shift, .. }
            | Self::IchimokuTenkan { shift, .. }
            | Self::IchimokuKijun { shift, .. }
            | Self::IchimokuSenkouA { shift, .. }
            | Self::IchimokuSenkouB { shift, .. }
            | Self::QqeLine { shift, .. }
            | Self::QqeTrail { shift, .. }
            | Self::Vwap { shift, .. }
            | Self::Cci { shift, .. } => shift,
        }
    }

    /// Cache key for the underlying indicator buffer. Series calculation ignores
    /// `shift` (it is a lookup offset, in both Scout and MQL5), so two expressions
    /// differing only in shift must share one buffer.
    pub fn buffer_key(&self) -> Self {
        let mut key = self.clone();
        *key.shift_mut() = 0;
        key
    }

    /// Representative lookback and bar shift, used for IR validation and by the
    /// evaluator to resolve reads against completed bars.
    pub fn period_and_shift(&self) -> (u16, u16) {
        match *self {
            Self::Sma { period, shift, .. }
            | Self::Ema { period, shift, .. }
            | Self::Wma { period, shift, .. }
            | Self::Rsi { period, shift, .. }
            | Self::Atr { period, shift }
            | Self::Adx { period, shift }
            | Self::PlusDi { period, shift }
            | Self::MinusDi { period, shift }
            | Self::DonchianHigh { period, shift }
            | Self::DonchianLow { period, shift }
            | Self::Highest { period, shift, .. }
            | Self::Lowest { period, shift, .. }
            | Self::StandardDeviation { period, shift, .. }
            | Self::ZScore { period, shift, .. }
            | Self::PercentileInRange { period, shift, .. }
            | Self::RateOfChange { period, shift, .. }
            | Self::LiquiditySweepScore { period, shift } => (period, shift),
            Self::SessionRangeHigh {
                range_bars, shift, ..
            }
            | Self::SessionRangeLow {
                range_bars, shift, ..
            } => (range_bars.max(2), shift),
            Self::BodyRangeRatio { shift } | Self::CloseLocationInBar { shift } => (2, shift),
            Self::AtrPercentile {
                atr_period,
                lookback,
                shift,
            } => (atr_period.max(lookback).max(2), shift),
            Self::SwingBaseZoneHigh {
                swing_left,
                swing_right,
                base_bars,
                shift,
            }
            | Self::SwingBaseZoneLow {
                swing_left,
                swing_right,
                base_bars,
                shift,
            } => (
                swing_left
                    .saturating_add(swing_right)
                    .saturating_add(base_bars)
                    .max(2),
                shift,
            ),
            Self::MacdMain {
                fast_period,
                slow_period,
                shift,
                ..
            } => (fast_period.max(slow_period).max(2), shift),
            Self::MacdSignal {
                fast_period,
                slow_period,
                signal_period,
                shift,
                ..
            }
            | Self::MacdHistogram {
                fast_period,
                slow_period,
                signal_period,
                shift,
                ..
            } => (
                fast_period
                    .max(slow_period)
                    .saturating_add(signal_period)
                    .max(2),
                shift,
            ),
            Self::BollingerMid { period, shift, .. }
            | Self::BollingerUpper { period, shift, .. }
            | Self::BollingerLower { period, shift, .. }
            | Self::BollingerBandwidth { period, shift, .. }
            | Self::IchimokuTenkan { period, shift }
            | Self::IchimokuKijun { period, shift }
            | Self::Vwap { period, shift }
            | Self::Cci { period, shift } => (period.max(2), shift),
            Self::IchimokuSenkouA {
                tenkan_period,
                kijun_period,
                shift,
            } => (
                tenkan_period.max(kijun_period).saturating_add(kijun_period),
                shift,
            ),
            Self::IchimokuSenkouB {
                period,
                kijun_period,
                shift,
            } => (period.saturating_add(kijun_period).max(2), shift),
            Self::QqeLine {
                rsi_period,
                smoothing_period,
                shift,
            }
            | Self::QqeTrail {
                rsi_period,
                smoothing_period,
                shift,
                ..
            } => (rsi_period.max(smoothing_period).max(2), shift),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextValue {
    SessionHour,
    DayOfWeek,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NumericExpr {
    Price { field: PriceField, shift: u16 },
    Indicator { value: IndicatorExpr },
    Context { value: ContextValue, shift: u16 },
    Constant { value: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonOp {
    GreaterThan,
    LessThan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "operator", rename_all = "snake_case")]
pub enum BoolExpr {
    Compare {
        comparison: ComparisonOp,
        left: NumericExpr,
        right: NumericExpr,
    },
    CrossAbove {
        left: NumericExpr,
        right: NumericExpr,
    },
    CrossBelow {
        left: NumericExpr,
        right: NumericExpr,
    },
    Between {
        value: NumericExpr,
        lower: NumericExpr,
        upper: NumericExpr,
    },
    And {
        children: Vec<BoolExpr>,
    },
    Or {
        children: Vec<BoolExpr>,
    },
    Not {
        child: Box<BoolExpr>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntrySignals {
    pub long: Option<BoolExpr>,
    pub short: Option<BoolExpr>,
    /// How a valid signal is converted into an order. Older IR files omit
    /// this field and retain the original market-entry behaviour.
    #[serde(default)]
    pub order: EntryOrderPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryDistancePolicy {
    FixedPoints { points: f64 },
    AtrMultiple { period: u16, multiplier: f64 },
    RangeMultiple { period: u16, multiplier: f64 },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryOrderPolicy {
    #[default]
    Market,
    Stop {
        distance: EntryDistancePolicy,
        expiry_bars: u16,
    },
    Limit {
        distance: EntryDistancePolicy,
        expiry_bars: u16,
    },
    /// MT5 stop-limit: stop trigger first, then limit fill.
    /// `stop_distance` places the trigger away from the signal reference;
    /// `limit_offset` places the limit inward from that trigger
    /// (buy: limit = stop − offset; sell: limit = stop + offset).
    StopLimit {
        stop_distance: EntryDistancePolicy,
        limit_offset: EntryDistancePolicy,
        expiry_bars: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    LongOnly,
    ShortOnly,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RiskPolicy {
    FixedCurrency { amount: f64 },
    PercentBalance { percent: f64 },
    /// SQX-style fixed lot size (ignores stop-distance risk budget for sizing).
    FixedLots { lots: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StopLossPolicy {
    FixedPoints { points: f64 },
    AtrMultiple { period: u16, multiplier: f64 },
    RangeMultiple { period: u16, multiplier: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TakeProfitPolicy {
    RiskMultiple { multiple: f64 },
    FixedPoints { points: f64 },
    AtrMultiple { period: u16, multiplier: f64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtectiveStops {
    pub stop_loss: StopLossPolicy,
    pub take_profit: TakeProfitPolicy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrailingPolicy {
    RiskMultiple {
        activate_at_r: f64,
        distance_r: f64,
    },
    AtrMultiple {
        activate_at_r: f64,
        period: u16,
        multiplier: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartialExit {
    pub at_r: f64,
    /// Fraction of the original position in the interval `(0, 1]`.
    pub fraction: f64,
}

fn default_end_of_day_hour() -> u8 {
    23
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagePolicy {
    pub break_even_at_r: Option<f64>,
    pub trailing: Option<TrailingPolicy>,
    pub time_stop_bars: Option<u16>,
    pub partial_exits: Vec<PartialExit>,
    pub flatten_end_of_day: bool,
    /// Broker-local hour when open positions flatten and pending orders cancel.
    #[serde(default = "default_end_of_day_hour")]
    pub end_of_day_hour: u8,
    /// When true, the first fill (market open or pending activation) locks the
    /// broker-local calendar day — no further market entries or pending orders
    /// may be placed that day, even if the first trade closed early. An unfilled
    /// / expired pending does not consume the day's slot.
    /// Production Discover stamps this from job config; it is not an evolvable gene.
    #[serde(default)]
    pub max_one_entry_per_day: bool,
    /// Cancel a working pending when the opposite side signals (SQX-style OCO-lite).
    #[serde(default = "default_true")]
    pub cancel_pending_on_opposite: bool,
    /// Cancel and re-place a working pending when the same side re-signals.
    #[serde(default)]
    pub replace_pending_on_reentry: bool,
    /// Recalculate price/SL/TP/expiry on a working pending when the same side
    /// re-signals (MT5 OrderModify semantics). Preferred over cancel+replace.
    #[serde(default = "default_true")]
    pub modify_pending_on_reentry: bool,
    /// Block new entries on Saturday/Sunday (SQX DontTradeOnWeekends).
    #[serde(default)]
    pub dont_trade_on_weekends: bool,
    /// Flatten open positions / cancel pendings on Friday at `end_of_day_hour`
    /// (SQX ExitOnFriday). Independent of `flatten_end_of_day`.
    #[serde(default)]
    pub exit_on_friday: bool,
    /// Cap fills per broker-local day. `None` means unlimited unless
    /// `max_one_entry_per_day` is set (which behaves as a cap of 1).
    #[serde(default)]
    pub max_trades_per_day: Option<u16>,
    /// Optional SL distance clamps in price points (SQX MinMaxSLPT).
    #[serde(default)]
    pub min_stop_points: Option<f64>,
    #[serde(default)]
    pub max_stop_points: Option<f64>,
    /// Optional TP distance clamps in price points.
    #[serde(default)]
    pub min_take_profit_points: Option<f64>,
    #[serde(default)]
    pub max_take_profit_points: Option<f64>,
}

fn default_true() -> bool {
    true
}

impl Default for ManagePolicy {
    fn default() -> Self {
        Self {
            break_even_at_r: None,
            trailing: None,
            time_stop_bars: None,
            partial_exits: Vec::new(),
            flatten_end_of_day: false,
            end_of_day_hour: default_end_of_day_hour(),
            max_one_entry_per_day: false,
            cancel_pending_on_opposite: true,
            replace_pending_on_reentry: false,
            modify_pending_on_reentry: true,
            dont_trade_on_weekends: false,
            exit_on_friday: false,
            max_trades_per_day: None,
            min_stop_points: None,
            max_stop_points: None,
            min_take_profit_points: None,
            max_take_profit_points: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyMeta {
    pub thesis_hint: String,
    pub complexity: u16,
    pub export_safe: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrategyIr {
    pub id: String,
    pub version: u16,
    pub entry: EntrySignals,
    /// Legacy exit applied to either position side. New strategies should use
    /// `exit_long` and `exit_short` so mirrored exits cannot close the wrong side.
    pub exit: Option<BoolExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_long: Option<BoolExpr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_short: Option<BoolExpr>,
    pub filters: Vec<BoolExpr>,
    pub side: Side,
    pub risk: RiskPolicy,
    pub stops: ProtectiveStops,
    pub manage: ManagePolicy,
    pub meta: StrategyMeta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IrLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
    pub max_boolean_children: usize,
    pub max_filters: usize,
    pub max_indicator_period: u16,
}

impl Default for IrLimits {
    fn default() -> Self {
        Self {
            max_depth: 8,
            max_nodes: 64,
            max_boolean_children: 8,
            max_filters: 6,
            max_indicator_period: 2_000,
        }
    }
}

impl StrategyIr {
    pub fn validate_export_safe(&self, limits: IrLimits) -> Result<(), IrError> {
        if self.version != STRATEGY_IR_VERSION {
            return Err(IrError::UnsupportedVersion(self.version));
        }
        if !self.meta.export_safe {
            return Err(IrError::NotExportSafe);
        }
        if self.filters.len() > limits.max_filters {
            return Err(IrError::Invalid {
                path: "filters".into(),
                reason: format!("contains more than {} filters", limits.max_filters),
            });
        }

        match self.side {
            Side::LongOnly if self.entry.long.is_none() || self.entry.short.is_some() => {
                return Err(IrError::Invalid {
                    path: "entry".into(),
                    reason: "long-only strategies require only a long entry expression".into(),
                });
            }
            Side::ShortOnly if self.entry.short.is_none() || self.entry.long.is_some() => {
                return Err(IrError::Invalid {
                    path: "entry".into(),
                    reason: "short-only strategies require only a short entry expression".into(),
                });
            }
            Side::Both if self.entry.long.is_none() || self.entry.short.is_none() => {
                return Err(IrError::Invalid {
                    path: "entry".into(),
                    reason: "two-sided strategies require distinct long and short expressions"
                        .into(),
                });
            }
            _ => {}
        }
        if let Some(entry) = &self.entry.long {
            validate_bool(entry, "entry.long", 1, limits)?;
        }
        if let Some(entry) = &self.entry.short {
            validate_bool(entry, "entry.short", 1, limits)?;
        }
        validate_entry_order(&self.entry.order, limits)?;
        if let Some(exit) = &self.exit {
            validate_bool(exit, "exit", 1, limits)?;
        }
        if let Some(exit) = &self.exit_long {
            validate_bool(exit, "exit_long", 1, limits)?;
        }
        if let Some(exit) = &self.exit_short {
            validate_bool(exit, "exit_short", 1, limits)?;
        }
        for (index, filter) in self.filters.iter().enumerate() {
            validate_bool(filter, &format!("filters[{index}]"), 1, limits)?;
        }
        validate_risk(&self.risk)?;
        validate_stops(&self.stops, limits)?;
        validate_manage(&self.manage, limits)?;

        let complexity = self.complexity();
        if complexity.node_count > limits.max_nodes {
            return Err(IrError::Invalid {
                path: "strategy".into(),
                reason: format!(
                    "contains {} nodes; maximum is {}",
                    complexity.node_count, limits.max_nodes
                ),
            });
        }
        Ok(())
    }

    pub fn complexity(&self) -> Complexity {
        let mut complexity = Complexity::default();
        if let Some(entry) = &self.entry.long {
            complexity += bool_complexity(entry);
        }
        if let Some(entry) = &self.entry.short {
            complexity += bool_complexity(entry);
        }
        if let Some(exit) = &self.exit {
            complexity += bool_complexity(exit);
        }
        if let Some(exit) = &self.exit_long {
            complexity += bool_complexity(exit);
        }
        if let Some(exit) = &self.exit_short {
            complexity += bool_complexity(exit);
        }
        for filter in &self.filters {
            complexity += bool_complexity(filter);
        }
        complexity += policy_complexity(self);
        complexity.filter_count = self.filters.len();
        complexity.score =
            complexity.node_count + complexity.parameter_count + complexity.filter_count;
        complexity
    }

    pub fn canonicalized(&self, policy: FloatPolicy) -> Result<Self, IrError> {
        let mut value = self.clone();
        if let Some(entry) = &mut value.entry.long {
            canonicalize_bool(entry, policy)?;
        }
        if let Some(entry) = &mut value.entry.short {
            canonicalize_bool(entry, policy)?;
        }
        canonicalize_entry_order(&mut value.entry.order, policy)?;
        if let Some(exit) = &mut value.exit {
            canonicalize_bool(exit, policy)?;
        }
        if let Some(exit) = &mut value.exit_long {
            canonicalize_bool(exit, policy)?;
        }
        if let Some(exit) = &mut value.exit_short {
            canonicalize_bool(exit, policy)?;
        }
        for filter in &mut value.filters {
            canonicalize_bool(filter, policy)?;
        }
        value.filters.sort_by_key(serialized_sort_key);

        canonicalize_risk(&mut value.risk, policy)?;
        canonicalize_stops(&mut value.stops, policy)?;
        canonicalize_manage(&mut value.manage, policy)?;
        value.meta.complexity = value.complexity().score.min(u16::MAX as usize) as u16;
        Ok(value)
    }

    pub fn structural_fingerprint(&self, policy: FloatPolicy) -> Result<ContentHash, IrError> {
        let canonical = self.canonicalized(policy)?;
        let material = FingerprintMaterial {
            version: canonical.version,
            entry: &canonical.entry,
            exit: &canonical.exit,
            filters: &canonical.filters,
            side: canonical.side,
            risk: &canonical.risk,
            stops: &canonical.stops,
            manage: &canonical.manage,
        };
        if canonical.exit_long.is_none() && canonical.exit_short.is_none() {
            // Preserve all pre-side-specific fingerprints byte-for-byte.
            Ok(stable_json_hash(&material)?)
        } else {
            Ok(stable_json_hash(&SideSpecificFingerprintMaterial {
                legacy: material,
                exit_long: &canonical.exit_long,
                exit_short: &canonical.exit_short,
            })?)
        }
    }

    /// Resolve a long-position exit while retaining legacy artifacts.
    pub fn long_exit(&self) -> Option<&BoolExpr> {
        self.exit_long.as_ref().or(self.exit.as_ref())
    }

    /// Resolve a short-position exit while retaining legacy artifacts.
    pub fn short_exit(&self) -> Option<&BoolExpr> {
        self.exit_short.as_ref().or(self.exit.as_ref())
    }
}

fn policy_complexity(strategy: &StrategyIr) -> Complexity {
    // Risk, protective exits and management rules are executable structure too;
    // excluding them would reward elaborate management disguised as "simple".
    let mut node_count = 4; // risk, protective-stops, stop-loss and take-profit
    let mut parameter_count = 1; // risk amount or percent

    match &strategy.entry.order {
        EntryOrderPolicy::Market => {}
        EntryOrderPolicy::Stop { distance, .. } | EntryOrderPolicy::Limit { distance, .. } => {
            node_count += 1;
            parameter_count += 1; // expiry bars
            parameter_count += match distance {
                EntryDistancePolicy::FixedPoints { .. } => 1,
                EntryDistancePolicy::AtrMultiple { .. }
                | EntryDistancePolicy::RangeMultiple { .. } => 2,
            };
        }
        EntryOrderPolicy::StopLimit {
            stop_distance,
            limit_offset,
            ..
        } => {
            node_count += 1;
            parameter_count += 1; // expiry bars
            for distance in [stop_distance, limit_offset] {
                parameter_count += match distance {
                    EntryDistancePolicy::FixedPoints { .. } => 1,
                    EntryDistancePolicy::AtrMultiple { .. }
                    | EntryDistancePolicy::RangeMultiple { .. } => 2,
                };
            }
        }
    }

    parameter_count += match strategy.stops.stop_loss {
        StopLossPolicy::FixedPoints { .. } => 1,
        StopLossPolicy::AtrMultiple { .. } | StopLossPolicy::RangeMultiple { .. } => 2,
    };
    parameter_count += match strategy.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { .. } | TakeProfitPolicy::FixedPoints { .. } => 1,
        TakeProfitPolicy::AtrMultiple { .. } => 2,
    };

    if strategy.manage.break_even_at_r.is_some() {
        node_count += 1;
        parameter_count += 1;
    }
    if let Some(trailing) = &strategy.manage.trailing {
        node_count += 1;
        parameter_count += match trailing {
            TrailingPolicy::RiskMultiple { .. } => 2,
            TrailingPolicy::AtrMultiple { .. } => 3,
        };
    }
    if strategy.manage.time_stop_bars.is_some() {
        node_count += 1;
        parameter_count += 1;
    }
    node_count += strategy.manage.partial_exits.len();
    parameter_count += strategy.manage.partial_exits.len() * 2;

    Complexity {
        node_count,
        parameter_count,
        filter_count: 0,
        score: node_count + parameter_count,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Complexity {
    pub node_count: usize,
    pub parameter_count: usize,
    pub filter_count: usize,
    pub score: usize,
}

impl std::ops::AddAssign for Complexity {
    fn add_assign(&mut self, other: Self) {
        self.node_count += other.node_count;
        self.parameter_count += other.parameter_count;
        self.filter_count += other.filter_count;
        self.score += other.score;
    }
}

#[derive(Serialize)]
struct FingerprintMaterial<'a> {
    version: u16,
    entry: &'a EntrySignals,
    exit: &'a Option<BoolExpr>,
    filters: &'a [BoolExpr],
    side: Side,
    risk: &'a RiskPolicy,
    stops: &'a ProtectiveStops,
    manage: &'a ManagePolicy,
}

#[derive(Serialize)]
struct SideSpecificFingerprintMaterial<'a> {
    legacy: FingerprintMaterial<'a>,
    exit_long: &'a Option<BoolExpr>,
    exit_short: &'a Option<BoolExpr>,
}

fn validate_bool(
    expression: &BoolExpr,
    path: &str,
    depth: usize,
    limits: IrLimits,
) -> Result<(), IrError> {
    if depth > limits.max_depth {
        return Err(IrError::Invalid {
            path: path.into(),
            reason: format!("exceeds maximum expression depth {}", limits.max_depth),
        });
    }

    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            validate_numeric(left, &format!("{path}.left"), limits)?;
            validate_numeric(right, &format!("{path}.right"), limits)?;
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            validate_numeric(value, &format!("{path}.value"), limits)?;
            validate_numeric(lower, &format!("{path}.lower"), limits)?;
            validate_numeric(upper, &format!("{path}.upper"), limits)?;
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            if !(2..=limits.max_boolean_children).contains(&children.len()) {
                return Err(IrError::Invalid {
                    path: path.into(),
                    reason: format!(
                        "must contain between 2 and {} children",
                        limits.max_boolean_children
                    ),
                });
            }
            for (index, child) in children.iter().enumerate() {
                validate_bool(
                    child,
                    &format!("{path}.children[{index}]"),
                    depth + 1,
                    limits,
                )?;
            }
        }
        BoolExpr::Not { child } => {
            validate_bool(child, &format!("{path}.child"), depth + 1, limits)?
        }
    }
    Ok(())
}

fn validate_numeric(expression: &NumericExpr, path: &str, limits: IrLimits) -> Result<(), IrError> {
    match expression {
        NumericExpr::Price { shift, .. } | NumericExpr::Context { shift, .. } => {
            require_completed_bar(*shift, path)?;
        }
        NumericExpr::Indicator { value } => {
            let (period, shift) = value.period_and_shift();
            if !(2..=limits.max_indicator_period).contains(&period) {
                return Err(IrError::Invalid {
                    path: path.into(),
                    reason: format!(
                        "indicator period must be between 2 and {}",
                        limits.max_indicator_period
                    ),
                });
            }
            require_completed_bar(shift, path)?;
        }
        NumericExpr::Constant { value } if !value.is_finite() => {
            return Err(IrError::Invalid {
                path: path.into(),
                reason: "constant must be finite".into(),
            });
        }
        NumericExpr::Constant { .. } => {}
    }
    Ok(())
}

fn require_completed_bar(shift: u16, path: &str) -> Result<(), IrError> {
    if shift == 0 {
        Err(IrError::Invalid {
            path: path.into(),
            reason: "forming-bar access is forbidden; shift must be at least 1".into(),
        })
    } else {
        Ok(())
    }
}

fn validate_risk(risk: &RiskPolicy) -> Result<(), IrError> {
    match risk {
        RiskPolicy::FixedCurrency { amount } => require_positive("risk.amount", *amount),
        RiskPolicy::PercentBalance { percent } => {
            require_positive("risk.percent", *percent)?;
            if *percent > 100.0 {
                return Err(IrError::Invalid {
                    path: "risk.percent".into(),
                    reason: "must not exceed 100".into(),
                });
            }
            Ok(())
        }
        RiskPolicy::FixedLots { lots } => require_positive("risk.lots", *lots),
    }
}

fn validate_entry_order(order: &EntryOrderPolicy, limits: IrLimits) -> Result<(), IrError> {
    let expiry_bars = match order {
        EntryOrderPolicy::Market => return Ok(()),
        EntryOrderPolicy::Stop { expiry_bars, .. }
        | EntryOrderPolicy::Limit { expiry_bars, .. }
        | EntryOrderPolicy::StopLimit { expiry_bars, .. } => *expiry_bars,
    };
    if expiry_bars == 0 {
        return Err(IrError::Invalid {
            path: "entry.order.expiry_bars".into(),
            reason: "must be greater than zero".into(),
        });
    }
    match order {
        EntryOrderPolicy::Market => Ok(()),
        EntryOrderPolicy::Stop { distance, .. } | EntryOrderPolicy::Limit { distance, .. } => {
            validate_entry_distance("entry.order.distance", distance, limits)
        }
        EntryOrderPolicy::StopLimit {
            stop_distance,
            limit_offset,
            ..
        } => {
            validate_entry_distance("entry.order.stop_distance", stop_distance, limits)?;
            validate_entry_distance("entry.order.limit_offset", limit_offset, limits)
        }
    }
}

fn validate_entry_distance(
    path: &str,
    distance: &EntryDistancePolicy,
    limits: IrLimits,
) -> Result<(), IrError> {
    match distance {
        EntryDistancePolicy::FixedPoints { points } => {
            require_positive(&format!("{path}.points"), *points)
        }
        EntryDistancePolicy::AtrMultiple { period, multiplier }
        | EntryDistancePolicy::RangeMultiple { period, multiplier } => {
            validate_period(&format!("{path}.period"), *period, limits)?;
            require_positive(&format!("{path}.multiplier"), *multiplier)
        }
    }
}

fn validate_stops(stops: &ProtectiveStops, limits: IrLimits) -> Result<(), IrError> {
    match stops.stop_loss {
        StopLossPolicy::FixedPoints { points } => {
            require_positive("stops.stop_loss.points", points)?
        }
        StopLossPolicy::AtrMultiple { period, multiplier }
        | StopLossPolicy::RangeMultiple { period, multiplier } => {
            validate_period("stops.stop_loss.period", period, limits)?;
            require_positive("stops.stop_loss.multiplier", multiplier)?;
        }
    }
    match stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => {
            require_positive("stops.take_profit.multiple", multiple)?
        }
        TakeProfitPolicy::FixedPoints { points } => {
            require_positive("stops.take_profit.points", points)?
        }
        TakeProfitPolicy::AtrMultiple { period, multiplier } => {
            validate_period("stops.take_profit.period", period, limits)?;
            require_positive("stops.take_profit.multiplier", multiplier)?;
        }
    }
    Ok(())
}

fn validate_manage(manage: &ManagePolicy, limits: IrLimits) -> Result<(), IrError> {
    if manage.end_of_day_hour > 23 {
        return Err(IrError::Invalid {
            path: "manage.end_of_day_hour".into(),
            reason: "must be between 0 and 23".into(),
        });
    }
    if let Some(value) = manage.break_even_at_r {
        require_positive("manage.break_even_at_r", value)?;
    }
    if manage.time_stop_bars == Some(0) {
        return Err(IrError::Invalid {
            path: "manage.time_stop_bars".into(),
            reason: "must be greater than zero".into(),
        });
    }
    if let Some(trailing) = &manage.trailing {
        match *trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => {
                require_positive("manage.trailing.activate_at_r", activate_at_r)?;
                require_positive("manage.trailing.distance_r", distance_r)?;
            }
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                period,
                multiplier,
            } => {
                require_positive("manage.trailing.activate_at_r", activate_at_r)?;
                validate_period("manage.trailing.period", period, limits)?;
                require_positive("manage.trailing.multiplier", multiplier)?;
            }
        }
    }

    let mut total_fraction = 0.0;
    for (index, partial) in manage.partial_exits.iter().enumerate() {
        require_positive("manage.partial_exits.at_r", partial.at_r)?;
        if !partial.fraction.is_finite() || partial.fraction <= 0.0 || partial.fraction > 1.0 {
            return Err(IrError::Invalid {
                path: format!("manage.partial_exits[{index}].fraction"),
                reason: "must be in the interval (0, 1]".into(),
            });
        }
        total_fraction += partial.fraction;
    }
    if total_fraction > 1.0 + f64::EPSILON {
        return Err(IrError::Invalid {
            path: "manage.partial_exits".into(),
            reason: "fractions must not total more than 1".into(),
        });
    }
    Ok(())
}

fn validate_period(path: &str, period: u16, limits: IrLimits) -> Result<(), IrError> {
    if !(2..=limits.max_indicator_period).contains(&period) {
        Err(IrError::Invalid {
            path: path.into(),
            reason: format!("must be between 2 and {}", limits.max_indicator_period),
        })
    } else {
        Ok(())
    }
}

fn require_positive(path: &str, value: f64) -> Result<(), IrError> {
    if !value.is_finite() || value <= 0.0 {
        Err(IrError::Invalid {
            path: path.into(),
            reason: "must be finite and greater than zero".into(),
        })
    } else {
        Ok(())
    }
}

fn bool_complexity(expression: &BoolExpr) -> Complexity {
    let mut result = Complexity {
        node_count: 1,
        ..Complexity::default()
    };
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            result += numeric_complexity(left);
            result += numeric_complexity(right);
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            result += numeric_complexity(value);
            result += numeric_complexity(lower);
            result += numeric_complexity(upper);
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children {
                result += bool_complexity(child);
            }
        }
        BoolExpr::Not { child } => result += bool_complexity(child),
    }
    result
}

fn numeric_complexity(expression: &NumericExpr) -> Complexity {
    let parameter_count = match expression {
        NumericExpr::Price { .. } | NumericExpr::Context { .. } => 1,
        NumericExpr::Indicator { .. } => 2,
        NumericExpr::Constant { .. } => 1,
    };
    Complexity {
        node_count: 1,
        parameter_count,
        filter_count: 0,
        score: 1 + parameter_count,
    }
}

fn canonicalize_bool(expression: &mut BoolExpr, policy: FloatPolicy) -> Result<(), IrError> {
    match expression {
        BoolExpr::Compare { left, right, .. }
        | BoolExpr::CrossAbove { left, right }
        | BoolExpr::CrossBelow { left, right } => {
            canonicalize_numeric(left, policy)?;
            canonicalize_numeric(right, policy)?;
        }
        BoolExpr::Between {
            value,
            lower,
            upper,
        } => {
            canonicalize_numeric(value, policy)?;
            canonicalize_numeric(lower, policy)?;
            canonicalize_numeric(upper, policy)?;
        }
        BoolExpr::And { children } | BoolExpr::Or { children } => {
            for child in children.iter_mut() {
                canonicalize_bool(child, policy)?;
            }
            children.sort_by_key(serialized_sort_key);
        }
        BoolExpr::Not { child } => canonicalize_bool(child, policy)?,
    }
    Ok(())
}

fn canonicalize_numeric(expression: &mut NumericExpr, policy: FloatPolicy) -> Result<(), IrError> {
    if let NumericExpr::Constant { value } = expression {
        *value = quantize(*value, policy.parameter_quantum)
            .map_err(|error| IrError::Canonicalization(error.to_string()))?;
    }
    Ok(())
}

fn canonicalize_risk(risk: &mut RiskPolicy, policy: FloatPolicy) -> Result<(), IrError> {
    match risk {
        RiskPolicy::FixedCurrency { amount } => *amount = q(*amount, policy)?,
        RiskPolicy::PercentBalance { percent } => *percent = q(*percent, policy)?,
        RiskPolicy::FixedLots { lots } => *lots = q(*lots, policy)?,
    }
    Ok(())
}

fn canonicalize_entry_order(
    order: &mut EntryOrderPolicy,
    policy: FloatPolicy,
) -> Result<(), IrError> {
    match order {
        EntryOrderPolicy::Market => Ok(()),
        EntryOrderPolicy::Stop { distance, .. } | EntryOrderPolicy::Limit { distance, .. } => {
            canonicalize_entry_distance(distance, policy)
        }
        EntryOrderPolicy::StopLimit {
            stop_distance,
            limit_offset,
            ..
        } => {
            canonicalize_entry_distance(stop_distance, policy)?;
            canonicalize_entry_distance(limit_offset, policy)
        }
    }
}

fn canonicalize_entry_distance(
    distance: &mut EntryDistancePolicy,
    policy: FloatPolicy,
) -> Result<(), IrError> {
    match distance {
        EntryDistancePolicy::FixedPoints { points } => *points = q(*points, policy)?,
        EntryDistancePolicy::AtrMultiple { multiplier, .. }
        | EntryDistancePolicy::RangeMultiple { multiplier, .. } => {
            *multiplier = q(*multiplier, policy)?
        }
    }
    Ok(())
}

fn canonicalize_stops(stops: &mut ProtectiveStops, policy: FloatPolicy) -> Result<(), IrError> {
    match &mut stops.stop_loss {
        StopLossPolicy::FixedPoints { points } => *points = q(*points, policy)?,
        StopLossPolicy::AtrMultiple { multiplier, .. }
        | StopLossPolicy::RangeMultiple { multiplier, .. } => *multiplier = q(*multiplier, policy)?,
    }
    match &mut stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => *multiple = q(*multiple, policy)?,
        TakeProfitPolicy::FixedPoints { points } => *points = q(*points, policy)?,
        TakeProfitPolicy::AtrMultiple { multiplier, .. } => *multiplier = q(*multiplier, policy)?,
    }
    Ok(())
}

fn canonicalize_manage(manage: &mut ManagePolicy, policy: FloatPolicy) -> Result<(), IrError> {
    if let Some(value) = &mut manage.break_even_at_r {
        *value = q(*value, policy)?;
    }
    if let Some(trailing) = &mut manage.trailing {
        match trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => {
                *activate_at_r = q(*activate_at_r, policy)?;
                *distance_r = q(*distance_r, policy)?;
            }
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                multiplier,
                ..
            } => {
                *activate_at_r = q(*activate_at_r, policy)?;
                *multiplier = q(*multiplier, policy)?;
            }
        }
    }
    for partial in &mut manage.partial_exits {
        partial.at_r = q(partial.at_r, policy)?;
        partial.fraction = q(partial.fraction, policy)?;
    }
    manage.partial_exits.sort_by(|left, right| {
        left.at_r
            .total_cmp(&right.at_r)
            .then_with(|| left.fraction.total_cmp(&right.fraction))
    });
    Ok(())
}

fn q(value: f64, policy: FloatPolicy) -> Result<f64, IrError> {
    quantize(value, policy.parameter_quantum)
        .map_err(|error| IrError::Canonicalization(error.to_string()))
}

fn serialized_sort_key<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).expect("IR variants contain only serializable fields")
}

#[derive(Debug, Error)]
pub enum IrError {
    #[error("unsupported strategy IR version {0}")]
    UnsupportedVersion(u16),
    #[error("strategy is marked as not export-safe")]
    NotExportSafe,
    #[error("invalid IR at {path}: {reason}")]
    Invalid { path: String, reason: String },
    #[error("could not canonicalize IR: {0}")]
    Canonicalization(String),
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close() -> NumericExpr {
        NumericExpr::Price {
            field: PriceField::Close,
            shift: 1,
        }
    }

    fn ema(period: u16) -> NumericExpr {
        NumericExpr::Indicator {
            value: IndicatorExpr::Ema {
                source: PriceField::Close,
                period,
                shift: 1,
            },
        }
    }

    fn fixture() -> StrategyIr {
        StrategyIr {
            id: "candidate-a".into(),
            version: STRATEGY_IR_VERSION,
            entry: EntrySignals {
                long: Some(BoolExpr::And {
                    children: vec![
                        BoolExpr::CrossAbove {
                            left: ema(12),
                            right: ema(48),
                        },
                        BoolExpr::Compare {
                            comparison: ComparisonOp::GreaterThan,
                            left: close(),
                            right: NumericExpr::Constant { value: 1.0 },
                        },
                    ],
                }),
                short: Some(BoolExpr::CrossBelow {
                    left: ema(12),
                    right: ema(48),
                }),
                order: Default::default(),
            },
            exit: None,
            exit_long: None,
            exit_short: None,
            filters: vec![],
            side: Side::Both,
            risk: RiskPolicy::FixedCurrency { amount: 1_000.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::AtrMultiple {
                    period: 14,
                    multiplier: 2.0,
                },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
            },
            manage: ManagePolicy::default(),
            meta: StrategyMeta {
                thesis_hint: "EMA trend".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    #[test]
    fn commutative_children_and_identity_metadata_do_not_change_fingerprint() {
        let first = fixture();
        let mut second = fixture();
        second.id = "candidate-b".into();
        second.meta.thesis_hint = "renamed".into();
        if let Some(BoolExpr::And { children }) = &mut second.entry.long {
            children.reverse();
        }

        assert_eq!(
            first
                .structural_fingerprint(FloatPolicy::default())
                .unwrap(),
            second
                .structural_fingerprint(FloatPolicy::default())
                .unwrap()
        );
    }

    #[test]
    fn forming_bar_access_is_rejected() {
        let mut strategy = fixture();
        strategy.entry.long = Some(BoolExpr::Compare {
            comparison: ComparisonOp::GreaterThan,
            left: NumericExpr::Price {
                field: PriceField::Close,
                shift: 0,
            },
            right: NumericExpr::Constant { value: 1.0 },
        });

        assert!(strategy.validate_export_safe(IrLimits::default()).is_err());
    }

    #[test]
    fn complexity_is_derived_from_structure() {
        let complexity = fixture().complexity();
        assert!(complexity.node_count >= 7);
        assert_eq!(complexity.filter_count, 0);
        assert_eq!(
            complexity.score,
            complexity.node_count + complexity.parameter_count
        );
    }
}
