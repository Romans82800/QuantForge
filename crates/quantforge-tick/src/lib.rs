//! Deterministic M1 acceptance engine used as QuantForge's internal judge.
//!
//! Strategy expressions are evaluated on the decision-timeframe bars. Orders,
//! protective gaps and stop/target chronology are replayed on M1 bars.

use chrono::Timelike;
use quantforge_broker::{BrokerClock, BrokerSpecError, SwapMode, SymbolSpecification, TradeMode};
use quantforge_core::FloatPolicy;
use quantforge_data::{Bar, BarDataset, QuoteBar, QuoteBarDataset, forward_fill_zero_spreads};
use quantforge_eval::{
    BacktestMetrics, CostModel, EntryWindow, EquityPoint, EvalError, ExitReason, FeatureCache,
    PositionSide, ScoutConfig, SpreadSource, Trade, accrue_swap, equity_sharpe_ratio,
    favorable_r as compute_favorable_r, normalize_price, placeable_stop_candidate,
    price_reaches_from_above, price_reaches_from_below, r_multiple, ratchet_favorable_peak,
    resolve_spread, stop_dollar_risk, trade_r_stats,
};
use quantforge_ir::{
    EntryDistancePolicy, EntryOrderPolicy, IndicatorExpr, IrError, IrLimits, RiskPolicy,
    StopLossPolicy, StrategyIr, TakeProfitPolicy, TrailingPolicy,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const ENGINE_TIER: &str = "m1-judge";

// Must match the execution gate in quantforge-eval and the generated MQL5
// template. MT5 has pre-test indicator history while an imported pack does not.
const PARITY_SIGNAL_WARMUP_BARS: usize = 320;
/// Matches the default leverage embedded by the MT5 exporter/tester pack.
/// The broker profile stores raw initial margin per lot; divide by this
/// leverage before deciding whether a fixed-risk order can be afforded.
const PARITY_TESTER_LEVERAGE: f64 = 100.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeConfig {
    pub initial_balance: f64,
    pub costs: CostModel,
    /// When true, continue M1 replay across in-bar minute gaps (research default).
    /// Gap events remain in telemetry for audit.
    pub allow_execution_gaps: bool,
    #[serde(default)]
    pub indicator_engine: quantforge_eval::IndicatorEngine,
    /// Broker-local hours in which new entries and pending orders may be placed.
    /// Must match the scout window, or M1 judgment will disagree by construction.
    #[serde(default)]
    pub entry_window: EntryWindow,
}

impl Default for JudgeConfig {
    fn default() -> Self {
        Self {
            initial_balance: 100_000.0,
            costs: CostModel::default(),
            allow_execution_gaps: true,
            indicator_engine: quantforge_eval::IndicatorEngine::Mt5,
            entry_window: EntryWindow::default(),
        }
    }
}

impl JudgeConfig {
    pub fn validate(&self) -> Result<(), JudgeError> {
        ScoutConfig {
            initial_balance: self.initial_balance,
            same_bar_policy: quantforge_eval::SameBarPolicy::Conservative,
            costs: self.costs.clone(),
            indicator_engine: self.indicator_engine,
            entry_window: self.entry_window,
            // The judge always replays in full; its metrics are the promotion evidence.
            abandon_above_drawdown_percent: None,
        }
        .validate()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgeTelemetry {
    pub decision_bars_replayed: usize,
    pub m1_bars_replayed: usize,
    pub m1_gap_events: usize,
    pub verified_no_tick_gap_events: usize,
    pub verified_no_tick_minutes: usize,
    pub same_minute_stop_target_collisions: usize,
    pub conflicting_entry_signals: usize,
    pub skipped_outside_session: usize,
    #[serde(default)]
    pub skipped_outside_entry_window: usize,
    pub skipped_for_spread: usize,
    pub skipped_for_broker_stop_level: usize,
    pub skipped_below_minimum_volume: usize,
    #[serde(default)]
    pub skipped_insufficient_margin: usize,
    pub pending_orders_placed: usize,
    pub pending_orders_filled: usize,
    pub pending_orders_expired: usize,
    pub partial_exits_executed: usize,
    pub break_even_moves: usize,
    pub trailing_stop_moves: usize,
    pub end_of_day_flattens: usize,
    #[serde(default)]
    pub skipped_max_one_entry_per_day: usize,
    pub synthetic_spread_bars: usize,
    pub fallback_spread_bars: usize,
    pub swap_rollover_events: usize,
    pub swap_effective_days: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResult {
    pub engine: String,
    pub decision_interval_ms: i64,
    pub execution_interval_ms: i64,
    pub trades: Vec<Trade>,
    pub equity: Vec<EquityPoint>,
    pub metrics: BacktestMetrics,
    pub telemetry: JudgeTelemetry,
}

#[derive(Debug)]
struct OpenPosition {
    side: PositionSide,
    entry_decision_index: usize,
    entry_timestamp_ms: i64,
    entry_price: f64,
    initial_volume: f64,
    volume: f64,
    stop_loss: f64,
    take_profit: f64,
    initial_stop_loss: f64,
    initial_take_profit: f64,
    initial_risk_distance: f64,
    initial_dollar_risk: f64,
    /// Best post-entry favorable price on completed M1 (fill-aware).
    peak_favorable_price: Option<f64>,
    entry_commission: f64,
    swap: f64,
    realized_gross_profit: f64,
    realized_exit_commission: f64,
    exited_volume: f64,
    weighted_exit_price: f64,
    partial_exit_done: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingKind {
    Stop,
    Limit,
}

#[derive(Debug)]
struct PendingOrder {
    side: PositionSide,
    kind: PendingKind,
    expiry_decision_index: usize,
    activation_price: f64,
    stop_loss: f64,
    take_profit: f64,
    stop_distance: f64,
    volume: f64,
}

#[derive(Debug, Clone, Copy)]
struct ExitEvent {
    base_price: f64,
    reason: ExitReason,
}

pub fn evaluate_strategy_m1(
    strategy: &StrategyIr,
    decision_dataset: &BarDataset,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
) -> Result<JudgeResult, JudgeError> {
    evaluate_strategy_m1_internal(strategy, decision_dataset, m1_dataset, None, broker, config)
}

pub fn evaluate_strategy_m1_with_quotes(
    strategy: &StrategyIr,
    decision_dataset: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: &QuoteBarDataset,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
) -> Result<JudgeResult, JudgeError> {
    evaluate_strategy_m1_internal(
        strategy,
        decision_dataset,
        m1_dataset,
        Some(quote_dataset),
        broker,
        config,
    )
}

fn evaluate_strategy_m1_internal(
    strategy: &StrategyIr,
    decision_dataset: &BarDataset,
    m1_dataset: &BarDataset,
    quote_dataset: Option<&QuoteBarDataset>,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
) -> Result<JudgeResult, JudgeError> {
    if decision_dataset.bars.len() < 2 {
        return Err(JudgeError::InsufficientDecisionBars);
    }
    if m1_dataset.bars.len() < 2 {
        return Err(JudgeError::InsufficientM1Bars);
    }
    broker.validate()?;
    config.validate()?;
    strategy.validate_export_safe(IrLimits::default())?;
    validate_broker_compatibility(strategy, broker)?;
    if decision_dataset.source_timezone != broker.timezone
        || m1_dataset.source_timezone != broker.timezone
    {
        return Err(JudgeError::TimezoneMismatch {
            broker: broker.timezone.clone(),
            decision: decision_dataset.source_timezone.clone(),
            execution: m1_dataset.source_timezone.clone(),
        });
    }

    let decision_interval_ms =
        median_interval(&decision_dataset.bars).ok_or(JudgeError::InvalidDecisionTimeframe)?;
    let execution_interval_ms = median_interval(&m1_dataset.bars)
        .ok_or(JudgeError::InvalidM1Timeframe { observed_ms: 0 })?;
    if execution_interval_ms != 60_000 {
        return Err(JudgeError::InvalidM1Timeframe {
            observed_ms: execution_interval_ms,
        });
    }
    if decision_interval_ms < execution_interval_ms
        || decision_interval_ms % execution_interval_ms != 0
    {
        return Err(JudgeError::InvalidDecisionTimeframe);
    }
    if let Some(quotes) = quote_dataset {
        quotes
            .validate_against(m1_dataset)
            .map_err(|error| JudgeError::QuoteDatasetMismatch(error.to_string()))?;
    }

    let strategy = strategy.canonicalized(FloatPolicy::default())?;
    // Quiet-minute SPREAD=0 on M1 understates ask stops. Carry the last positive
    // M1 spread only — do not forward-fill decision bars (H1 can stamp 100+ pt
    // spikes that would poison every subsequent hour if carried).
    let decision_bars = &decision_dataset.bars;
    let signal_warmup_bars = if decision_bars.len() > PARITY_SIGNAL_WARMUP_BARS {
        PARITY_SIGNAL_WARMUP_BARS
    } else {
        0
    };
    // The robustness battery calls this once per fold and once per parameter
    // sample, so an unconditional clone of the execution series dominates
    // promotion cost. Borrow whenever the data already carries spreads.
    let m1_bars_owned: std::borrow::Cow<'_, [Bar]> =
        if quantforge_data::needs_spread_forward_fill(&m1_dataset.bars) {
            let mut owned = m1_dataset.bars.clone();
            forward_fill_zero_spreads(&mut owned);
            std::borrow::Cow::Owned(owned)
        } else {
            std::borrow::Cow::Borrowed(&m1_dataset.bars)
        };
    let m1_bars: &[Bar] = &m1_bars_owned;
    let quote_bars_owned = if let Some(quotes) = quote_dataset {
        std::borrow::Cow::Borrowed(quotes.bars.as_slice())
    } else {
        std::borrow::Cow::Owned(derive_quote_bars(m1_bars, broker, &config.costs)?)
    };
    let quote_bars: &[QuoteBar] = &quote_bars_owned;
    let mut features =
        FeatureCache::with_engine(decision_bars, &broker.timezone, config.indicator_engine)?;
    let mut balance = config.initial_balance;
    let mut position: Option<OpenPosition> = None;
    let mut pending: Option<PendingOrder> = None;
    let mut trades = Vec::new();
    let mut equity = Vec::new();
    let mut telemetry = JudgeTelemetry::default();
    let mut m1_cursor = 0usize;
    let mut last_execution_timestamp_ms: Option<i64> = None;
    let mut last_execution_bars: &[Bar] = &[];
    let mut last_execution_quotes: &[QuoteBar] = &[];
    let broker_clock = BrokerClock::parse(&broker.timezone)?;
    let mut active_entry_day: Option<chrono::NaiveDate> = None;
    // First fill (market open or pending activation) locks the broker day.
    let mut signal_taken_today = false;

    for (decision_index, decision_bar) in decision_bars.iter().enumerate().skip(1) {
        let start = decision_bar.timestamp_ms;
        let end = start
            .checked_add(decision_interval_ms)
            .ok_or(JudgeError::InvalidDecisionTimeframe)?;
        while m1_cursor < m1_bars.len() && m1_bars[m1_cursor].timestamp_ms < start {
            m1_cursor += 1;
        }
        if m1_bars
            .get(m1_cursor)
            .is_none_or(|bar| bar.timestamp_ms >= end)
        {
            return Err(JudgeError::MissingM1Open {
                timestamp_ms: start,
            });
        }
        let slice_start = m1_cursor;
        while m1_cursor < m1_bars.len() && m1_bars[m1_cursor].timestamp_ms < end {
            m1_cursor += 1;
        }
        let execution_bars = &m1_bars[slice_start..m1_cursor];
        let execution_quotes = &quote_bars[slice_start..m1_cursor];
        let (gap_events, missing_minutes) =
            execution_gap_summary(execution_bars, start, end, execution_interval_ms);
        if gap_events > 0 {
            let tick_volume: u64 = execution_bars.iter().map(|bar| bar.tick_volume).sum();
            let real_volume: u64 = execution_bars.iter().map(|bar| bar.real_volume).sum();
            if tick_volume == decision_bar.tick_volume && real_volume == decision_bar.real_volume {
                telemetry.verified_no_tick_gap_events += gap_events;
                telemetry.verified_no_tick_minutes += missing_minutes;
            } else {
                telemetry.m1_gap_events += gap_events;
                if !config.allow_execution_gaps {
                    return Err(JudgeError::M1Gap {
                        decision_timestamp_ms: start,
                        gap_events,
                    });
                }
            }
        }
        validate_m1_aggregate(decision_bar, execution_bars, broker.tick_size)?;
        telemetry.decision_bars_replayed += 1;

        let opening_minute = &execution_bars[0];
        let opening_quote = &execution_quotes[0];
        let opening_spread_price = opening_quote.spread_open();
        // In canonical mode the sidecar is the source of truth. Never apply a
        // synthetic/fallback SPREAD column to the max-spread gate when the
        // actual bid/ask quote is available.
        let opening_spread_points = if quote_dataset.is_some() {
            opening_spread_price / broker.point
        } else {
            let spread = resolve_spread(opening_minute, broker, &config.costs)?;
            record_spread_source(spread.source, &mut telemetry);
            spread.points
        };
        let mut closed_this_decision = false;
        let previous_decision_bar = &decision_bars[decision_index - 1];
        let previous_spread_price =
            resolve_spread(previous_decision_bar, broker, &config.costs)?.points * broker.point;
        // M1 window for the just-completed decision bar (set at end of prior loop).
        let previous_execution_bars = last_execution_bars;
        let previous_execution_quotes = last_execution_quotes;
        let current_local = broker_clock.local_datetime(opening_minute.timestamp_ms)?;
        let in_close_blackout = current_local.hour() >= strategy.manage.end_of_day_hour as u32;
        let in_entry_window = config.entry_window.contains(current_local.hour());
        let day_key = current_local.date();
        if active_entry_day != Some(day_key) {
            active_entry_day = Some(day_key);
            signal_taken_today = false;
        }

        if strategy.manage.flatten_end_of_day && in_close_blackout {
            // Match the generated EA: 22:00 is exclusively a flatten/cancel
            // cycle and the remaining broker-day bars cannot reopen exposure.
            closed_this_decision = true;
            if pending.take().is_some() {
                telemetry.pending_orders_expired += 1;
            }
            if let Some(open) = position.take() {
                let event = ExitEvent {
                    base_price: market_exit_base_quote(open.side, opening_quote),
                    reason: ExitReason::EndOfDay,
                };
                close_position(
                    open,
                    event,
                    decision_index,
                    opening_minute.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
                telemetry.end_of_day_flattens += 1;
            }
        } else if !in_entry_window && pending.take().is_some() {
            telemetry.pending_orders_expired += 1;
        }

        if !closed_this_decision
            && let (Some(open), Some(previous_timestamp_ms)) =
                (position.as_mut(), last_execution_timestamp_ms)
        {
            apply_swap(
                open,
                if quote_dataset.is_some() {
                    if open.side == PositionSide::Long {
                        opening_quote.bid_open
                    } else {
                        opening_quote.ask_open
                    }
                } else {
                    opening_minute.open
                },
                previous_timestamp_ms,
                opening_minute.timestamp_ms,
                broker,
                &mut balance,
                &mut telemetry,
            )?;
        }

        if !closed_this_decision && let Some(open) = position.as_mut() {
            apply_completed_bar_management(
                open,
                &strategy,
                previous_execution_bars,
                previous_execution_quotes,
                previous_spread_price,
                decision_index,
                opening_minute,
                opening_quote,
                opening_spread_price,
                broker,
                config,
                &mut features,
                &mut balance,
                &mut telemetry,
            )?;
            if open.volume <= 1.0e-12 {
                let open = position.take().expect("managed position exists");
                let side = open.side;
                close_position(
                    open,
                    ExitEvent {
                        base_price: market_exit_base_quote(side, opening_quote),
                        reason: ExitReason::PartialExit,
                    },
                    decision_index,
                    opening_minute.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
                closed_this_decision = true;
            }
        }

        if !closed_this_decision && let Some(open) = position.as_ref() {
            let event =
                if let Some(event) = protective_gap_exit_quote(open, opening_quote, broker) {
                    Some(event)
                } else if let Some(exit) = match open.side {
                    PositionSide::Long => strategy.long_exit(),
                    PositionSide::Short => strategy.short_exit(),
                } && features.evaluate_bool(exit, decision_index)?
                {
                    Some(ExitEvent {
                        base_price: market_exit_base_quote(open.side, opening_quote),
                        reason: ExitReason::Indicator,
                    })
                } else if strategy.manage.time_stop_bars.is_some_and(|limit| {
                    decision_index - open.entry_decision_index >= limit as usize
                }) {
                    Some(ExitEvent {
                        base_price: market_exit_base_quote(open.side, opening_quote),
                        reason: ExitReason::TimeStop,
                    })
                } else {
                    None
                };
            if let Some(event) = event {
                let open = position.take().expect("position was checked above");
                close_position(
                    open,
                    event,
                    decision_index,
                    opening_minute.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
                closed_this_decision = true;
            }
        }

        if pending
            .as_ref()
            .is_some_and(|order| decision_index >= order.expiry_decision_index)
        {
            pending = None;
            telemetry.pending_orders_expired += 1;
        }

        if position.is_none()
            && pending.is_none()
            && !closed_this_decision
            && !(strategy.manage.flatten_end_of_day && in_close_blackout)
            && decision_index >= signal_warmup_bars
        {
            let filters_pass = strategy
                .filters
                .iter()
                .map(|filter| features.evaluate_bool(filter, decision_index))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .all(|value| value);
            if filters_pass {
                let long_signal = strategy
                    .entry
                    .long
                    .as_ref()
                    .map(|entry| features.evaluate_bool(entry, decision_index))
                    .transpose()?
                    .unwrap_or(false);
                let short_signal = strategy
                    .entry
                    .short
                    .as_ref()
                    .map(|entry| features.evaluate_bool(entry, decision_index))
                    .transpose()?
                    .unwrap_or(false);
                let side = match (long_signal, short_signal) {
                    (true, false) => Some(PositionSide::Long),
                    (false, true) => Some(PositionSide::Short),
                    (true, true) => {
                        telemetry.conflicting_entry_signals += 1;
                        None
                    }
                    (false, false) => None,
                };
                if let Some(side) = side {
                    if !broker.is_trading_at(opening_minute.timestamp_ms)? {
                        telemetry.skipped_outside_session += 1;
                    } else if !in_entry_window {
                        telemetry.skipped_outside_entry_window += 1;
                    } else if config
                        .costs
                        .max_spread_points
                        .is_some_and(|maximum| opening_spread_points > maximum)
                    {
                        telemetry.skipped_for_spread += 1;
                    } else if strategy.manage.max_one_entry_per_day && signal_taken_today {
                        telemetry.skipped_max_one_entry_per_day += 1;
                    } else {
                        match &strategy.entry.order {
                            EntryOrderPolicy::Market => {
                                if let Some(open) = open_position(
                                    side,
                                    decision_index,
                                    opening_minute,
                                    opening_spread_price,
                                    opening_quote,
                                    balance,
                                    &strategy,
                                    broker,
                                    config,
                                    &mut features,
                                    &mut telemetry,
                                )? {
                                    balance -= open.entry_commission;
                                    position = Some(open);
                                    signal_taken_today = true;
                                }
                            }
                            EntryOrderPolicy::Stop { .. } | EntryOrderPolicy::Limit { .. } => {
                                if let Some(order) = place_pending_order(
                                    side,
                                    decision_index,
                                    opening_minute,
                                    opening_spread_price,
                                    opening_quote,
                                    balance,
                                    &strategy,
                                    broker,
                                    config,
                                    &mut features,
                                    &mut telemetry,
                                )? {
                                    pending = Some(order);
                                    telemetry.pending_orders_placed += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        for (minute_index, minute) in execution_bars.iter().enumerate() {
            telemetry.m1_bars_replayed += 1;
            let quote = &execution_quotes[minute_index];
            if minute_index > 0
                && let Some(open) = position.as_mut()
            {
                apply_swap(
                    open,
                    if quote_dataset.is_some() {
                        if open.side == PositionSide::Long {
                            quote.bid_open
                        } else {
                            quote.ask_open
                        }
                    } else {
                        minute.open
                    },
                    execution_bars[minute_index - 1].timestamp_ms,
                    minute.timestamp_ms,
                    broker,
                    &mut balance,
                    &mut telemetry,
                )?;
            }
            let minute_local = broker_clock.local_datetime(minute.timestamp_ms)?;
            let minute_in_entry_window = config.entry_window.contains(minute_local.hour());
            if !minute_in_entry_window && pending.take().is_some() {
                telemetry.pending_orders_expired += 1;
            }
            let mut filled_this_minute = false;
            if position.is_none()
                && !closed_this_decision
                && minute_in_entry_window
                && let Some(fill_price) = pending
                    .as_ref()
                    .and_then(|order| pending_fill_price(order, quote))
            {
                let order = pending.take().expect("pending order was checked");
                let open = fill_pending_order(
                    order,
                    decision_index,
                    minute.timestamp_ms,
                    fill_price,
                    &strategy,
                    broker,
                    config,
                );
                balance -= open.entry_commission;
                position = Some(open);
                telemetry.pending_orders_filled += 1;
                signal_taken_today = true;
                filled_this_minute = true;
            }
            if let Some(open) = position.as_ref() {
                let event = if filled_this_minute {
                    protective_intrabar_exit_quote(open, quote, broker, &mut telemetry)
                } else {
                    protective_gap_exit_quote(open, quote, broker).or_else(|| {
                        protective_intrabar_exit_quote(open, quote, broker, &mut telemetry)
                    })
                };
                if let Some(event) = event {
                    let open = position.take().expect("position was checked above");
                    close_position(
                        open,
                        event,
                        decision_index,
                        minute.timestamp_ms,
                        broker,
                        config,
                        &mut balance,
                        &mut trades,
                    );
                }
            }
            let marked_equity = position.as_ref().map_or(balance, |open| {
                liquidation_equity_quote(open, quote, balance, broker, config)
            });
            equity.push(EquityPoint {
                timestamp_ms: minute.timestamp_ms,
                balance,
                equity: marked_equity,
            });
            last_execution_timestamp_ms = Some(minute.timestamp_ms);
        }
        last_execution_bars = execution_bars;
        last_execution_quotes = execution_quotes;
    }

    if let Some(open) = position.take() {
        let final_minute = m1_bars
            .get(m1_cursor.saturating_sub(1))
            .ok_or(JudgeError::InsufficientM1Bars)?;
        let final_quote = quote_bars
            .get(m1_cursor.saturating_sub(1))
            .ok_or(JudgeError::InsufficientM1Bars)?;
        let event = ExitEvent {
            base_price: market_exit_base_quote(open.side, final_quote),
            reason: ExitReason::EndOfData,
        };
        close_position(
            open,
            event,
            decision_bars.len() - 1,
            final_minute.timestamp_ms,
            broker,
            config,
            &mut balance,
            &mut trades,
        );
        if let Some(last) = equity.last_mut() {
            last.balance = balance;
            last.equity = balance;
        }
    }

    let metrics = calculate_metrics(config.initial_balance, balance, &trades, &equity);
    Ok(JudgeResult {
        engine: ENGINE_TIER.into(),
        decision_interval_ms,
        execution_interval_ms,
        trades,
        equity,
        metrics,
        telemetry,
    })
}

fn validate_broker_compatibility(
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
) -> Result<(), JudgeError> {
    if matches!(
        broker.swap_mode,
        SwapMode::ReopenCurrent | SwapMode::ReopenBid
    ) {
        return Err(JudgeError::UnsupportedBrokerFeature(
            "reopen-price swap modes",
        ));
    }
    match (broker.trade_mode, strategy.side) {
        (TradeMode::Disabled | TradeMode::CloseOnly, _)
        | (TradeMode::LongOnly, quantforge_ir::Side::ShortOnly | quantforge_ir::Side::Both)
        | (TradeMode::ShortOnly, quantforge_ir::Side::LongOnly | quantforge_ir::Side::Both) => {
            Err(JudgeError::IncompatibleBrokerTradeMode)
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn open_position(
    side: PositionSide,
    decision_index: usize,
    bar: &Bar,
    _spread_price: f64,
    quote: &QuoteBar,
    balance: f64,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
    features: &mut FeatureCache<'_>,
    telemetry: &mut JudgeTelemetry,
) -> Result<Option<OpenPosition>, JudgeError> {
    let Some(stop_distance) = stop_distance(strategy, decision_index, broker, features)? else {
        return Ok(None);
    };
    let Some(target_distance) =
        target_distance(strategy, decision_index, broker, features, stop_distance)?
    else {
        return Ok(None);
    };
    let minimum_distance = broker.stops_level_points as f64 * broker.point;
    if stop_distance < minimum_distance || target_distance < minimum_distance {
        telemetry.skipped_for_broker_stop_level += 1;
        return Ok(None);
    }

    let risk_budget = match strategy.risk {
        RiskPolicy::FixedCurrency { amount } => amount,
        RiskPolicy::PercentBalance { percent } => balance * percent / 100.0,
    };
    let price_risk_per_lot = stop_distance / broker.tick_size * broker.tick_value;
    let cost_risk_per_lot = if config.costs.include_costs_in_risk {
        config.costs.commission_per_lot_round_turn
            + 2.0 * config.costs.adverse_slippage_points_per_side * broker.point / broker.tick_size
                * broker.tick_value
    } else {
        0.0
    };
    let raw_volume = risk_budget / (price_risk_per_lot + cost_risk_per_lot);
    let Some(volume) = normalize_volume(raw_volume, broker) else {
        telemetry.skipped_below_minimum_volume += 1;
        return Ok(None);
    };
    let margin_reference_price = match side {
        PositionSide::Long => quote.ask_open,
        PositionSide::Short => quote.bid_open,
    };
    if !margin_is_affordable(balance, volume, broker, margin_reference_price) {
        telemetry.skipped_insufficient_margin += 1;
        return Ok(None);
    }

    let slippage = config.costs.adverse_slippage_points_per_side * broker.point;
    let intended_entry_price = normalize_price(
        match side {
            PositionSide::Long => quote.ask_open,
            PositionSide::Short => quote.bid_open,
        },
        broker,
    );
    let entry_price = normalize_price(
        match side {
            PositionSide::Long => intended_entry_price + slippage,
            PositionSide::Short => intended_entry_price - slippage,
        },
        broker,
    );
    let (stop_loss, take_profit) = match side {
        PositionSide::Long => (
            normalize_price(intended_entry_price - stop_distance, broker),
            normalize_price(intended_entry_price + target_distance, broker),
        ),
        PositionSide::Short => (
            normalize_price(intended_entry_price + stop_distance, broker),
            normalize_price(intended_entry_price - target_distance, broker),
        ),
    };
    let initial_risk_distance = (entry_price - stop_loss).abs();
    if initial_risk_distance <= 0.0 {
        return Ok(None);
    }
    let initial_dollar_risk = stop_dollar_risk(
        initial_risk_distance,
        volume,
        broker.tick_size,
        broker.tick_value,
    );
    Ok(Some(OpenPosition {
        side,
        entry_decision_index: decision_index,
        entry_timestamp_ms: bar.timestamp_ms,
        entry_price,
        initial_volume: volume,
        volume,
        stop_loss,
        take_profit,
        initial_stop_loss: stop_loss,
        initial_take_profit: take_profit,
        initial_risk_distance,
        initial_dollar_risk,
        peak_favorable_price: None,
        entry_commission: volume * config.costs.commission_per_lot_round_turn / 2.0,
        swap: 0.0,
        realized_gross_profit: 0.0,
        realized_exit_commission: 0.0,
        exited_volume: 0.0,
        weighted_exit_price: 0.0,
        partial_exit_done: vec![false; strategy.manage.partial_exits.len()],
    }))
}

#[allow(clippy::too_many_arguments)]
fn place_pending_order(
    side: PositionSide,
    decision_index: usize,
    _bar: &Bar,
    _spread_price: f64,
    quote: &QuoteBar,
    balance: f64,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
    features: &mut FeatureCache<'_>,
    telemetry: &mut JudgeTelemetry,
) -> Result<Option<PendingOrder>, JudgeError> {
    let (kind, distance_policy, expiry_bars) = match &strategy.entry.order {
        EntryOrderPolicy::Market => return Ok(None),
        EntryOrderPolicy::Stop {
            distance,
            expiry_bars,
        } => (PendingKind::Stop, distance, *expiry_bars),
        EntryOrderPolicy::Limit {
            distance,
            expiry_bars,
        } => (PendingKind::Limit, distance, *expiry_bars),
    };
    let Some(entry_distance) = entry_distance(distance_policy, decision_index, broker, features)?
    else {
        return Ok(None);
    };
    let Some(stop_distance) = stop_distance(strategy, decision_index, broker, features)? else {
        return Ok(None);
    };
    let Some(target_distance) =
        target_distance(strategy, decision_index, broker, features, stop_distance)?
    else {
        return Ok(None);
    };
    let minimum_distance = broker.stops_level_points as f64 * broker.point;
    if entry_distance < minimum_distance
        || stop_distance < minimum_distance
        || target_distance < minimum_distance
    {
        telemetry.skipped_for_broker_stop_level += 1;
        return Ok(None);
    }
    let reference = match side {
        PositionSide::Long => quote.ask_open,
        PositionSide::Short => quote.bid_open,
    };
    let activation_price = normalize_price(
        match (side, kind) {
            (PositionSide::Long, PendingKind::Stop) => reference + entry_distance,
            (PositionSide::Short, PendingKind::Stop) => reference - entry_distance,
            (PositionSide::Long, PendingKind::Limit) => reference - entry_distance,
            (PositionSide::Short, PendingKind::Limit) => reference + entry_distance,
        },
        broker,
    );
    let (stop_loss, take_profit) = match side {
        PositionSide::Long => (
            normalize_price(activation_price - stop_distance, broker),
            normalize_price(activation_price + target_distance, broker),
        ),
        PositionSide::Short => (
            normalize_price(activation_price + stop_distance, broker),
            normalize_price(activation_price - target_distance, broker),
        ),
    };
    let normalized_stop_distance = (activation_price - stop_loss).abs();
    if normalized_stop_distance <= 0.0 {
        return Ok(None);
    }
    let risk_budget = match strategy.risk {
        RiskPolicy::FixedCurrency { amount } => amount,
        RiskPolicy::PercentBalance { percent } => balance * percent / 100.0,
    };
    let price_risk_per_lot = normalized_stop_distance / broker.tick_size * broker.tick_value;
    let cost_risk_per_lot = if config.costs.include_costs_in_risk {
        config.costs.commission_per_lot_round_turn
            + 2.0 * config.costs.adverse_slippage_points_per_side * broker.point / broker.tick_size
                * broker.tick_value
    } else {
        0.0
    };
    let Some(volume) = normalize_volume(
        risk_budget / (price_risk_per_lot + cost_risk_per_lot),
        broker,
    ) else {
        telemetry.skipped_below_minimum_volume += 1;
        return Ok(None);
    };
    if !margin_is_affordable(balance, volume, broker, activation_price) {
        telemetry.skipped_insufficient_margin += 1;
        return Ok(None);
    }
    Ok(Some(PendingOrder {
        side,
        kind,
        expiry_decision_index: decision_index.saturating_add(expiry_bars as usize),
        activation_price,
        stop_loss,
        take_profit,
        stop_distance: normalized_stop_distance,
        volume,
    }))
}

fn entry_distance(
    policy: &EntryDistancePolicy,
    decision_index: usize,
    broker: &SymbolSpecification,
    features: &mut FeatureCache<'_>,
) -> Result<Option<f64>, JudgeError> {
    let value = match *policy {
        EntryDistancePolicy::FixedPoints { points } => Some(points * broker.point),
        EntryDistancePolicy::AtrMultiple { period, multiplier } => features
            .indicator_at_decision(&IndicatorExpr::Atr { period, shift: 1 }, decision_index)?
            .map(|atr| atr * multiplier),
        EntryDistancePolicy::RangeMultiple { period, multiplier } => {
            average_completed_range(features.bars_for_eval(), decision_index, period as usize)
                .map(|range| range * multiplier)
        }
    };
    Ok(value.filter(|distance| distance.is_finite() && *distance > 0.0))
}

fn pending_fill_price(order: &PendingOrder, quote: &QuoteBar) -> Option<f64> {
    let open = match order.side {
        PositionSide::Long => quote.ask_open,
        PositionSide::Short => quote.bid_open,
    };
    let touched = match (order.side, order.kind) {
        (PositionSide::Long, PendingKind::Stop) => quote.ask_high >= order.activation_price,
        (PositionSide::Short, PendingKind::Stop) => quote.bid_low <= order.activation_price,
        (PositionSide::Long, PendingKind::Limit) => quote.ask_low <= order.activation_price,
        (PositionSide::Short, PendingKind::Limit) => quote.bid_high >= order.activation_price,
    };
    if !touched {
        return None;
    }
    Some(match (order.side, order.kind) {
        (PositionSide::Long, PendingKind::Stop) => open.max(order.activation_price),
        (PositionSide::Short, PendingKind::Stop) => open.min(order.activation_price),
        (PositionSide::Long, PendingKind::Limit) => open.min(order.activation_price),
        (PositionSide::Short, PendingKind::Limit) => open.max(order.activation_price),
    })
}

fn favorable_quote_sample(
    side: PositionSide,
    bars: &[Bar],
    quotes: &[QuoteBar],
    entry_timestamp_ms: i64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for (bar, quote) in bars.iter().zip(quotes) {
        if bar.timestamp_ms < entry_timestamp_ms {
            continue;
        }
        let sample = if bar.timestamp_ms == entry_timestamp_ms {
            match side {
                PositionSide::Long => quote.bid_close,
                PositionSide::Short => quote.ask_close,
            }
        } else {
            match side {
                PositionSide::Long => quote.bid_high,
                PositionSide::Short => quote.ask_low,
            }
        };
        best = Some(match (side, best) {
            (PositionSide::Long, Some(current)) => current.max(sample),
            (PositionSide::Short, Some(current)) => current.min(sample),
            (_, None) => sample,
        });
    }
    best
}

fn protective_gap_exit_quote(
    position: &OpenPosition,
    quote: &QuoteBar,
    broker: &SymbolSpecification,
) -> Option<ExitEvent> {
    let open = match position.side {
        PositionSide::Long => quote.bid_open,
        PositionSide::Short => quote.ask_open,
    };
    match position.side {
        PositionSide::Long if price_reaches_from_above(open, position.stop_loss, broker) => {
            Some(ExitEvent {
                // MT5's Model=1 OHLC engine records protective stops at the
                // requested stop price even when the synthesized minute opens
                // beyond it. Real-tick audits may model broker gap slippage,
                // but canonical M1 certification must reproduce Model=1.
                base_price: position.stop_loss,
                reason: ExitReason::StopLoss,
            })
        }
        PositionSide::Long if price_reaches_from_below(open, position.take_profit, broker) => {
            Some(ExitEvent {
                base_price: position.take_profit,
                reason: ExitReason::TakeProfit,
            })
        }
        PositionSide::Short if price_reaches_from_below(open, position.stop_loss, broker) => {
            Some(ExitEvent {
                base_price: position.stop_loss,
                reason: ExitReason::StopLoss,
            })
        }
        PositionSide::Short if price_reaches_from_above(open, position.take_profit, broker) => {
            Some(ExitEvent {
                base_price: position.take_profit,
                reason: ExitReason::TakeProfit,
            })
        }
        _ => None,
    }
}

fn protective_intrabar_exit_quote(
    position: &OpenPosition,
    quote: &QuoteBar,
    broker: &SymbolSpecification,
    telemetry: &mut JudgeTelemetry,
) -> Option<ExitEvent> {
    let (stop_touched, target_touched) = match position.side {
        PositionSide::Long => (
            price_reaches_from_above(quote.bid_low, position.stop_loss, broker),
            price_reaches_from_below(quote.bid_high, position.take_profit, broker),
        ),
        PositionSide::Short => (
            price_reaches_from_below(quote.ask_high, position.stop_loss, broker),
            price_reaches_from_above(quote.ask_low, position.take_profit, broker),
        ),
    };
    if stop_touched && target_touched {
        telemetry.same_minute_stop_target_collisions += 1;
    }
    if stop_touched {
        Some(ExitEvent {
            base_price: position.stop_loss,
            reason: ExitReason::StopLoss,
        })
    } else if target_touched {
        Some(ExitEvent {
            base_price: position.take_profit,
            reason: ExitReason::TakeProfit,
        })
    } else {
        None
    }
}

fn fill_pending_order(
    order: PendingOrder,
    decision_index: usize,
    timestamp_ms: i64,
    fill_base_price: f64,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
) -> OpenPosition {
    let slippage = config.costs.adverse_slippage_points_per_side * broker.point;
    let entry_price = normalize_price(
        match order.side {
            PositionSide::Long => fill_base_price + slippage,
            PositionSide::Short => fill_base_price - slippage,
        },
        broker,
    );
    let stop_loss = normalize_price(order.stop_loss, broker);
    let take_profit = normalize_price(order.take_profit, broker);
    let initial_risk_distance = (entry_price - stop_loss).abs().max(order.stop_distance);
    let initial_dollar_risk = stop_dollar_risk(
        initial_risk_distance,
        order.volume,
        broker.tick_size,
        broker.tick_value,
    );
    OpenPosition {
        side: order.side,
        entry_decision_index: decision_index,
        entry_timestamp_ms: timestamp_ms,
        entry_price,
        initial_volume: order.volume,
        volume: order.volume,
        stop_loss,
        take_profit,
        initial_stop_loss: stop_loss,
        initial_take_profit: take_profit,
        initial_risk_distance,
        initial_dollar_risk,
        peak_favorable_price: None,
        entry_commission: order.volume * config.costs.commission_per_lot_round_turn / 2.0,
        swap: 0.0,
        realized_gross_profit: 0.0,
        realized_exit_commission: 0.0,
        exited_volume: 0.0,
        weighted_exit_price: 0.0,
        partial_exit_done: vec![false; strategy.manage.partial_exits.len()],
    }
}

fn stop_distance(
    strategy: &StrategyIr,
    decision_index: usize,
    broker: &SymbolSpecification,
    features: &mut FeatureCache<'_>,
) -> Result<Option<f64>, JudgeError> {
    let distance = match strategy.stops.stop_loss {
        StopLossPolicy::FixedPoints { points } => Some(points * broker.point),
        StopLossPolicy::AtrMultiple { period, multiplier } => features
            .indicator_at_decision(&IndicatorExpr::Atr { period, shift: 1 }, decision_index)?
            .map(|atr| atr * multiplier),
        StopLossPolicy::RangeMultiple { period, multiplier } => {
            average_completed_range(features.bars_for_eval(), decision_index, period as usize)
                .map(|range| range * multiplier)
        }
    }
    .filter(|distance| distance.is_finite() && *distance > 0.0);
    Ok(distance)
}

fn target_distance(
    strategy: &StrategyIr,
    decision_index: usize,
    broker: &SymbolSpecification,
    features: &mut FeatureCache<'_>,
    stop_distance: f64,
) -> Result<Option<f64>, JudgeError> {
    let distance = match strategy.stops.take_profit {
        TakeProfitPolicy::RiskMultiple { multiple } => Some(stop_distance * multiple),
        TakeProfitPolicy::FixedPoints { points } => Some(points * broker.point),
        TakeProfitPolicy::AtrMultiple { period, multiplier } => features
            .indicator_at_decision(&IndicatorExpr::Atr { period, shift: 1 }, decision_index)?
            .map(|atr| atr * multiplier),
    }
    .filter(|distance| distance.is_finite() && *distance > 0.0);
    Ok(distance)
}

fn average_completed_range(bars: &[Bar], decision_index: usize, period: usize) -> Option<f64> {
    let end = decision_index.checked_sub(1)?;
    let start = end.checked_add(1)?.checked_sub(period)?;
    let window = bars.get(start..=end)?;
    Some(window.iter().map(|bar| bar.high - bar.low).sum::<f64>() / period as f64)
}

fn normalize_volume(raw_volume: f64, broker: &SymbolSpecification) -> Option<f64> {
    if !raw_volume.is_finite() || raw_volume <= 0.0 {
        return None;
    }
    let steps = (raw_volume / broker.volume_step + 1.0e-12).floor();
    let volume = (steps * broker.volume_step).min(broker.volume_max);
    (volume + 1.0e-12 >= broker.volume_min).then_some(volume)
}

fn margin_is_affordable(
    balance: f64,
    volume: f64,
    broker: &SymbolSpecification,
    reference_price: f64,
) -> bool {
    if !balance.is_finite() || balance <= 0.0 || !reference_price.is_finite() {
        return false;
    }
    // Some MT5 symbols (notably indices and crypto) omit an initial-margin
    // value from the exported broker profile. In that case use the standard
    // contract-value/leverage estimate so QF cannot keep opening fixed-risk
    // orders after MT5 would reject them for insufficient free margin.
    let raw_margin_per_lot = broker
        .margin_initial_per_lot
        .unwrap_or(broker.contract_size * reference_price);
    if !raw_margin_per_lot.is_finite() || raw_margin_per_lot <= 0.0 {
        return false;
    }
    let required = raw_margin_per_lot / PARITY_TESTER_LEVERAGE * volume;
    required.is_finite() && balance + 1.0e-9 >= required
}

#[allow(clippy::too_many_arguments)]
fn apply_completed_bar_management(
    position: &mut OpenPosition,
    strategy: &StrategyIr,
    previous_execution_bars: &[Bar],
    previous_execution_quotes: &[QuoteBar],
    _completed_spread_price: f64,
    decision_index: usize,
    _current_bar: &Bar,
    current_quote: &QuoteBar,
    _current_spread_price: f64,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
    features: &mut FeatureCache<'_>,
    balance: &mut f64,
    telemetry: &mut JudgeTelemetry,
) -> Result<(), JudgeError> {
    let Some(sample) = favorable_quote_sample(
        position.side,
        previous_execution_bars,
        previous_execution_quotes,
        position.entry_timestamp_ms,
    ) else {
        return Ok(());
    };
    position.peak_favorable_price =
        ratchet_favorable_peak(position.side, position.peak_favorable_price, sample);
    let Some(favorable_price) = position.peak_favorable_price else {
        return Ok(());
    };
    let favorable_r = compute_favorable_r(
        position.side,
        favorable_price,
        position.entry_price,
        position.initial_risk_distance,
    );
    let minimum_distance = broker.stops_level_points as f64 * broker.point;

    if strategy
        .manage
        .break_even_at_r
        .is_some_and(|activation| favorable_r >= activation)
    {
        let reference_open = match position.side {
            PositionSide::Long => current_quote.bid_open,
            PositionSide::Short => current_quote.ask_open,
        };
        if let Some(candidate) = placeable_stop_candidate(
            position.side,
            position.entry_price,
            reference_open,
            current_quote.spread_open(),
            minimum_distance,
        ) {
            if tighten_stop(position, candidate) {
                telemetry.break_even_moves += 1;
            }
        }
    }

    if let Some(trailing) = &strategy.manage.trailing {
        let (activate_at_r, distance) = match *trailing {
            TrailingPolicy::RiskMultiple {
                activate_at_r,
                distance_r,
            } => (
                activate_at_r,
                Some(distance_r * position.initial_risk_distance),
            ),
            TrailingPolicy::AtrMultiple {
                activate_at_r,
                period,
                multiplier,
            } => (
                activate_at_r,
                features
                    .indicator_at_decision(
                        &IndicatorExpr::Atr { period, shift: 1 },
                        decision_index,
                    )?
                    .map(|atr| atr * multiplier),
            ),
        };
        if favorable_r >= activate_at_r
            && let Some(distance) = distance.filter(|value| value.is_finite() && *value > 0.0)
        {
            let raw_candidate = match position.side {
                PositionSide::Long => favorable_price - distance,
                PositionSide::Short => favorable_price + distance,
            };
            let reference_open = match position.side {
                PositionSide::Long => current_quote.bid_open,
                PositionSide::Short => current_quote.ask_open,
            };
            if let Some(candidate) = placeable_stop_candidate(
                position.side,
                raw_candidate,
                reference_open,
                current_quote.spread_open(),
                minimum_distance,
            ) {
                if tighten_stop(position, candidate) {
                    telemetry.trailing_stop_moves += 1;
                }
            }
        }
    }

    for (partial_index, partial) in strategy.manage.partial_exits.iter().enumerate() {
        if position.partial_exit_done[partial_index] || favorable_r < partial.at_r {
            continue;
        }
        let requested = position.initial_volume * partial.fraction;
        let Some(close_volume) = normalize_partial_volume(requested, position.volume, broker)
        else {
            continue;
        };
        let base_price = market_exit_base_quote(position.side, current_quote);
        realize_exit_volume(position, close_volume, base_price, broker, config, balance);
        position.partial_exit_done[partial_index] = true;
        telemetry.partial_exits_executed += 1;
    }
    Ok(())
}

fn tighten_stop(position: &mut OpenPosition, candidate: f64) -> bool {
    if !candidate.is_finite() {
        return false;
    }
    match position.side {
        PositionSide::Long if candidate > position.stop_loss + 1.0e-12 => {
            position.stop_loss = candidate;
            true
        }
        PositionSide::Short if candidate < position.stop_loss - 1.0e-12 => {
            position.stop_loss = candidate;
            true
        }
        _ => false,
    }
}

fn normalize_partial_volume(
    requested: f64,
    remaining: f64,
    broker: &SymbolSpecification,
) -> Option<f64> {
    if requested + 1.0e-12 >= remaining {
        return Some(remaining);
    }
    let steps = (requested / broker.volume_step + 1.0e-12).floor();
    let mut volume = steps * broker.volume_step;
    if volume + 1.0e-12 < broker.volume_min {
        return None;
    }
    if remaining - volume + 1.0e-12 < broker.volume_min {
        volume = remaining;
    }
    Some(volume.min(remaining))
}

fn realize_exit_volume(
    position: &mut OpenPosition,
    volume: f64,
    base_price: f64,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
    balance: &mut f64,
) {
    if volume <= 0.0 {
        return;
    }
    let slippage = config.costs.adverse_slippage_points_per_side * broker.point;
    let exit_price = match position.side {
        PositionSide::Long => base_price - slippage,
        PositionSide::Short => base_price + slippage,
    };
    let direction = match position.side {
        PositionSide::Long => 1.0,
        PositionSide::Short => -1.0,
    };
    let gross_profit = (exit_price - position.entry_price)
        * direction
        * broker.contract_size
        * profit_currency_to_account(exit_price, broker)
        * volume;
    let exit_commission = volume * config.costs.commission_per_lot_round_turn / 2.0;
    *balance += gross_profit - exit_commission;
    position.realized_gross_profit += gross_profit;
    position.realized_exit_commission += exit_commission;
    position.exited_volume += volume;
    position.weighted_exit_price += exit_price * volume;
    position.volume = (position.volume - volume).max(0.0);
}

fn record_spread_source(source: SpreadSource, telemetry: &mut JudgeTelemetry) {
    match source {
        SpreadSource::Recorded => {}
        SpreadSource::BrokerWindow => telemetry.synthetic_spread_bars += 1,
        SpreadSource::ExplicitFallback => telemetry.fallback_spread_bars += 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_swap(
    position: &mut OpenPosition,
    current_price: f64,
    previous_timestamp_ms: i64,
    current_timestamp_ms: i64,
    broker: &SymbolSpecification,
    balance: &mut f64,
    telemetry: &mut JudgeTelemetry,
) -> Result<(), JudgeError> {
    let accrual = accrue_swap(
        position.side,
        position.volume,
        position.entry_price,
        current_price,
        previous_timestamp_ms,
        current_timestamp_ms,
        broker,
    )?;
    position.swap += accrual.cash;
    *balance += accrual.cash;
    telemetry.swap_rollover_events += accrual.rollover_events;
    telemetry.swap_effective_days += accrual.effective_days;
    Ok(())
}

#[allow(dead_code)]
fn protective_gap_exit(
    position: &OpenPosition,
    bar: &Bar,
    spread_price: f64,
    broker: &SymbolSpecification,
) -> Option<ExitEvent> {
    match position.side {
        PositionSide::Long if price_reaches_from_above(bar.open, position.stop_loss, broker) => {
            Some(ExitEvent {
                base_price: bar.open,
                reason: ExitReason::StopLoss,
            })
        }
        PositionSide::Long if price_reaches_from_below(bar.open, position.take_profit, broker) => {
            Some(ExitEvent {
                base_price: position.take_profit,
                reason: ExitReason::TakeProfit,
            })
        }
        PositionSide::Short
            if price_reaches_from_below(bar.open + spread_price, position.stop_loss, broker) =>
        {
            Some(ExitEvent {
                base_price: bar.open + spread_price,
                reason: ExitReason::StopLoss,
            })
        }
        PositionSide::Short
            if price_reaches_from_above(bar.open + spread_price, position.take_profit, broker) =>
        {
            Some(ExitEvent {
                base_price: position.take_profit,
                reason: ExitReason::TakeProfit,
            })
        }
        _ => None,
    }
}

#[allow(dead_code)]
fn protective_intrabar_exit(
    position: &OpenPosition,
    bar: &Bar,
    spread_price: f64,
    broker: &SymbolSpecification,
    telemetry: &mut JudgeTelemetry,
) -> Option<ExitEvent> {
    let (stop_touched, target_touched) = match position.side {
        PositionSide::Long => (
            price_reaches_from_above(bar.low, position.stop_loss, broker),
            price_reaches_from_below(bar.high, position.take_profit, broker),
        ),
        PositionSide::Short => (
            price_reaches_from_below(bar.high + spread_price, position.stop_loss, broker),
            price_reaches_from_above(bar.low + spread_price, position.take_profit, broker),
        ),
    };
    if stop_touched && target_touched {
        telemetry.same_minute_stop_target_collisions += 1;
    }
    if stop_touched {
        Some(ExitEvent {
            base_price: position.stop_loss,
            reason: ExitReason::StopLoss,
        })
    } else if target_touched {
        Some(ExitEvent {
            base_price: position.take_profit,
            reason: ExitReason::TakeProfit,
        })
    } else {
        None
    }
}

#[allow(clippy::too_many_arguments)]
fn close_position(
    mut position: OpenPosition,
    event: ExitEvent,
    decision_index: usize,
    exit_timestamp_ms: i64,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
    balance: &mut f64,
    trades: &mut Vec<Trade>,
) {
    let remaining = position.volume;
    realize_exit_volume(
        &mut position,
        remaining,
        event.base_price,
        broker,
        config,
        balance,
    );
    let exit_price = if position.exited_volume > 0.0 {
        position.weighted_exit_price / position.exited_volume
    } else {
        position.entry_price
    };
    let gross_profit = position.realized_gross_profit;
    let commission = position.entry_commission + position.realized_exit_commission;
    let net_profit = gross_profit - commission + position.swap;
    let r_multiple = r_multiple(net_profit, position.initial_dollar_risk);
    trades.push(Trade {
        side: position.side,
        entry_timestamp_ms: position.entry_timestamp_ms,
        exit_timestamp_ms,
        entry_price: position.entry_price,
        exit_price,
        volume: position.initial_volume,
        initial_stop_loss: position.initial_stop_loss,
        initial_take_profit: position.initial_take_profit,
        gross_profit,
        commission,
        swap: position.swap,
        net_profit,
        bars_held: decision_index - position.entry_decision_index,
        exit_reason: event.reason,
        r_multiple,
    });
}

#[allow(dead_code)]
fn market_exit_base(side: PositionSide, bid_price: f64, spread_price: f64) -> f64 {
    match side {
        PositionSide::Long => bid_price,
        PositionSide::Short => bid_price + spread_price,
    }
}

fn market_exit_base_quote(side: PositionSide, quote: &QuoteBar) -> f64 {
    match side {
        PositionSide::Long => quote.bid_open,
        PositionSide::Short => quote.ask_open,
    }
}

/// Match MT5's dynamic quote-to-account conversion for account-base crosses
/// while keeping account-quoted instruments at a one-to-one conversion.
fn profit_currency_to_account(price: f64, broker: &SymbolSpecification) -> f64 {
    if broker.profit_currency == broker.account_currency {
        1.0
    } else if broker.base_currency == broker.account_currency && price.is_finite() && price > 0.0 {
        1.0 / price
    } else {
        broker.tick_value / (broker.tick_size * broker.contract_size)
    }
}

#[allow(dead_code)]
fn liquidation_equity(
    position: &OpenPosition,
    bar: &Bar,
    spread_price: f64,
    balance: f64,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
) -> f64 {
    let exit_price = market_exit_base(position.side, bar.close, spread_price);
    let direction = match position.side {
        PositionSide::Long => 1.0,
        PositionSide::Short => -1.0,
    };
    let gross = (exit_price - position.entry_price)
        * direction
        * broker.contract_size
        * profit_currency_to_account(exit_price, broker)
        * position.volume;
    let exit_commission = position.volume * config.costs.commission_per_lot_round_turn / 2.0;
    balance + gross - exit_commission
}

fn liquidation_equity_quote(
    position: &OpenPosition,
    quote: &QuoteBar,
    balance: f64,
    broker: &SymbolSpecification,
    config: &JudgeConfig,
) -> f64 {
    let exit_price = match position.side {
        PositionSide::Long => quote.bid_close,
        PositionSide::Short => quote.ask_close,
    };
    let direction = match position.side {
        PositionSide::Long => 1.0,
        PositionSide::Short => -1.0,
    };
    let gross = (exit_price - position.entry_price)
        * direction
        * broker.contract_size
        * profit_currency_to_account(exit_price, broker)
        * position.volume;
    let exit_commission = position.volume * config.costs.commission_per_lot_round_turn / 2.0;
    balance + gross - exit_commission
}

fn derive_quote_bars(
    bars: &[Bar],
    broker: &SymbolSpecification,
    costs: &CostModel,
) -> Result<Vec<QuoteBar>, JudgeError> {
    bars.iter()
        .map(|bar| {
            let spread = resolve_spread(bar, broker, costs)?.points * broker.point;
            Ok(QuoteBar {
                timestamp_ms: bar.timestamp_ms,
                bid_open: bar.open,
                bid_high: bar.high,
                bid_low: bar.low,
                bid_close: bar.close,
                ask_open: bar.open + spread,
                ask_high: bar.high + spread,
                ask_low: bar.low + spread,
                ask_close: bar.close + spread,
                tick_count: bar.tick_volume,
            })
        })
        .collect()
}

fn calculate_metrics(
    initial_balance: f64,
    ending_balance: f64,
    trades: &[Trade],
    equity: &[EquityPoint],
) -> BacktestMetrics {
    let winning_trades = trades.iter().filter(|trade| trade.net_profit > 0.0).count();
    let losing_trades = trades.iter().filter(|trade| trade.net_profit < 0.0).count();
    let gross_wins = trades
        .iter()
        .filter(|trade| trade.net_profit > 0.0)
        .map(|trade| trade.net_profit)
        .sum::<f64>()
        .max(0.0);
    let gross_losses = -trades
        .iter()
        .filter(|trade| trade.net_profit < 0.0)
        .map(|trade| trade.net_profit)
        .sum::<f64>();
    let profit_factor = (gross_losses > 0.0).then_some(gross_wins / gross_losses);
    let mut peak = initial_balance;
    let mut max_drawdown = 0.0_f64;
    let mut max_drawdown_percent = 0.0_f64;
    for point in equity {
        peak = peak.max(point.equity);
        let drawdown = peak - point.equity;
        max_drawdown = max_drawdown.max(drawdown);
        if peak > 0.0 {
            max_drawdown_percent = max_drawdown_percent.max(drawdown / peak * 100.0);
        }
    }
    let net_profit = ending_balance - initial_balance;
    let sharpe_ratio = equity_sharpe_ratio(initial_balance, equity);
    let expectancy = if trades.is_empty() {
        0.0
    } else {
        trades.iter().map(|trade| trade.net_profit).sum::<f64>() / trades.len() as f64
    };
    let (expectancy_r, median_r) = trade_r_stats(trades);
    BacktestMetrics {
        initial_balance,
        ending_balance,
        net_profit,
        return_percent: net_profit / initial_balance * 100.0,
        trade_count: trades.len(),
        winning_trades,
        losing_trades,
        win_rate: if trades.is_empty() {
            0.0
        } else {
            winning_trades as f64 / trades.len() as f64 * 100.0
        },
        profit_factor,
        max_drawdown,
        max_drawdown_percent,
        sharpe_ratio,
        expectancy,
        expectancy_r,
        median_r,
    }
}

fn median_interval(bars: &[Bar]) -> Option<i64> {
    let mut intervals: Vec<i64> = bars
        .windows(2)
        .map(|pair| pair[1].timestamp_ms - pair[0].timestamp_ms)
        .filter(|interval| *interval > 0)
        .collect();
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_unstable();
    Some(intervals[intervals.len() / 2])
}

fn validate_m1_aggregate(
    decision: &Bar,
    execution: &[Bar],
    tick_size: f64,
) -> Result<(), JudgeError> {
    let aggregate = [
        ("open", decision.open, execution[0].open),
        (
            "high",
            decision.high,
            execution
                .iter()
                .map(|bar| bar.high)
                .fold(f64::NEG_INFINITY, f64::max),
        ),
        (
            "low",
            decision.low,
            execution
                .iter()
                .map(|bar| bar.low)
                .fold(f64::INFINITY, f64::min),
        ),
        (
            "close",
            decision.close,
            execution.last().expect("execution is non-empty").close,
        ),
    ];
    let tolerance = tick_size * 0.5 + f64::EPSILON;
    for (field, decision_value, execution_value) in aggregate {
        if (decision_value - execution_value).abs() > tolerance {
            return Err(JudgeError::M1AggregateMismatch {
                timestamp_ms: decision.timestamp_ms,
                field,
                decision_value,
                execution_value,
            });
        }
    }
    Ok(())
}

fn execution_gap_summary(
    execution: &[Bar],
    start_ms: i64,
    end_ms: i64,
    interval_ms: i64,
) -> (usize, usize) {
    let first = execution
        .first()
        .expect("execution gap summary requires at least one bar")
        .timestamp_ms;
    let last = execution
        .last()
        .expect("execution gap summary requires at least one bar")
        .timestamp_ms;
    let leading_minutes = ((first - start_ms) / interval_ms).max(0) as usize;
    let trailing_minutes = ((end_ms - interval_ms - last) / interval_ms).max(0) as usize;
    let leading_events = usize::from(leading_minutes > 0);
    let trailing_events = usize::from(trailing_minutes > 0);
    let (internal_events, internal_minutes) =
        execution
            .windows(2)
            .fold((0, 0), |(events, minutes), pair| {
                let delta = pair[1].timestamp_ms - pair[0].timestamp_ms;
                if delta == interval_ms {
                    (events, minutes)
                } else {
                    let missing = if delta > interval_ms {
                        (delta / interval_ms - 1) as usize
                    } else {
                        0
                    };
                    (events + 1, minutes + missing)
                }
            });
    (
        leading_events + internal_events + trailing_events,
        leading_minutes + internal_minutes + trailing_minutes,
    )
}

#[derive(Debug, Error)]
pub enum JudgeError {
    #[error("at least two decision-timeframe bars are required")]
    InsufficientDecisionBars,
    #[error("at least two M1 execution bars are required")]
    InsufficientM1Bars,
    #[error("decision bars must use a fixed whole-minute timeframe")]
    InvalidDecisionTimeframe,
    #[error("execution data must be M1; observed median interval was {observed_ms}ms")]
    InvalidM1Timeframe { observed_ms: i64 },
    #[error("quote sidecar mismatch: {0}")]
    QuoteDatasetMismatch(String),
    #[error("M1 data has no bar at decision open {timestamp_ms}")]
    MissingM1Open { timestamp_ms: i64 },
    #[error(
        "M1 coverage for decision bar {decision_timestamp_ms} does not reach expected final minute {expected_last_timestamp_ms}"
    )]
    IncompleteM1Coverage {
        decision_timestamp_ms: i64,
        expected_last_timestamp_ms: i64,
    },
    #[error("M1 data contains {gap_events} in-bar gaps at decision time {decision_timestamp_ms}")]
    M1Gap {
        decision_timestamp_ms: i64,
        gap_events: usize,
    },
    #[error(
        "M1 {field} aggregate at {timestamp_ms} is {execution_value}, decision bar is {decision_value}"
    )]
    M1AggregateMismatch {
        timestamp_ms: i64,
        field: &'static str,
        decision_value: f64,
        execution_value: f64,
    },
    #[error(
        "timezone mismatch: broker={broker}, decision_data={decision}, execution_data={execution}"
    )]
    TimezoneMismatch {
        broker: String,
        decision: String,
        execution: String,
    },
    #[error("unsupported broker feature in M1 judge v1: {0}")]
    UnsupportedBrokerFeature(&'static str),
    #[error("strategy side is incompatible with broker trade mode")]
    IncompatibleBrokerTradeMode,
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error(transparent)]
    Broker(#[from] BrokerSpecError),
    #[error(transparent)]
    Ir(#[from] IrError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_broker::{DayOfWeek, FillingMode};
    use quantforge_core::{ContentHash, STRATEGY_IR_VERSION};
    use quantforge_eval::{SameBarPolicy, evaluate_strategy};
    use quantforge_ir::{
        BoolExpr, ComparisonOp, EntryDistancePolicy, EntryOrderPolicy, EntrySignals, ManagePolicy,
        NumericExpr, PartialExit, PriceField, ProtectiveStops, Side, StrategyMeta, TrailingPolicy,
    };

    #[test]
    fn m1_chronology_resolves_a_coarse_collision() {
        let decisions = decision_dataset(97.0);
        let scout = evaluate_strategy(
            &strategy(false),
            &decisions,
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                same_bar_policy: SameBarPolicy::Conservative,
                costs: CostModel::default(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(scout.trades[0].exit_reason, ExitReason::StopLoss);

        let judge = evaluate_strategy_m1(
            &strategy(false),
            &decisions,
            &m1_dataset(false, false),
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(judge.trades[0].exit_reason, ExitReason::TakeProfit);
        assert_eq!(judge.trades[0].exit_timestamp_ms, FIXTURE_BASE_MS + 300_000);
        assert_eq!(judge.telemetry.same_minute_stop_target_collisions, 0);
    }

    #[test]
    fn quote_side_pending_triggers_use_ask_for_long_and_bid_for_short() {
        let quote = QuoteBar {
            timestamp_ms: FIXTURE_BASE_MS,
            bid_open: 99.0,
            bid_high: 101.0,
            bid_low: 98.0,
            bid_close: 100.0,
            ask_open: 100.0,
            ask_high: 102.0,
            ask_low: 99.0,
            ask_close: 101.0,
            tick_count: 10,
        };
        let long_stop = PendingOrder {
            side: PositionSide::Long,
            kind: PendingKind::Stop,
            expiry_decision_index: 1,
            activation_price: 101.5,
            stop_loss: 98.0,
            take_profit: 105.0,
            stop_distance: 2.0,
            volume: 1.0,
        };
        let short_stop = PendingOrder {
            side: PositionSide::Short,
            kind: PendingKind::Stop,
            expiry_decision_index: 1,
            activation_price: 98.5,
            stop_loss: 102.0,
            take_profit: 95.0,
            stop_distance: 2.0,
            volume: 1.0,
        };
        assert_eq!(pending_fill_price(&long_stop, &quote), Some(101.5));
        assert_eq!(pending_fill_price(&short_stop, &quote), Some(98.5));
    }

    #[test]
    fn a_same_minute_collision_remains_conservative_and_visible() {
        let judge = evaluate_strategy_m1(
            &strategy(false),
            &decision_dataset(97.0),
            &m1_dataset(true, false),
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(judge.trades[0].exit_reason, ExitReason::StopLoss);
        assert_eq!(judge.telemetry.same_minute_stop_target_collisions, 1);
    }

    #[test]
    fn short_protective_levels_use_the_recorded_ask_path() {
        let mut decisions = decision_dataset(95.5);
        decisions.bars[1].high = 100.5;
        decisions.bars[2].high = 100.5;
        decisions.bars[2].low = 95.5;
        let judge = evaluate_strategy_m1(
            &strategy(true),
            &decisions,
            &m1_dataset(false, true),
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        // Bid reaches below the 96 target, but the one-point spread keeps Ask
        // above it. A bid-only replay would incorrectly take profit.
        assert_eq!(judge.trades[0].exit_reason, ExitReason::EndOfData);
    }

    #[test]
    fn an_in_bar_m1_gap_is_rejected_when_gaps_disallowed() {
        let mut execution = m1_dataset(false, false);
        execution.bars.remove(7);
        assert!(matches!(
            evaluate_strategy_m1(
                &strategy(false),
                &decision_dataset(97.0),
                &execution,
                &broker(),
                &JudgeConfig {
                    initial_balance: 100.0,
                    costs: CostModel::default(),
                    allow_execution_gaps: false,
                    ..Default::default()
                }
            ),
            Err(JudgeError::M1Gap { .. })
        ));
    }

    #[test]
    fn volume_reconciled_no_tick_minutes_are_accepted_and_audited() {
        let mut decisions = decision_dataset(97.0);
        let mut execution = m1_dataset(false, false);
        execution.bars.remove(7);
        execution.bars.remove(8);
        decisions.bars[1].tick_volume = execution
            .bars
            .iter()
            .filter(|bar| {
                (FIXTURE_BASE_MS + 300_000..FIXTURE_BASE_MS + 600_000).contains(&bar.timestamp_ms)
            })
            .map(|bar| bar.tick_volume)
            .sum();

        let judge = evaluate_strategy_m1(
            &strategy(false),
            &decisions,
            &execution,
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(judge.telemetry.m1_gap_events, 0);
        assert_eq!(judge.telemetry.verified_no_tick_gap_events, 2);
        assert_eq!(judge.telemetry.verified_no_tick_minutes, 2);
    }

    #[test]
    fn judge_books_swap_at_the_broker_midnight_before_exit() {
        // Enter inside [02:00,19:00), hold across broker midnight (Wed triple-swap).
        let base = chrono::DateTime::parse_from_rfc3339("2024-01-03T17:00:00Z")
            .unwrap()
            .timestamp_millis();
        let decisions = dataset(
            (0..8)
                .map(|index| {
                    let mut decision = bar(base + index * 3_600_000, 100.0, 101.0, 99.0, 100.0, 0);
                    decision.tick_volume = 60;
                    decision
                })
                .collect(),
            b"swap-decisions",
        );
        let execution = dataset(
            (0..(8 * 60))
                .map(|index| bar(base + index * 60_000, 100.0, 101.0, 99.0, 100.0, 0))
                .collect(),
            b"swap-m1",
        );
        let mut broker = broker();
        broker.swap_mode = SwapMode::Points;
        broker.swap_long = -2.0;
        let mut managed = strategy(false);
        managed.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 50.0 };
        let result = evaluate_strategy_m1(
            &managed,
            &decisions,
            &execution,
            &broker,
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(result.trades[0].swap < 0.0);
        assert_eq!(result.telemetry.swap_rollover_events, 1);
        assert_eq!(result.telemetry.swap_effective_days, 3);
    }

    #[test]
    fn judge_replays_pending_fill_and_completed_bar_management_together() {
        let decisions = dataset(
            vec![
                bar(0, 100.0, 101.0, 99.0, 100.0, 0),
                bar(300_000, 100.0, 103.0, 97.0, 102.0, 0),
                bar(600_000, 104.0, 104.0, 101.0, 102.0, 0),
            ],
            b"managed-decisions",
        );
        let mut minutes = Vec::new();
        for index in 0..5 {
            minutes.push(bar(index * 60_000, 100.0, 101.0, 99.0, 100.0, 0));
        }
        minutes.push(bar(300_000, 100.0, 103.0, 97.0, 102.0, 0));
        for index in 6..10 {
            minutes.push(bar(index * 60_000, 102.0, 102.0, 102.0, 102.0, 0));
        }
        minutes.push(bar(600_000, 104.0, 104.0, 101.0, 102.0, 0));
        for index in 11..15 {
            minutes.push(bar(index * 60_000, 102.0, 102.0, 102.0, 102.0, 0));
        }
        let execution = dataset(minutes, b"managed-m1");
        let mut managed = strategy(false);
        managed.entry.order = EntryOrderPolicy::Limit {
            distance: EntryDistancePolicy::FixedPoints { points: 2.0 },
            expiry_bars: 2,
        };
        managed.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        managed.manage.break_even_at_r = Some(1.0);
        managed.manage.trailing = Some(TrailingPolicy::RiskMultiple {
            activate_at_r: 1.5,
            distance_r: 0.5,
        });
        managed.manage.partial_exits = vec![PartialExit {
            at_r: 1.0,
            fraction: 0.4,
        }];
        let result = evaluate_strategy_m1(
            &managed,
            &decisions,
            &execution,
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.telemetry.pending_orders_placed, 1);
        assert_eq!(result.telemetry.pending_orders_filled, 1);
        assert_eq!(result.telemetry.break_even_moves, 1);
        assert_eq!(result.telemetry.trailing_stop_moves, 1);
        assert_eq!(result.telemetry.partial_exits_executed, 1);
        assert_eq!(result.trades[0].entry_price, 98.0);
        // Fill-aware peak uses post-entry M1 (102), not the pre-fill H1 high (103).
        assert_eq!(result.trades[0].gross_profit, 21.0);
        assert_eq!(result.trades[0].exit_reason, ExitReason::StopLoss);
    }

    #[test]
    fn judge_ignores_pre_entry_m1_high_for_break_even() {
        // Decision OHLC must match M1 aggregates. Pre-fill spike to 110 must not arm BE.
        let decisions = dataset(
            vec![
                bar(0, 100.0, 101.0, 99.0, 100.0, 0),
                bar(300_000, 100.0, 110.0, 97.0, 98.2, 0),
                bar(600_000, 98.2, 98.5, 97.0, 98.0, 0),
            ],
            b"pre-entry-be-decisions",
        );
        let mut minutes = Vec::new();
        for index in 0..5 {
            minutes.push(bar(index * 60_000, 100.0, 101.0, 99.0, 100.0, 0));
        }
        minutes.push(bar(300_000, 100.0, 110.0, 100.0, 109.0, 0));
        minutes.push(bar(360_000, 98.0, 98.5, 97.0, 98.0, 0));
        for index in 7..10 {
            minutes.push(bar(index * 60_000, 98.0, 98.5, 98.0, 98.2, 0));
        }
        for index in 10..15 {
            minutes.push(bar(index * 60_000, 98.2, 98.5, 97.0, 98.0, 0));
        }
        let execution = dataset(minutes, b"pre-entry-be-m1");
        let mut managed = strategy(false);
        managed.entry.order = EntryOrderPolicy::Limit {
            distance: EntryDistancePolicy::FixedPoints { points: 2.0 },
            expiry_bars: 2,
        };
        managed.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        managed.manage.break_even_at_r = Some(1.0);
        let result = evaluate_strategy_m1(
            &managed,
            &decisions,
            &execution,
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.telemetry.pending_orders_filled, 1);
        assert_eq!(result.telemetry.break_even_moves, 0);
    }

    #[test]
    fn judge_flattens_at_22_and_blocks_the_remaining_broker_day() {
        // Enter inside the session window, then flatten at 22:00.
        let start = chrono::DateTime::parse_from_rfc3339("2024-01-01T17:00:00Z")
            .unwrap()
            .timestamp_millis();
        let mut decision_bars = Vec::new();
        for hour in [0, 1, 5, 6] {
            let mut decision = bar(start + hour * 3_600_000, 100.0, 100.0, 100.0, 100.0, 0);
            decision.tick_volume = 60;
            decision_bars.push(decision);
        }
        let decisions = dataset(decision_bars, b"close-at-22-decisions");
        let flatten_ms = chrono::DateTime::parse_from_rfc3339("2024-01-01T23:00:00Z")
            .unwrap()
            .timestamp_millis();
        let end_ms = chrono::DateTime::parse_from_rfc3339("2024-01-02T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let execution = dataset(
            (0..((end_ms - start) / 60_000 + 60))
                .map(|minute| bar(start + minute * 60_000, 100.0, 100.0, 100.0, 100.0, 0))
                .collect(),
            b"close-at-22-m1",
        );
        let mut managed = strategy(false);
        managed.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        managed.manage.flatten_end_of_day = true;
        managed.manage.end_of_day_hour = 23;
        let result = evaluate_strategy_m1(
            &managed,
            &decisions,
            &execution,
            &broker(),
            &JudgeConfig {
                initial_balance: 100.0,
                costs: CostModel::default(),
                allow_execution_gaps: false,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].exit_reason, ExitReason::EndOfDay);
        assert_eq!(result.trades[0].exit_timestamp_ms, flatten_ms);
        assert_eq!(result.telemetry.end_of_day_flattens, 1);
    }

    fn strategy(short: bool) -> StrategyIr {
        let signal = BoolExpr::Compare {
            comparison: ComparisonOp::GreaterThan,
            left: NumericExpr::Price {
                field: PriceField::Close,
                shift: 1,
            },
            right: NumericExpr::Constant { value: 0.0 },
        };
        StrategyIr {
            id: if short { "short" } else { "long" }.into(),
            version: STRATEGY_IR_VERSION,
            entry: EntrySignals {
                long: (!short).then_some(signal.clone()),
                short: short.then_some(signal),
                order: Default::default(),
            },
            exit: None,
            exit_long: None,
            exit_short: None,
            filters: vec![],
            side: if short {
                Side::ShortOnly
            } else {
                Side::LongOnly
            },
            risk: RiskPolicy::FixedCurrency { amount: 10.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::FixedPoints { points: 2.0 },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
            },
            manage: ManagePolicy::default(),
            meta: StrategyMeta {
                thesis_hint: "judge test".into(),
                complexity: 1,
                export_safe: true,
            },
        }
    }

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Fixture".into(),
            symbol: "TEST".into(),
            digits: 2,
            point: 1.0,
            tick_size: 1.0,
            tick_value: 1.0,
            contract_size: 1.0,
            volume_min: 1.0,
            volume_step: 1.0,
            volume_max: 100.0,
            stops_level_points: 0,
            freeze_level_points: 0,
            filling_modes: vec![FillingMode::FillOrKill],
            trade_mode: TradeMode::Full,
            margin_initial_per_lot: None,
            swap_mode: SwapMode::Disabled,
            swap_long: 0.0,
            swap_short: 0.0,
            triple_swap_day: DayOfWeek::Wednesday,
            swap_multipliers: vec![],
            sessions: vec![],
            timezone: "Etc/UTC".into(),
            account_currency: "USD".into(),
            base_currency: "USD".into(),
            profit_currency: "USD".into(),
            margin_currency: "USD".into(),
            synthetic_spreads: vec![],
        }
    }

    fn decision_dataset(low: f64) -> BarDataset {
        dataset(
            vec![
                bar(0, 100.0, 101.0, 99.0, 100.0, 0),
                bar(300_000, 100.0, 105.0, low, 100.0, 0),
                bar(600_000, 100.0, 101.0, 99.0, 100.0, 0),
            ],
            b"decision",
        )
    }

    fn m1_dataset(collision: bool, short_ask_test: bool) -> BarDataset {
        let mut bars: Vec<Bar> = (0..15)
            .map(|index| bar(index * 60_000, 100.0, 101.0, 99.0, 100.0, 0))
            .collect();
        if collision {
            bars[5] = bar(300_000, 100.0, 105.0, 97.0, 100.0, 0);
        } else if short_ask_test {
            for value in bars.iter_mut().skip(5) {
                value.high = 100.5;
                value.low = 95.5;
                value.close = 100.0;
                value.spread_points = Some(1);
            }
        } else {
            bars[5] = bar(300_000, 100.0, 105.0, 99.0, 104.0, 0);
            bars[6] = bar(360_000, 104.0, 105.0, 97.0, 100.0, 0);
        }
        dataset(bars, b"m1")
    }

    /// Place synthetic relative bars at 10:00 UTC so they sit inside `[02:00, 19:00)`.
    /// Absolute RFC3339 fixtures (ms since epoch ≥ 1 day) are left unchanged.
    const FIXTURE_BASE_MS: i64 = 10 * 3_600_000;

    fn bar(
        timestamp_ms: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        spread_points: u32,
    ) -> Bar {
        let timestamp_ms = if timestamp_ms < 86_400_000 {
            FIXTURE_BASE_MS + timestamp_ms
        } else {
            timestamp_ms
        };
        Bar {
            timestamp_ms,
            open,
            high,
            low,
            close,
            tick_volume: 1,
            real_volume: 0,
            spread_points: Some(spread_points),
        }
    }

    fn dataset(bars: Vec<Bar>, identity: &[u8]) -> BarDataset {
        BarDataset {
            source_rows: bars.len(),
            bars,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: ',',
            source_timezone: "Etc/UTC".into(),
            data_hash: ContentHash::sha256(identity),
        }
    }
}
