//! Broker and symbol rules that are bound to every promotion-grade run.

use chrono::{Datelike, Duration, NaiveDateTime, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use quantforge_core::{ContentHash, HashError, stable_json_hash};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const IC_MARKETS_EST_PLUS_7: &str = "ICMarkets/EST+7";

/// Converts absolute timestamps into the wall clock used for broker sessions,
/// time-based features and swap rollovers. Most brokers use a normal IANA
/// timezone; IC Markets' New-York-plus-seven server clock is an explicit
/// supported rule because no single IANA zone represents it.
#[derive(Debug, Clone, Copy)]
pub struct BrokerClock {
    timezone: Tz,
    wall_clock_shift_hours: i64,
}

impl BrokerClock {
    pub fn parse(value: &str) -> Result<Self, BrokerSpecError> {
        if value.eq_ignore_ascii_case(IC_MARKETS_EST_PLUS_7) {
            return Ok(Self {
                timezone: chrono_tz::America::New_York,
                wall_clock_shift_hours: 7,
            });
        }
        value
            .parse::<Tz>()
            .map(|timezone| Self {
                timezone,
                wall_clock_shift_hours: 0,
            })
            .map_err(|_| BrokerSpecError::InvalidTimezone(value.into()))
    }

    pub fn local_datetime(self, timestamp_ms: i64) -> Result<NaiveDateTime, BrokerSpecError> {
        let timestamp = Utc
            .timestamp_millis_opt(timestamp_ms)
            .single()
            .ok_or(BrokerSpecError::InvalidTimestamp(timestamp_ms))?;
        timestamp
            .with_timezone(&self.timezone)
            .naive_local()
            .checked_add_signed(Duration::hours(self.wall_clock_shift_hours))
            .ok_or(BrokerSpecError::InvalidTimestamp(timestamp_ms))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayOfWeek {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TradingSession {
    pub day: DayOfWeek,
    /// Minutes since midnight in the broker timezone.
    pub open_minute: u16,
    /// Exclusive close; 1440 represents midnight at the end of the day.
    pub close_minute: u16,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SyntheticSpreadWindow {
    pub day: DayOfWeek,
    /// Minutes since midnight in the broker timezone.
    pub open_minute: u16,
    /// Exclusive close; 1440 represents midnight at the end of the day.
    pub close_minute: u16,
    pub spread_points: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DailySwapMultiplier {
    pub day: DayOfWeek,
    /// MT5 supports 0 (none), 1 (single) and 3 (triple).
    pub multiplier: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillingMode {
    FillOrKill,
    ImmediateOrCancel,
    Return,
    BookOrCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TradeMode {
    Disabled,
    LongOnly,
    ShortOnly,
    CloseOnly,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwapMode {
    Disabled,
    Points,
    SymbolCurrency,
    MarginCurrency,
    DepositCurrency,
    ProfitCurrency,
    InterestCurrent,
    InterestOpen,
    ReopenCurrent,
    ReopenBid,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolSpecification {
    pub profile_name: String,
    pub symbol: String,
    pub digits: u8,
    pub point: f64,
    pub tick_size: f64,
    pub tick_value: f64,
    pub contract_size: f64,
    pub volume_min: f64,
    pub volume_step: f64,
    pub volume_max: f64,
    pub stops_level_points: u32,
    pub freeze_level_points: u32,
    pub filling_modes: Vec<FillingMode>,
    pub trade_mode: TradeMode,
    pub margin_initial_per_lot: Option<f64>,
    pub swap_mode: SwapMode,
    pub swap_long: f64,
    pub swap_short: f64,
    pub triple_swap_day: DayOfWeek,
    /// Exact MT5 per-weekday rollover ratios. When empty, legacy profiles use
    /// one on weekdays, zero on weekends and three on `triple_swap_day`.
    #[serde(default)]
    pub swap_multipliers: Vec<DailySwapMultiplier>,
    pub sessions: Vec<TradingSession>,
    /// IANA timezone, for example `Europe/London`.
    pub timezone: String,
    pub account_currency: String,
    pub base_currency: String,
    pub profit_currency: String,
    pub margin_currency: String,
    /// Used only when bar/tick rows do not carry a recorded spread. Windows
    /// must not overlap for the same broker-local weekday.
    #[serde(default)]
    pub synthetic_spreads: Vec<SyntheticSpreadWindow>,
}

impl SymbolSpecification {
    pub fn validate(&self) -> Result<(), BrokerSpecError> {
        require_text("profile_name", &self.profile_name)?;
        require_text("symbol", &self.symbol)?;
        require_text("timezone", &self.timezone)?;
        require_text("account_currency", &self.account_currency)?;
        require_text("base_currency", &self.base_currency)?;
        require_text("profit_currency", &self.profit_currency)?;
        require_text("margin_currency", &self.margin_currency)?;

        if self.digits > 12 {
            return Err(BrokerSpecError::InvalidField {
                field: "digits",
                reason: "must be at most 12",
            });
        }

        for (field, value) in [
            ("point", self.point),
            ("tick_size", self.tick_size),
            ("tick_value", self.tick_value),
            ("contract_size", self.contract_size),
            ("volume_min", self.volume_min),
            ("volume_step", self.volume_step),
            ("volume_max", self.volume_max),
        ] {
            require_positive(field, value)?;
        }

        for (field, value) in [
            ("swap_long", self.swap_long),
            ("swap_short", self.swap_short),
        ] {
            if !value.is_finite() {
                return Err(BrokerSpecError::InvalidField {
                    field,
                    reason: "must be finite",
                });
            }
        }

        if self.volume_min > self.volume_max {
            return Err(BrokerSpecError::InvalidField {
                field: "volume_min",
                reason: "must not exceed volume_max",
            });
        }
        if self.volume_step > self.volume_max {
            return Err(BrokerSpecError::InvalidField {
                field: "volume_step",
                reason: "must not exceed volume_max",
            });
        }
        if self.filling_modes.is_empty() {
            return Err(BrokerSpecError::InvalidField {
                field: "filling_modes",
                reason: "at least one filling mode is required",
            });
        }
        if let Some(margin) = self.margin_initial_per_lot
            && (!margin.is_finite() || margin < 0.0)
        {
            return Err(BrokerSpecError::InvalidField {
                field: "margin_initial_per_lot",
                reason: "must be finite and non-negative",
            });
        }
        for session in &self.sessions {
            if session.open_minute >= session.close_minute || session.close_minute > 1440 {
                return Err(BrokerSpecError::InvalidSession(session.clone()));
            }
        }
        if !self.swap_multipliers.is_empty() {
            if self.swap_multipliers.len() != 7 {
                return Err(BrokerSpecError::InvalidSwapMultipliers(
                    "must contain exactly one entry for each weekday",
                ));
            }
            for (index, value) in self.swap_multipliers.iter().enumerate() {
                if !matches!(value.multiplier, 0 | 1 | 3) {
                    return Err(BrokerSpecError::InvalidSwapMultipliers(
                        "multipliers must be 0, 1 or 3",
                    ));
                }
                if self
                    .swap_multipliers
                    .iter()
                    .skip(index + 1)
                    .any(|other| other.day == value.day)
                {
                    return Err(BrokerSpecError::InvalidSwapMultipliers(
                        "weekday entries must be unique",
                    ));
                }
            }
        }
        for window in &self.synthetic_spreads {
            if window.open_minute >= window.close_minute || window.close_minute > 1440 {
                return Err(BrokerSpecError::InvalidSpreadWindow(window.clone()));
            }
            if !window.spread_points.is_finite() || window.spread_points < 0.0 {
                return Err(BrokerSpecError::InvalidSpreadWindow(window.clone()));
            }
        }
        for (index, left) in self.synthetic_spreads.iter().enumerate() {
            if self.synthetic_spreads.iter().skip(index + 1).any(|right| {
                left.day == right.day
                    && left.open_minute < right.close_minute
                    && right.open_minute < left.close_minute
            }) {
                return Err(BrokerSpecError::OverlappingSpreadWindows(left.day));
            }
        }
        BrokerClock::parse(&self.timezone)?;

        Ok(())
    }

    pub fn content_hash(&self) -> Result<ContentHash, BrokerSpecError> {
        self.validate()?;
        let mut canonical = self.clone();
        canonical.filling_modes.sort_unstable();
        canonical.filling_modes.dedup();
        canonical.sessions.sort_unstable();
        canonical.sessions.dedup();
        canonical.swap_multipliers.sort_unstable();
        canonical.synthetic_spreads.sort_by(|left, right| {
            left.day
                .cmp(&right.day)
                .then(left.open_minute.cmp(&right.open_minute))
                .then(left.close_minute.cmp(&right.close_minute))
                .then(left.spread_points.total_cmp(&right.spread_points))
        });
        canonical.synthetic_spreads.dedup();
        Ok(stable_json_hash(&canonical)?)
    }

    pub fn is_trading_at(&self, timestamp_ms: i64) -> Result<bool, BrokerSpecError> {
        if self.sessions.is_empty() {
            return Ok(true);
        }
        let (day, minute) = self.local_day_and_minute(timestamp_ms)?;
        Ok(self.sessions.iter().any(|session| {
            session.day == day && minute >= session.open_minute && minute < session.close_minute
        }))
    }

    pub fn synthetic_spread_points_at(
        &self,
        timestamp_ms: i64,
    ) -> Result<Option<f64>, BrokerSpecError> {
        let (day, minute) = self.local_day_and_minute(timestamp_ms)?;
        Ok(self
            .synthetic_spreads
            .iter()
            .find(|window| {
                window.day == day && minute >= window.open_minute && minute < window.close_minute
            })
            .map(|window| window.spread_points))
    }

    pub fn local_day_and_minute(
        &self,
        timestamp_ms: i64,
    ) -> Result<(DayOfWeek, u16), BrokerSpecError> {
        let local = BrokerClock::parse(&self.timezone)?.local_datetime(timestamp_ms)?;
        Ok((
            DayOfWeek::from_chrono(local.weekday()),
            (local.hour() * 60 + local.minute()) as u16,
        ))
    }

    pub fn swap_multiplier(&self, day: DayOfWeek) -> u8 {
        self.swap_multipliers
            .iter()
            .find(|value| value.day == day)
            .map(|value| value.multiplier)
            .unwrap_or_else(|| {
                if day.is_weekend() {
                    0
                } else if day == self.triple_swap_day {
                    3
                } else {
                    1
                }
            })
    }
}

impl DayOfWeek {
    pub fn from_chrono(value: Weekday) -> Self {
        match value {
            Weekday::Mon => Self::Monday,
            Weekday::Tue => Self::Tuesday,
            Weekday::Wed => Self::Wednesday,
            Weekday::Thu => Self::Thursday,
            Weekday::Fri => Self::Friday,
            Weekday::Sat => Self::Saturday,
            Weekday::Sun => Self::Sunday,
        }
    }

    pub fn is_weekend(self) -> bool {
        matches!(self, Self::Saturday | Self::Sunday)
    }
}

fn require_text(field: &'static str, value: &str) -> Result<(), BrokerSpecError> {
    if value.trim().is_empty() {
        Err(BrokerSpecError::InvalidField {
            field,
            reason: "must not be empty",
        })
    } else {
        Ok(())
    }
}

fn require_positive(field: &'static str, value: f64) -> Result<(), BrokerSpecError> {
    if !value.is_finite() || value <= 0.0 {
        Err(BrokerSpecError::InvalidField {
            field,
            reason: "must be finite and greater than zero",
        })
    } else {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum BrokerSpecError {
    #[error("invalid broker field {field}: {reason}")]
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    #[error("invalid trading session: {0:?}")]
    InvalidSession(TradingSession),
    #[error("invalid synthetic spread window: {0:?}")]
    InvalidSpreadWindow(SyntheticSpreadWindow),
    #[error("synthetic spread windows overlap on {0:?}")]
    OverlappingSpreadWindows(DayOfWeek),
    #[error("broker timezone or supported broker clock rule is invalid: {0}")]
    InvalidTimezone(String),
    #[error("timestamp is outside the supported range: {0}")]
    InvalidTimestamp(i64),
    #[error("invalid daily swap multipliers: {0}")]
    InvalidSwapMultipliers(&'static str),
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "Demo Raw".into(),
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
            filling_modes: vec![FillingMode::Return, FillingMode::FillOrKill],
            trade_mode: TradeMode::Full,
            margin_initial_per_lot: None,
            swap_mode: SwapMode::Points,
            swap_long: -6.2,
            swap_short: 2.1,
            triple_swap_day: DayOfWeek::Wednesday,
            swap_multipliers: vec![],
            sessions: vec![TradingSession {
                day: DayOfWeek::Monday,
                open_minute: 0,
                close_minute: 1440,
            }],
            timezone: "Europe/London".into(),
            account_currency: "USD".into(),
            base_currency: "EUR".into(),
            profit_currency: "USD".into(),
            margin_currency: "EUR".into(),
            synthetic_spreads: vec![],
        }
    }

    #[test]
    fn hash_ignores_set_order_for_filling_modes() {
        let mut first = fixture();
        first.synthetic_spreads = vec![
            SyntheticSpreadWindow {
                day: DayOfWeek::Monday,
                open_minute: 0,
                close_minute: 600,
                spread_points: 9.0,
            },
            SyntheticSpreadWindow {
                day: DayOfWeek::Monday,
                open_minute: 600,
                close_minute: 1440,
                spread_points: 7.0,
            },
        ];
        let mut second = fixture();
        second.synthetic_spreads = first.synthetic_spreads.clone();
        second.filling_modes.reverse();
        second.synthetic_spreads.reverse();
        assert_eq!(
            first.content_hash().unwrap(),
            second.content_hash().unwrap()
        );
    }

    #[test]
    fn invalid_volume_range_is_rejected() {
        let mut spec = fixture();
        spec.volume_min = 101.0;
        assert!(spec.validate().is_err());
    }

    #[test]
    fn broker_local_sessions_and_spread_windows_are_resolved() {
        let mut broker = fixture();
        broker.sessions = vec![TradingSession {
            day: DayOfWeek::Monday,
            open_minute: 8 * 60,
            close_minute: 17 * 60,
        }];
        broker.synthetic_spreads = vec![SyntheticSpreadWindow {
            day: DayOfWeek::Monday,
            open_minute: 8 * 60,
            close_minute: 17 * 60,
            spread_points: 7.5,
        }];
        broker.timezone = "Europe/London".into();
        let open = chrono::DateTime::parse_from_rfc3339("2024-07-01T08:00:00Z")
            .unwrap()
            .timestamp_millis();
        let closed = chrono::DateTime::parse_from_rfc3339("2024-07-01T17:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert!(broker.is_trading_at(open).unwrap());
        assert_eq!(broker.synthetic_spread_points_at(open).unwrap(), Some(7.5));
        assert!(!broker.is_trading_at(closed).unwrap());
        assert_eq!(broker.synthetic_spread_points_at(closed).unwrap(), None);
    }

    #[test]
    fn ic_markets_clock_tracks_new_york_dst_plus_seven_hours() {
        let clock = BrokerClock::parse("ICMarkets/EST+7").unwrap();
        let winter = chrono::DateTime::parse_from_rfc3339("2024-01-08T07:00:00Z")
            .unwrap()
            .timestamp_millis();
        let summer = chrono::DateTime::parse_from_rfc3339("2024-07-08T06:00:00Z")
            .unwrap()
            .timestamp_millis();
        assert_eq!(clock.local_datetime(winter).unwrap().hour(), 9);
        assert_eq!(clock.local_datetime(summer).unwrap().hour(), 9);
    }

    #[test]
    fn overlapping_spread_windows_are_rejected() {
        let mut broker = fixture();
        broker.synthetic_spreads = vec![
            SyntheticSpreadWindow {
                day: DayOfWeek::Monday,
                open_minute: 0,
                close_minute: 600,
                spread_points: 8.0,
            },
            SyntheticSpreadWindow {
                day: DayOfWeek::Monday,
                open_minute: 500,
                close_minute: 700,
                spread_points: 9.0,
            },
        ];
        assert!(matches!(
            broker.validate(),
            Err(BrokerSpecError::OverlappingSpreadWindows(DayOfWeek::Monday))
        ));
    }
}
