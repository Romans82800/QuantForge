use crate::features::FeatureCache;
use crate::model::{
    BacktestMetrics, EquityPoint, EvalError, ExitReason, PositionSide, ScoutConfig, ScoutResult,
    ScoutTelemetry, Trade,
};
use crate::{SpreadSource, accrue_swap, resolve_spread};
use chrono::Timelike;
use quantforge_broker::{BrokerClock, SwapMode, SymbolSpecification, TradeMode};
use quantforge_core::FloatPolicy;
use quantforge_data::{Bar, BarDataset};
use quantforge_ir::{
    EntryDistancePolicy, EntryOrderPolicy, IndicatorExpr, IrLimits, RiskPolicy, StopLossPolicy,
    StrategyIr, TakeProfitPolicy, TrailingPolicy,
};

#[derive(Debug)]
struct OpenPosition {
    side: PositionSide,
    entry_index: usize,
    entry_timestamp_ms: i64,
    entry_price: f64,
    initial_volume: f64,
    volume: f64,
    stop_loss: f64,
    take_profit: f64,
    initial_stop_loss: f64,
    initial_take_profit: f64,
    initial_risk_distance: f64,
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
    expiry_index: usize,
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

pub fn evaluate_strategy(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
) -> Result<ScoutResult, EvalError> {
    evaluate_strategy_inner(strategy, dataset, broker, config, None)
}

/// Evaluates a strategy with all earlier bars available as indicator warm-up,
/// while forbidding new entries before `entry_start_timestamp_ms`.
///
/// Challenge folds use this to avoid indicator resets without scoring trades
/// from the pre-fold context. Callers bound the fold end by slicing `dataset`.
pub fn evaluate_strategy_from(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
    entry_start_timestamp_ms: i64,
) -> Result<ScoutResult, EvalError> {
    if dataset
        .bars
        .last()
        .is_none_or(|bar| entry_start_timestamp_ms > bar.timestamp_ms)
    {
        return Err(EvalError::InvalidConfig(
            "entry start must not be after the final bar".into(),
        ));
    }
    evaluate_strategy_inner(
        strategy,
        dataset,
        broker,
        config,
        Some(entry_start_timestamp_ms),
    )
}

fn evaluate_strategy_inner(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
    entry_start_timestamp_ms: Option<i64>,
) -> Result<ScoutResult, EvalError> {
    if dataset.bars.len() < 2 {
        return Err(EvalError::InsufficientBars);
    }
    broker.validate()?;
    config.validate()?;
    strategy.validate_export_safe(IrLimits::default())?;
    if matches!(
        broker.swap_mode,
        SwapMode::ReopenCurrent | SwapMode::ReopenBid
    ) {
        return Err(EvalError::UnsupportedBrokerFeature(
            "reopen-price swap modes",
        ));
    }
    match (broker.trade_mode, strategy.side) {
        (TradeMode::Disabled | TradeMode::CloseOnly, _)
        | (TradeMode::LongOnly, quantforge_ir::Side::ShortOnly | quantforge_ir::Side::Both)
        | (TradeMode::ShortOnly, quantforge_ir::Side::LongOnly | quantforge_ir::Side::Both) => {
            return Err(EvalError::IncompatibleBrokerTradeMode);
        }
        _ => {}
    }

    // Canonicalization makes evaluator input stable before any feature keys are
    // generated and removes parameter noise below the documented float policy.
    let strategy = strategy.canonicalized(FloatPolicy::default())?;
    let bars = &dataset.bars;
    let mut features = FeatureCache::new(bars, &broker.timezone)?;
    let mut balance = config.initial_balance;
    let mut position: Option<OpenPosition> = None;
    let mut pending: Option<PendingOrder> = None;
    let mut trades = Vec::new();
    let mut equity = Vec::with_capacity(bars.len() - 1);
    let mut telemetry = ScoutTelemetry::default();
    let broker_clock = BrokerClock::parse(&broker.timezone)?;

    for (index, bar) in bars.iter().enumerate().skip(1) {
        let spread = resolve_spread(bar, broker, &config.costs)?;
        match spread.source {
            SpreadSource::Recorded => {}
            SpreadSource::BrokerWindow => telemetry.synthetic_spread_bars += 1,
            SpreadSource::ExplicitFallback => telemetry.fallback_spread_bars += 1,
        }
        let spread_price = spread.points * broker.point;
        let previous_bar = &bars[index - 1];
        let previous_spread_price =
            resolve_spread(previous_bar, broker, &config.costs)?.points * broker.point;
        let current_local = broker_clock.local_datetime(bar.timestamp_ms)?;
        let in_close_blackout = current_local.hour() >= 22;
        let mut closed_this_bar = false;
        let mut opened_this_bar = false;

        if strategy.manage.flatten_end_of_day && in_close_blackout {
            // The 22:00 broker-time bar is reserved for flatten/cancellation.
            // The entry gate below keeps the strategy flat through 23:59.
            closed_this_bar = true;
            if pending.take().is_some() {
                telemetry.pending_orders_expired += 1;
            }
            if let Some(open) = position.take() {
                let event = ExitEvent {
                    base_price: market_exit_base(open.side, bar.open, spread_price),
                    reason: ExitReason::EndOfDay,
                };
                close_position(
                    open,
                    event,
                    index,
                    bar.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
                telemetry.end_of_day_flattens += 1;
            }
        }

        if !closed_this_bar && let Some(open) = position.as_mut() {
            let accrual = accrue_swap(
                open.side,
                open.volume,
                open.entry_price,
                bar.open,
                bars[index - 1].timestamp_ms,
                bar.timestamp_ms,
                broker,
            )?;
            open.swap += accrual.cash;
            balance += accrual.cash;
            telemetry.swap_rollover_events += accrual.rollover_events;
            telemetry.swap_effective_days += accrual.effective_days;
        }

        if !closed_this_bar && let Some(open) = position.as_mut() {
            apply_completed_bar_management(
                open,
                &strategy,
                previous_bar,
                previous_spread_price,
                index,
                bar,
                spread_price,
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
                        base_price: market_exit_base(side, bar.open, spread_price),
                        reason: ExitReason::PartialExit,
                    },
                    index,
                    bar.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
                closed_this_bar = true;
            }
        }

        if !closed_this_bar && let Some(open) = position.as_ref() {
            let event = if let Some(event) = protective_gap_exit(open, bar, spread_price) {
                Some(event)
            } else if let Some(exit) = &strategy.exit
                && features.evaluate_bool(exit, index)?
            {
                Some(ExitEvent {
                    base_price: market_exit_base(open.side, bar.open, spread_price),
                    reason: ExitReason::Indicator,
                })
            } else if strategy
                .manage
                .time_stop_bars
                .is_some_and(|limit| index - open.entry_index >= limit as usize)
            {
                Some(ExitEvent {
                    base_price: market_exit_base(open.side, bar.open, spread_price),
                    reason: ExitReason::TimeStop,
                })
            } else {
                protective_intrabar_exit(open, bar, spread_price, config)
            };

            if let Some(event) = event {
                let open = position.take().expect("position was checked above");
                close_position(
                    open,
                    event,
                    index,
                    bar.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
                closed_this_bar = true;
            }
        }

        if pending
            .as_ref()
            .is_some_and(|order| index >= order.expiry_index)
        {
            pending = None;
            telemetry.pending_orders_expired += 1;
        }

        if position.is_none()
            && pending.is_none()
            && !closed_this_bar
            && !(strategy.manage.flatten_end_of_day && in_close_blackout)
            && entry_start_timestamp_ms.is_none_or(|start| bar.timestamp_ms >= start)
        {
            let filters_pass = strategy
                .filters
                .iter()
                .map(|filter| features.evaluate_bool(filter, index))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .all(|value| value);
            if filters_pass {
                let long_signal = strategy
                    .entry
                    .long
                    .as_ref()
                    .map(|entry| features.evaluate_bool(entry, index))
                    .transpose()?
                    .unwrap_or(false);
                let short_signal = strategy
                    .entry
                    .short
                    .as_ref()
                    .map(|entry| features.evaluate_bool(entry, index))
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
                    if !broker.is_trading_at(bar.timestamp_ms)? {
                        telemetry.skipped_outside_session += 1;
                    } else if config
                        .costs
                        .max_spread_points
                        .is_some_and(|maximum| spread.points > maximum)
                    {
                        telemetry.skipped_for_spread += 1;
                    } else {
                        match &strategy.entry.order {
                            EntryOrderPolicy::Market => {
                                if let Some(open) = open_position(
                                    side,
                                    index,
                                    bar,
                                    spread_price,
                                    balance,
                                    &strategy,
                                    broker,
                                    config,
                                    &mut features,
                                    &mut telemetry,
                                )? {
                                    balance -= open.entry_commission;
                                    position = Some(open);
                                    opened_this_bar = true;
                                }
                            }
                            EntryOrderPolicy::Stop { .. } | EntryOrderPolicy::Limit { .. } => {
                                if let Some(order) = place_pending_order(
                                    side,
                                    index,
                                    bar,
                                    spread_price,
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

        if position.is_none()
            && !closed_this_bar
            && let Some(fill_price) = pending
                .as_ref()
                .and_then(|order| pending_fill_price(order, bar, spread_price))
        {
            let order = pending.take().expect("pending order was checked");
            let open = fill_pending_order(
                order,
                index,
                bar.timestamp_ms,
                fill_price,
                &strategy,
                broker,
                config,
            );
            balance -= open.entry_commission;
            position = Some(open);
            telemetry.pending_orders_filled += 1;
            opened_this_bar = true;
        }

        if opened_this_bar {
            let event = position
                .as_ref()
                .and_then(|open| protective_intrabar_exit(open, bar, spread_price, config));
            if let Some(event) = event {
                let open = position.take().expect("position was just opened");
                close_position(
                    open,
                    event,
                    index,
                    bar.timestamp_ms,
                    broker,
                    config,
                    &mut balance,
                    &mut trades,
                );
            }
        }

        let marked_equity = position.as_ref().map_or(balance, |open| {
            liquidation_equity(open, bar, spread_price, balance, broker, config)
        });
        equity.push(EquityPoint {
            timestamp_ms: bar.timestamp_ms,
            balance,
            equity: marked_equity,
        });
    }

    if let Some(open) = position.take() {
        let final_index = bars.len() - 1;
        let bar = &bars[final_index];
        let spread_price = resolve_spread(bar, broker, &config.costs)?.points * broker.point;
        let event = ExitEvent {
            base_price: market_exit_base(open.side, bar.close, spread_price),
            reason: ExitReason::EndOfData,
        };
        close_position(
            open,
            event,
            final_index,
            bar.timestamp_ms,
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
    Ok(ScoutResult {
        trades,
        equity,
        metrics,
        telemetry,
    })
}

#[allow(clippy::too_many_arguments)]
fn open_position(
    side: PositionSide,
    index: usize,
    bar: &Bar,
    spread_price: f64,
    balance: f64,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
    features: &mut FeatureCache<'_>,
    telemetry: &mut ScoutTelemetry,
) -> Result<Option<OpenPosition>, EvalError> {
    let Some(stop_distance) = stop_distance(strategy, index, broker, features)? else {
        return Ok(None);
    };
    let Some(target_distance) = target_distance(strategy, index, broker, features, stop_distance)?
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

    let slippage = config.costs.adverse_slippage_points_per_side * broker.point;
    let intended_entry_price = match side {
        PositionSide::Long => bar.open + spread_price,
        PositionSide::Short => bar.open,
    };
    let entry_price = match side {
        PositionSide::Long => intended_entry_price + slippage,
        PositionSide::Short => intended_entry_price - slippage,
    };
    let (stop_loss, take_profit) = match side {
        PositionSide::Long => (
            intended_entry_price - stop_distance,
            intended_entry_price + target_distance,
        ),
        PositionSide::Short => (
            intended_entry_price + stop_distance,
            intended_entry_price - target_distance,
        ),
    };
    let entry_commission = volume * config.costs.commission_per_lot_round_turn / 2.0;

    Ok(Some(OpenPosition {
        side,
        entry_index: index,
        entry_timestamp_ms: bar.timestamp_ms,
        entry_price,
        initial_volume: volume,
        volume,
        stop_loss,
        take_profit,
        initial_stop_loss: stop_loss,
        initial_take_profit: take_profit,
        initial_risk_distance: stop_distance,
        entry_commission,
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
    index: usize,
    bar: &Bar,
    spread_price: f64,
    balance: f64,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
    features: &mut FeatureCache<'_>,
    telemetry: &mut ScoutTelemetry,
) -> Result<Option<PendingOrder>, EvalError> {
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
    let Some(entry_distance) = entry_distance(distance_policy, index, broker, features)? else {
        return Ok(None);
    };
    let Some(stop_distance) = stop_distance(strategy, index, broker, features)? else {
        return Ok(None);
    };
    let Some(target_distance) = target_distance(strategy, index, broker, features, stop_distance)?
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
        PositionSide::Long => bar.open + spread_price,
        PositionSide::Short => bar.open,
    };
    let activation_price = match (side, kind) {
        (PositionSide::Long, PendingKind::Stop) => reference + entry_distance,
        (PositionSide::Short, PendingKind::Stop) => reference - entry_distance,
        (PositionSide::Long, PendingKind::Limit) => reference - entry_distance,
        (PositionSide::Short, PendingKind::Limit) => reference + entry_distance,
    };
    let (stop_loss, take_profit) = match side {
        PositionSide::Long => (
            activation_price - stop_distance,
            activation_price + target_distance,
        ),
        PositionSide::Short => (
            activation_price + stop_distance,
            activation_price - target_distance,
        ),
    };
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
    let Some(volume) = normalize_volume(
        risk_budget / (price_risk_per_lot + cost_risk_per_lot),
        broker,
    ) else {
        telemetry.skipped_below_minimum_volume += 1;
        return Ok(None);
    };
    Ok(Some(PendingOrder {
        side,
        kind,
        expiry_index: index.saturating_add(expiry_bars as usize),
        activation_price,
        stop_loss,
        take_profit,
        stop_distance,
        volume,
    }))
}

fn entry_distance(
    policy: &EntryDistancePolicy,
    decision_index: usize,
    broker: &SymbolSpecification,
    features: &mut FeatureCache<'_>,
) -> Result<Option<f64>, EvalError> {
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

fn pending_fill_price(order: &PendingOrder, bar: &Bar, spread_price: f64) -> Option<f64> {
    let open = match order.side {
        PositionSide::Long => bar.open + spread_price,
        PositionSide::Short => bar.open,
    };
    let touched = match (order.side, order.kind) {
        (PositionSide::Long, PendingKind::Stop) => {
            bar.high + spread_price >= order.activation_price
        }
        (PositionSide::Short, PendingKind::Stop) => bar.low <= order.activation_price,
        (PositionSide::Long, PendingKind::Limit) => {
            bar.low + spread_price <= order.activation_price
        }
        (PositionSide::Short, PendingKind::Limit) => bar.high >= order.activation_price,
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

fn fill_pending_order(
    order: PendingOrder,
    index: usize,
    timestamp_ms: i64,
    fill_base_price: f64,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
) -> OpenPosition {
    let slippage = config.costs.adverse_slippage_points_per_side * broker.point;
    let entry_price = match order.side {
        PositionSide::Long => fill_base_price + slippage,
        PositionSide::Short => fill_base_price - slippage,
    };
    OpenPosition {
        side: order.side,
        entry_index: index,
        entry_timestamp_ms: timestamp_ms,
        entry_price,
        initial_volume: order.volume,
        volume: order.volume,
        stop_loss: order.stop_loss,
        take_profit: order.take_profit,
        initial_stop_loss: order.stop_loss,
        initial_take_profit: order.take_profit,
        initial_risk_distance: order.stop_distance,
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
) -> Result<Option<f64>, EvalError> {
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
) -> Result<Option<f64>, EvalError> {
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

#[allow(clippy::too_many_arguments)]
fn apply_completed_bar_management(
    position: &mut OpenPosition,
    strategy: &StrategyIr,
    completed_bar: &Bar,
    completed_spread_price: f64,
    decision_index: usize,
    current_bar: &Bar,
    current_spread_price: f64,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
    features: &mut FeatureCache<'_>,
    balance: &mut f64,
    telemetry: &mut ScoutTelemetry,
) -> Result<(), EvalError> {
    let favorable_price = match position.side {
        PositionSide::Long => completed_bar.high,
        PositionSide::Short => completed_bar.low + completed_spread_price,
    };
    let favorable_r = match position.side {
        PositionSide::Long => {
            (favorable_price - position.entry_price) / position.initial_risk_distance
        }
        PositionSide::Short => {
            (position.entry_price - favorable_price) / position.initial_risk_distance
        }
    };
    let minimum_distance = broker.stops_level_points as f64 * broker.point;

    if strategy
        .manage
        .break_even_at_r
        .is_some_and(|activation| favorable_r >= activation)
    {
        let candidate = match position.side {
            PositionSide::Long => position
                .entry_price
                .min(current_bar.open - minimum_distance),
            PositionSide::Short => position
                .entry_price
                .max(current_bar.open + current_spread_price + minimum_distance),
        };
        if tighten_stop(position, candidate) {
            telemetry.break_even_moves += 1;
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
            let candidate = match position.side {
                PositionSide::Long => raw_candidate.min(current_bar.open - minimum_distance),
                PositionSide::Short => {
                    raw_candidate.max(current_bar.open + current_spread_price + minimum_distance)
                }
            };
            if tighten_stop(position, candidate) {
                telemetry.trailing_stop_moves += 1;
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
        let base_price = market_exit_base(position.side, current_bar.open, current_spread_price);
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
    config: &ScoutConfig,
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
    let gross_profit = (exit_price - position.entry_price) * direction / broker.tick_size
        * broker.tick_value
        * volume;
    let exit_commission = volume * config.costs.commission_per_lot_round_turn / 2.0;
    *balance += gross_profit - exit_commission;
    position.realized_gross_profit += gross_profit;
    position.realized_exit_commission += exit_commission;
    position.exited_volume += volume;
    position.weighted_exit_price += exit_price * volume;
    position.volume = (position.volume - volume).max(0.0);
}

fn protective_gap_exit(position: &OpenPosition, bar: &Bar, spread_price: f64) -> Option<ExitEvent> {
    match position.side {
        PositionSide::Long if bar.open <= position.stop_loss => Some(ExitEvent {
            base_price: bar.open,
            reason: ExitReason::StopLoss,
        }),
        PositionSide::Long if bar.open >= position.take_profit => Some(ExitEvent {
            base_price: position.take_profit,
            reason: ExitReason::TakeProfit,
        }),
        PositionSide::Short if bar.open + spread_price >= position.stop_loss => Some(ExitEvent {
            base_price: bar.open + spread_price,
            reason: ExitReason::StopLoss,
        }),
        PositionSide::Short if bar.open + spread_price <= position.take_profit => Some(ExitEvent {
            base_price: position.take_profit,
            reason: ExitReason::TakeProfit,
        }),
        _ => None,
    }
}

fn protective_intrabar_exit(
    position: &OpenPosition,
    bar: &Bar,
    spread_price: f64,
    _config: &ScoutConfig,
) -> Option<ExitEvent> {
    let (stop_touched, target_touched) = match position.side {
        PositionSide::Long => (
            bar.low <= position.stop_loss,
            bar.high >= position.take_profit,
        ),
        PositionSide::Short => (
            bar.high + spread_price >= position.stop_loss,
            bar.low + spread_price <= position.take_profit,
        ),
    };
    if stop_touched {
        // Conservative is the only v1 policy: it also resolves bars touching
        // both levels without fabricating an intrabar path.
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
    exit_index: usize,
    exit_timestamp_ms: i64,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
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
        bars_held: exit_index - position.entry_index,
        exit_reason: event.reason,
    });
}

fn market_exit_base(side: PositionSide, bid_price: f64, spread_price: f64) -> f64 {
    match side {
        PositionSide::Long => bid_price,
        PositionSide::Short => bid_price + spread_price,
    }
}

fn liquidation_equity(
    position: &OpenPosition,
    bar: &Bar,
    spread_price: f64,
    balance: f64,
    broker: &SymbolSpecification,
    config: &ScoutConfig,
) -> f64 {
    let exit_price = market_exit_base(position.side, bar.close, spread_price);
    let direction = match position.side {
        PositionSide::Long => 1.0,
        PositionSide::Short => -1.0,
    };
    let gross = (exit_price - position.entry_price) * direction / broker.tick_size
        * broker.tick_value
        * position.volume;
    let exit_commission = position.volume * config.costs.commission_per_lot_round_turn / 2.0;
    balance + gross - exit_commission
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
    let mut max_drawdown = 0.0;
    let mut max_drawdown_percent = 0.0;
    for point in equity {
        peak = peak.max(point.equity);
        let drawdown = peak - point.equity;
        if drawdown > max_drawdown {
            max_drawdown = drawdown;
        }
        if peak > 0.0 {
            let percentage = drawdown / peak * 100.0;
            if percentage > max_drawdown_percent {
                max_drawdown_percent = percentage;
            }
        }
    }

    let net_profit = ending_balance - initial_balance;
    let sharpe_ratio = equity_sharpe_ratio(initial_balance, equity);
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
    }
}

pub fn equity_sharpe_ratio(initial_balance: f64, equity: &[EquityPoint]) -> Option<f64> {
    if equity.len() < 2 || initial_balance <= 0.0 {
        return None;
    }
    let mut previous = initial_balance;
    let returns: Vec<_> = equity
        .iter()
        .filter_map(|point| {
            let value = (previous > 0.0).then_some(point.equity / previous - 1.0);
            previous = point.equity;
            value.filter(|value| value.is_finite())
        })
        .collect();
    if returns.len() < 2 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    let deviation = variance.sqrt();
    (deviation > 1.0e-12).then_some(mean / deviation * (returns.len() as f64).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_core::{ContentHash, STRATEGY_IR_VERSION};
    use quantforge_ir::{
        BoolExpr, ComparisonOp, EntryDistancePolicy, EntryOrderPolicy, EntrySignals, ManagePolicy,
        NumericExpr, PartialExit, PriceField, ProtectiveStops, Side, StrategyMeta, TrailingPolicy,
    };

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

    #[test]
    fn equity_sharpe_requires_variation_and_rewards_positive_path() {
        let rising = vec![
            EquityPoint {
                timestamp_ms: 1,
                balance: 100.0,
                equity: 101.0,
            },
            EquityPoint {
                timestamp_ms: 2,
                balance: 100.0,
                equity: 103.0,
            },
            EquityPoint {
                timestamp_ms: 3,
                balance: 100.0,
                equity: 104.0,
            },
        ];
        assert!(equity_sharpe_ratio(100.0, &rising).is_some_and(|value| value > 0.0));
        assert_eq!(equity_sharpe_ratio(100.0, &rising[..1]), None);
    }

    fn strategy() -> StrategyIr {
        StrategyIr {
            id: "long-fixture".into(),
            version: STRATEGY_IR_VERSION,
            entry: EntrySignals {
                long: Some(BoolExpr::Compare {
                    comparison: ComparisonOp::GreaterThan,
                    left: NumericExpr::Price {
                        field: PriceField::Close,
                        shift: 1,
                    },
                    right: NumericExpr::Constant { value: 0.0 },
                }),
                short: None,
                order: Default::default(),
            },
            exit: None,
            filters: vec![],
            side: Side::LongOnly,
            risk: RiskPolicy::FixedCurrency { amount: 10.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::FixedPoints { points: 2.0 },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 2.0 },
            },
            manage: ManagePolicy::default(),
            meta: StrategyMeta {
                thesis_hint: "test".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    fn short_strategy() -> StrategyIr {
        let mut strategy = strategy();
        strategy.id = "short-fixture".into();
        strategy.side = Side::ShortOnly;
        strategy.entry = EntrySignals {
            long: None,
            short: strategy.entry.long.take(),
            order: strategy.entry.order.clone(),
        };
        strategy
    }

    fn dataset(low: f64) -> BarDataset {
        let bars = vec![
            Bar {
                timestamp_ms: 0,
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                tick_volume: 1,
                real_volume: 0,
                spread_points: Some(0),
            },
            Bar {
                timestamp_ms: 60_000,
                open: 100.0,
                high: 105.0,
                low,
                close: 104.0,
                tick_volume: 1,
                real_volume: 0,
                spread_points: Some(0),
            },
        ];
        BarDataset {
            data_hash: ContentHash::sha256(b"fixture"),
            bars,
            source_rows: 2,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        }
    }

    fn dataset_with_bars(bars: Vec<Bar>) -> BarDataset {
        BarDataset {
            data_hash: ContentHash::sha256(b"managed-fixture"),
            source_rows: bars.len(),
            bars,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        }
    }

    fn bar(timestamp_ms: i64, open: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar {
            timestamp_ms,
            open,
            high,
            low,
            close,
            tick_volume: 1,
            real_volume: 0,
            spread_points: Some(0),
        }
    }

    #[test]
    fn enters_next_bar_open_and_takes_target() {
        let result = evaluate_strategy(
            &strategy(),
            &dataset(99.0),
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].entry_price, 100.0);
        assert_eq!(result.trades[0].volume, 5.0);
        assert_eq!(result.trades[0].exit_reason, ExitReason::TakeProfit);
        assert_eq!(result.metrics.ending_balance, 120.0);
    }

    #[test]
    fn challenge_window_uses_warmup_bars_without_scoring_their_entries() {
        let mut dataset = dataset(99.0);
        dataset.bars.push(Bar {
            timestamp_ms: 120_000,
            open: 100.0,
            high: 105.0,
            low: 99.0,
            close: 104.0,
            tick_volume: 1,
            real_volume: 0,
            spread_points: Some(0),
        });
        dataset.source_rows = dataset.bars.len();
        let result = evaluate_strategy_from(
            &strategy(),
            &dataset,
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
            120_000,
        )
        .unwrap();

        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].entry_timestamp_ms, 120_000);
    }

    #[test]
    fn conservative_same_bar_ambiguity_chooses_stop() {
        let result = evaluate_strategy(
            &strategy(),
            &dataset(97.0),
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();

        assert_eq!(result.trades[0].exit_reason, ExitReason::StopLoss);
        assert_eq!(result.metrics.ending_balance, 90.0);
    }

    #[test]
    fn break_even_moves_on_a_completed_bar_and_protects_the_next_bar() {
        let mut strategy = strategy();
        strategy.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        strategy.manage.break_even_at_r = Some(1.0);
        let data = dataset_with_bars(vec![
            bar(0, 100.0, 101.0, 99.0, 100.0),
            bar(60_000, 100.0, 103.0, 99.0, 102.0),
            bar(120_000, 100.0, 101.0, 99.0, 100.0),
        ]);
        let result =
            evaluate_strategy(&strategy, &data, &broker(), &ScoutConfig::default()).unwrap();
        assert_eq!(result.trades[0].exit_reason, ExitReason::StopLoss);
        assert_eq!(result.trades[0].exit_price, 100.0);
        assert_eq!(result.telemetry.break_even_moves, 1);
    }

    #[test]
    fn trailing_stop_uses_the_completed_favorable_extreme() {
        let mut strategy = strategy();
        strategy.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        strategy.manage.trailing = Some(TrailingPolicy::RiskMultiple {
            activate_at_r: 1.0,
            distance_r: 0.5,
        });
        let data = dataset_with_bars(vec![
            bar(0, 100.0, 101.0, 99.0, 100.0),
            bar(60_000, 100.0, 104.0, 99.0, 103.0),
            bar(120_000, 104.0, 104.0, 102.0, 103.0),
        ]);
        let result =
            evaluate_strategy(&strategy, &data, &broker(), &ScoutConfig::default()).unwrap();
        assert_eq!(result.trades[0].exit_price, 103.0);
        assert_eq!(result.telemetry.trailing_stop_moves, 1);
    }

    #[test]
    fn partial_exit_realizes_only_the_configured_original_volume_fraction() {
        let mut strategy = strategy();
        strategy.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        strategy.manage.partial_exits = vec![PartialExit {
            at_r: 1.0,
            fraction: 0.4,
        }];
        let data = dataset_with_bars(vec![
            bar(0, 100.0, 101.0, 99.0, 100.0),
            bar(60_000, 100.0, 103.0, 99.0, 102.0),
            bar(120_000, 104.0, 104.0, 99.0, 100.0),
        ]);
        let result = evaluate_strategy(
            &strategy,
            &data,
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();
        assert_eq!(result.telemetry.partial_exits_executed, 1);
        assert_eq!(result.trades[0].volume, 5.0);
        assert_eq!(result.trades[0].gross_profit, 8.0);
        assert_eq!(result.metrics.ending_balance, 108.0);
    }

    #[test]
    fn stop_and_limit_entries_use_persistent_pending_orders() {
        for (order, expected_entry, expected_reason) in [
            (
                EntryOrderPolicy::Stop {
                    distance: EntryDistancePolicy::FixedPoints { points: 2.0 },
                    expiry_bars: 2,
                },
                102.0,
                ExitReason::EndOfData,
            ),
            (
                EntryOrderPolicy::Limit {
                    distance: EntryDistancePolicy::FixedPoints { points: 2.0 },
                    expiry_bars: 2,
                },
                98.0,
                ExitReason::TakeProfit,
            ),
        ] {
            let mut strategy = strategy();
            strategy.entry.order = order;
            let low = if expected_entry > 100.0 { 101.0 } else { 97.0 };
            let data = dataset_with_bars(vec![
                bar(0, 100.0, 101.0, 99.0, 100.0),
                bar(60_000, 100.0, 105.0, low, 104.0),
            ]);
            let result = evaluate_strategy(
                &strategy,
                &data,
                &broker(),
                &ScoutConfig {
                    initial_balance: 100.0,
                    ..ScoutConfig::default()
                },
            )
            .unwrap();
            assert_eq!(result.telemetry.pending_orders_placed, 1);
            assert_eq!(result.telemetry.pending_orders_filled, 1);
            assert_eq!(result.trades[0].entry_price, expected_entry);
            assert_eq!(result.trades[0].exit_reason, expected_reason);
        }
    }

    #[test]
    fn end_of_day_flatten_closes_at_22_and_blocks_later_entries() {
        let mut strategy = strategy();
        strategy.stops.take_profit = TakeProfitPolicy::RiskMultiple { multiple: 10.0 };
        strategy.manage.flatten_end_of_day = true;
        let data = dataset_with_bars(vec![
            timed_bar("2024-01-01T20:00:00Z"),
            timed_bar("2024-01-01T21:00:00Z"),
            timed_bar("2024-01-01T22:00:00Z"),
            timed_bar("2024-01-01T23:00:00Z"),
        ]);
        let result =
            evaluate_strategy(&strategy, &data, &broker(), &ScoutConfig::default()).unwrap();
        assert_eq!(result.trades[0].exit_reason, ExitReason::EndOfDay);
        assert_eq!(result.telemetry.end_of_day_flattens, 1);
        assert_eq!(result.trades.len(), 1);
        assert_eq!(
            result.trades[0].exit_timestamp_ms,
            timed_bar("2024-01-01T22:00:00Z").timestamp_ms
        );
    }

    #[test]
    fn risk_sizing_accounts_for_round_turn_commission() {
        let result = evaluate_strategy(
            &strategy(),
            &dataset(99.0),
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                costs: crate::CostModel {
                    commission_per_lot_round_turn: 2.0,
                    ..crate::CostModel::default()
                },
                ..ScoutConfig::default()
            },
        )
        .unwrap();

        assert_eq!(result.trades[0].volume, 2.0);
        assert_eq!(result.trades[0].commission, 4.0);
        assert_eq!(result.metrics.ending_balance, 104.0);
    }

    #[test]
    fn short_targets_are_triggered_on_ask_prices() {
        let mut dataset = dataset(95.0);
        dataset.bars[1].high = 100.0;
        dataset.bars[1].spread_points = Some(1);
        let result = evaluate_strategy(
            &short_strategy(),
            &dataset,
            &broker(),
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();

        assert_eq!(result.trades[0].entry_price, 100.0);
        assert_eq!(result.trades[0].exit_price, 96.0);
        assert_eq!(result.trades[0].exit_reason, ExitReason::TakeProfit);
        assert_eq!(result.metrics.ending_balance, 120.0);
    }

    #[test]
    fn triple_day_swap_is_booked_into_trade_and_balance() {
        let mut broker = broker();
        broker.swap_mode = SwapMode::Points;
        broker.swap_long = -2.0;
        let dataset = BarDataset {
            bars: vec![
                timed_bar("2024-01-03T22:00:00Z"),
                timed_bar("2024-01-03T23:00:00Z"),
                timed_bar("2024-01-04T00:00:00Z"),
            ],
            source_rows: 3,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: ',',
            source_timezone: "Etc/UTC".into(),
            data_hash: ContentHash::sha256(b"swap-fixture"),
        };
        let result = evaluate_strategy(
            &strategy(),
            &dataset,
            &broker,
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.trades[0].swap, -30.0);
        assert_eq!(result.trades[0].net_profit, -30.0);
        assert_eq!(result.metrics.ending_balance, 70.0);
        assert_eq!(result.telemetry.swap_rollover_events, 1);
        assert_eq!(result.telemetry.swap_effective_days, 3);
    }

    #[test]
    fn entries_outside_the_broker_session_are_skipped() {
        let mut broker = broker();
        broker.sessions = vec![quantforge_broker::TradingSession {
            day: DayOfWeek::Wednesday,
            open_minute: 0,
            close_minute: 60,
        }];
        let dataset = BarDataset {
            bars: vec![
                timed_bar("2024-01-03T21:00:00Z"),
                timed_bar("2024-01-03T22:00:00Z"),
                timed_bar("2024-01-03T23:00:00Z"),
            ],
            source_rows: 3,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: ',',
            source_timezone: "Etc/UTC".into(),
            data_hash: ContentHash::sha256(b"session-fixture"),
        };
        let result = evaluate_strategy(
            &strategy(),
            &dataset,
            &broker,
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();
        assert!(result.trades.is_empty());
        assert_eq!(result.telemetry.skipped_outside_session, 2);
    }

    #[test]
    fn broker_spread_window_replaces_missing_bar_spread() {
        let mut broker = broker();
        broker.synthetic_spreads = vec![quantforge_broker::SyntheticSpreadWindow {
            day: DayOfWeek::Thursday,
            open_minute: 0,
            close_minute: 1440,
            spread_points: 0.0,
        }];
        let mut dataset = dataset(99.0);
        for bar in &mut dataset.bars {
            bar.spread_points = None;
        }
        let result = evaluate_strategy(
            &strategy(),
            &dataset,
            &broker,
            &ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
        )
        .unwrap();
        assert_eq!(result.trades.len(), 1);
        assert_eq!(result.telemetry.synthetic_spread_bars, 1);
        assert_eq!(result.telemetry.fallback_spread_bars, 0);
    }

    fn timed_bar(timestamp: &str) -> Bar {
        Bar {
            timestamp_ms: chrono::DateTime::parse_from_rfc3339(timestamp)
                .unwrap()
                .timestamp_millis(),
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.0,
            tick_volume: 1,
            real_volume: 0,
            spread_points: Some(0),
        }
    }
}
