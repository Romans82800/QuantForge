use crate::{CostModel, EvalError, PositionSide};
use chrono::Datelike;
use quantforge_broker::{BrokerClock, DayOfWeek, SwapMode, SymbolSpecification};
use quantforge_data::Bar;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpreadSource {
    Recorded,
    BrokerWindow,
    ExplicitFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSpread {
    pub points: f64,
    pub source: SpreadSource,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct SwapAccrual {
    pub cash: f64,
    pub rollover_events: usize,
    pub effective_days: u32,
}

pub fn resolve_spread(
    bar: &Bar,
    broker: &SymbolSpecification,
    costs: &CostModel,
) -> Result<ResolvedSpread, EvalError> {
    // MT5 exports often stamp quiet minutes as spread=0. That understates ask-side
    // stops vs the tester; when a fallback is configured, treat 0 as missing.
    if let Some(points) = bar.spread_points {
        if points > 0 || costs.fallback_spread_points.is_none() {
            return Ok(ResolvedSpread {
                points: f64::from(points),
                source: SpreadSource::Recorded,
            });
        }
    }
    if let Some(points) = broker.synthetic_spread_points_at(bar.timestamp_ms)? {
        return Ok(ResolvedSpread {
            points,
            source: SpreadSource::BrokerWindow,
        });
    }
    if let Some(points) = costs.fallback_spread_points {
        return Ok(ResolvedSpread {
            points,
            source: SpreadSource::ExplicitFallback,
        });
    }
    if let Some(points) = bar.spread_points {
        return Ok(ResolvedSpread {
            points: f64::from(points),
            source: SpreadSource::Recorded,
        });
    }
    Err(EvalError::MissingSpread {
        timestamp_ms: bar.timestamp_ms,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn accrue_swap(
    side: PositionSide,
    volume: f64,
    entry_price: f64,
    current_price: f64,
    previous_timestamp_ms: i64,
    current_timestamp_ms: i64,
    broker: &SymbolSpecification,
) -> Result<SwapAccrual, EvalError> {
    if broker.swap_mode == SwapMode::Disabled || current_timestamp_ms <= previous_timestamp_ms {
        return Ok(SwapAccrual::default());
    }
    let clock = BrokerClock::parse(&broker.timezone)
        .map_err(|_| EvalError::InvalidBrokerTimezone(broker.timezone.clone()))?;
    let previous = clock
        .local_datetime(previous_timestamp_ms)
        .map_err(|_| EvalError::InvalidConfig("previous swap timestamp is invalid".into()))?;
    let current = clock
        .local_datetime(current_timestamp_ms)
        .map_err(|_| EvalError::InvalidConfig("current swap timestamp is invalid".into()))?;
    let mut date = previous.date();
    let end_date = current.date();
    let mut rollover_events = 0usize;
    let mut effective_days = 0u32;
    while date < end_date {
        let day = DayOfWeek::from_chrono(date.weekday());
        let multiplier = broker.swap_multiplier(day);
        if multiplier > 0 {
            rollover_events += 1;
            effective_days += u32::from(multiplier);
        }
        date = date
            .succ_opt()
            .ok_or_else(|| EvalError::InvalidConfig("swap date overflow".into()))?;
    }
    if effective_days == 0 {
        return Ok(SwapAccrual::default());
    }

    let rate = match side {
        PositionSide::Long => broker.swap_long,
        PositionSide::Short => broker.swap_short,
    };
    let cash_per_lot_day = match broker.swap_mode {
        SwapMode::Disabled => 0.0,
        SwapMode::Points => rate * broker.point / broker.tick_size * broker.tick_value,
        SwapMode::DepositCurrency => rate,
        SwapMode::ProfitCurrency => {
            rate * currency_to_account(&broker.profit_currency, current_price, broker)?
        }
        SwapMode::SymbolCurrency => {
            rate * currency_to_account(&broker.base_currency, current_price, broker)?
        }
        SwapMode::MarginCurrency => {
            rate * currency_to_account(&broker.margin_currency, current_price, broker)?
        }
        SwapMode::InterestCurrent => annual_interest_cash(rate, current_price, broker),
        SwapMode::InterestOpen => annual_interest_cash(rate, entry_price, broker),
        // MT5 reopen modes close/reopen at a rollover price; cash-equivalent is
        // the configured swap rate treated as points (same formula as Points).
        SwapMode::ReopenCurrent | SwapMode::ReopenBid => {
            rate * broker.point / broker.tick_size * broker.tick_value
        }
    };
    Ok(SwapAccrual {
        cash: cash_per_lot_day * volume * f64::from(effective_days),
        rollover_events,
        effective_days,
    })
}

fn annual_interest_cash(rate_percent: f64, price: f64, broker: &SymbolSpecification) -> f64 {
    let quote_to_account = quote_to_account(broker);
    rate_percent / 100.0 / 360.0 * broker.contract_size * price * quote_to_account
}

fn currency_to_account(
    currency: &str,
    current_price: f64,
    broker: &SymbolSpecification,
) -> Result<f64, EvalError> {
    if currency == broker.account_currency {
        Ok(1.0)
    } else if currency == broker.profit_currency {
        Ok(quote_to_account(broker))
    } else if currency == broker.base_currency {
        Ok(current_price * quote_to_account(broker))
    } else {
        Err(EvalError::UnsupportedBrokerFeature(
            "swap currency conversion outside the symbol pair",
        ))
    }
}

fn quote_to_account(broker: &SymbolSpecification) -> f64 {
    broker.tick_value / (broker.tick_size * broker.contract_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_broker::{DailySwapMultiplier, FillingMode, SyntheticSpreadWindow, TradeMode};

    #[test]
    fn recorded_zero_defers_to_fallback_when_configured() {
        let broker = broker();
        let costs = CostModel {
            fallback_spread_points: Some(8.0),
            ..CostModel::default()
        };
        let bar = bar("2024-01-03T12:00:00Z", Some(0));
        assert_eq!(
            resolve_spread(&bar, &broker, &costs).unwrap(),
            ResolvedSpread {
                points: 8.0,
                source: SpreadSource::ExplicitFallback,
            }
        );
    }

    #[test]
    fn recorded_spread_precedes_window_and_fallback() {
        let mut broker = broker();
        broker.synthetic_spreads = vec![SyntheticSpreadWindow {
            day: DayOfWeek::Wednesday,
            open_minute: 0,
            close_minute: 1440,
            spread_points: 12.0,
        }];
        let costs = CostModel {
            fallback_spread_points: Some(20.0),
            ..CostModel::default()
        };
        let mut bar = bar("2024-01-03T12:00:00Z", Some(8));
        assert_eq!(
            resolve_spread(&bar, &broker, &costs).unwrap(),
            ResolvedSpread {
                points: 8.0,
                source: SpreadSource::Recorded
            }
        );
        bar.spread_points = None;
        assert_eq!(
            resolve_spread(&bar, &broker, &costs).unwrap().source,
            SpreadSource::BrokerWindow
        );
    }

    #[test]
    fn points_swap_applies_the_configured_triple_day() {
        let broker = broker();
        let accrual = accrue_swap(
            PositionSide::Long,
            2.0,
            1.1,
            1.1,
            timestamp("2024-01-03T23:59:00Z"),
            timestamp("2024-01-04T00:00:00Z"),
            &broker,
        )
        .unwrap();
        // -2 points * $1/point * 2 lots * Wednesday triple.
        assert_eq!(accrual.cash, -12.0);
        assert_eq!(accrual.rollover_events, 1);
        assert_eq!(accrual.effective_days, 3);
    }

    #[test]
    fn weekend_gap_does_not_invent_weekend_rollovers() {
        let broker = broker();
        let accrual = accrue_swap(
            PositionSide::Short,
            1.0,
            1.1,
            1.1,
            timestamp("2024-01-05T23:59:00Z"),
            timestamp("2024-01-08T00:00:00Z"),
            &broker,
        )
        .unwrap();
        assert_eq!(accrual.cash, 1.0);
        assert_eq!(accrual.rollover_events, 1);
        assert_eq!(accrual.effective_days, 1);
    }

    #[test]
    fn symbol_currency_swap_is_converted_through_the_bound_tick_value() {
        let mut broker = broker();
        broker.swap_mode = SwapMode::SymbolCurrency;
        broker.swap_long = -1.0;
        let accrual = accrue_swap(
            PositionSide::Long,
            1.0,
            1.1,
            1.1,
            timestamp("2024-01-04T23:59:00Z"),
            timestamp("2024-01-05T00:00:00Z"),
            &broker,
        )
        .unwrap();
        assert!((accrual.cash - -1.1).abs() < 1.0e-12);
    }

    #[test]
    fn exact_daily_multiplier_overrides_the_legacy_triple_day() {
        let mut broker = broker();
        broker.swap_multipliers = [
            DayOfWeek::Monday,
            DayOfWeek::Tuesday,
            DayOfWeek::Wednesday,
            DayOfWeek::Thursday,
            DayOfWeek::Friday,
            DayOfWeek::Saturday,
            DayOfWeek::Sunday,
        ]
        .into_iter()
        .map(|day| DailySwapMultiplier {
            day,
            multiplier: if day.is_weekend() { 0 } else { 1 },
        })
        .collect();
        broker.validate().unwrap();
        let accrual = accrue_swap(
            PositionSide::Long,
            2.0,
            1.1,
            1.1,
            timestamp("2024-01-03T23:59:00Z"),
            timestamp("2024-01-04T00:00:00Z"),
            &broker,
        )
        .unwrap();
        assert_eq!(accrual.cash, -4.0);
        assert_eq!(accrual.effective_days, 1);
    }

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Fixture".into(),
            symbol: "EURUSD".into(),
            digits: 5,
            point: 0.00001,
            tick_size: 0.00001,
            tick_value: 1.0,
            contract_size: 100_000.0,
            volume_min: 0.01,
            volume_step: 0.01,
            volume_max: 100.0,
            stops_level_points: 0,
            freeze_level_points: 0,
            filling_modes: vec![FillingMode::FillOrKill],
            trade_mode: TradeMode::Full,
            margin_initial_per_lot: None,
            swap_mode: SwapMode::Points,
            swap_long: -2.0,
            swap_short: 1.0,
            triple_swap_day: DayOfWeek::Wednesday,
            swap_multipliers: vec![],
            sessions: vec![],
            timezone: "Etc/UTC".into(),
            account_currency: "USD".into(),
            base_currency: "EUR".into(),
            profit_currency: "USD".into(),
            margin_currency: "EUR".into(),
            synthetic_spreads: vec![],
        }
    }

    fn bar(value: &str, spread_points: Option<u32>) -> Bar {
        Bar {
            timestamp_ms: timestamp(value),
            open: 1.1,
            high: 1.1,
            low: 1.1,
            close: 1.1,
            tick_volume: 0,
            real_volume: 0,
            spread_points,
        }
    }

    fn timestamp(value: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_millis()
    }
}
