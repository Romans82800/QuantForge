//! EveryTick path helpers: synthetic OHLC walk and tick-file replay.
//!
//! When real ticks are absent, MT5/SQX fall back to a deterministic
//! Open → extreme1 → extreme2 → Close walk under
//! [`crate::SameBarPolicy::EveryTickOhlc`]. When a [`TickDataset`] is supplied
//! and [`crate::ScoutConfig::enable_tick_file_replay`] is on, protective
//! collisions walk bids/asks inside each bar window instead.

use std::fs;
use std::path::Path;

use quantforge_data::Bar;

use crate::PositionSide;
use crate::model::EvalError;

/// One synthetic bid tick on an OHLC-derived EveryTick path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntheticTick {
    /// Fraction through the bar `[0, 1]` (ordering only; not wall-clock).
    pub path_fraction: f64,
    pub bid: f64,
}

/// One real (or exported) tick used for file-backed EveryTick replay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tick {
    pub timestamp_ms: i64,
    pub bid: f64,
    pub ask: f64,
}

/// Sorted tick series for bar-window replay.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickDataset {
    pub ticks: Vec<Tick>,
}

impl TickDataset {
    pub fn is_empty(&self) -> bool {
        self.ticks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ticks.len()
    }
}

/// Build the classic 4-point EveryTick OHLC path for a bar.
///
/// If the bar closes at or above its open, the path is Open → Low → High → Close.
/// Otherwise Open → High → Low → Close.
pub fn ohlc_everytick_path(bar: &Bar) -> [SyntheticTick; 4] {
    let (first_extreme, second_extreme) = if bar.close >= bar.open {
        (bar.low, bar.high)
    } else {
        (bar.high, bar.low)
    };
    [
        SyntheticTick {
            path_fraction: 0.0,
            bid: bar.open,
        },
        SyntheticTick {
            path_fraction: 1.0 / 3.0,
            bid: first_extreme,
        },
        SyntheticTick {
            path_fraction: 2.0 / 3.0,
            bid: second_extreme,
        },
        SyntheticTick {
            path_fraction: 1.0,
            bid: bar.close,
        },
    ]
}

/// Walk an EveryTick OHLC path and report which protective level is hit first.
///
/// Returns `Some(true)` if the stop is hit first, `Some(false)` if the target
/// is hit first, and `None` if neither level is touched on the path.
pub fn everytick_stop_hit_first(
    side: PositionSide,
    stop: f64,
    target: f64,
    bar: &Bar,
    spread_price: f64,
) -> Option<bool> {
    for tick in ohlc_everytick_path(bar) {
        match side {
            PositionSide::Long => {
                if tick.bid <= stop {
                    return Some(true);
                }
                if tick.bid >= target {
                    return Some(false);
                }
            }
            PositionSide::Short => {
                let ask = tick.bid + spread_price;
                if ask >= stop {
                    return Some(true);
                }
                if ask <= target {
                    return Some(false);
                }
            }
        }
    }
    None
}

/// Ticks with `previous_bar.timestamp_ms < tick.timestamp_ms <= bar.timestamp_ms`.
pub fn ticks_in_bar_window<'a>(
    ticks: &'a TickDataset,
    previous_bar: &Bar,
    bar: &Bar,
) -> &'a [Tick] {
    let start = ticks
        .ticks
        .partition_point(|tick| tick.timestamp_ms <= previous_bar.timestamp_ms);
    let end = ticks
        .ticks
        .partition_point(|tick| tick.timestamp_ms <= bar.timestamp_ms);
    &ticks.ticks[start..end]
}

/// Walk real ticks and report which protective level is hit first.
pub fn tick_file_stop_hit_first(
    side: PositionSide,
    stop: f64,
    target: f64,
    window: &[Tick],
) -> Option<bool> {
    for tick in window {
        match side {
            PositionSide::Long => {
                if tick.bid <= stop {
                    return Some(true);
                }
                if tick.bid >= target {
                    return Some(false);
                }
            }
            PositionSide::Short => {
                if tick.ask >= stop {
                    return Some(true);
                }
                if tick.ask <= target {
                    return Some(false);
                }
            }
        }
    }
    None
}

/// Load a tick CSV: `timestamp_ms,bid,ask` or `timestamp_ms,bid,spread_points`
/// (ask = bid + spread_points * point when a `point` is supplied).
///
/// Header row optional. Lines starting with `#` are ignored.
pub fn load_tick_csv(
    path: &Path,
    point: Option<f64>,
) -> Result<TickDataset, EvalError> {
    let text = fs::read_to_string(path).map_err(|err| {
        EvalError::InvalidConfig(format!("failed to read tick file {}: {err}", path.display()))
    })?;
    parse_tick_csv(&text, point)
}

/// Parse tick CSV text (see [`load_tick_csv`]).
pub fn parse_tick_csv(text: &str, point: Option<f64>) -> Result<TickDataset, EvalError> {
    let mut ticks = Vec::new();
    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if line_no == 0
            && (lower.contains("timestamp") || lower.contains("bid") || lower.contains("time"))
        {
            continue;
        }
        let parts: Vec<&str> = line.split([',', ';', '\t']).map(str::trim).collect();
        if parts.len() < 2 {
            return Err(EvalError::InvalidConfig(format!(
                "tick CSV line {}: expected timestamp,bid[,ask|spread]",
                line_no + 1
            )));
        }
        let timestamp_ms: i64 = parts[0].parse().map_err(|_| {
            EvalError::InvalidConfig(format!(
                "tick CSV line {}: invalid timestamp_ms",
                line_no + 1
            ))
        })?;
        let bid: f64 = parts[1].parse().map_err(|_| {
            EvalError::InvalidConfig(format!("tick CSV line {}: invalid bid", line_no + 1))
        })?;
        let ask = if parts.len() >= 3 {
            let third: f64 = parts[2].parse().map_err(|_| {
                EvalError::InvalidConfig(format!(
                    "tick CSV line {}: invalid ask/spread",
                    line_no + 1
                ))
            })?;
            // Heuristic: values >> bid are spread points when point is known.
            if let Some(point) = point {
                if third > bid * 0.5 && third < bid * 2.0 {
                    third
                } else {
                    bid + third * point
                }
            } else if third >= bid {
                third
            } else {
                bid + third
            }
        } else {
            bid
        };
        if !bid.is_finite() || !ask.is_finite() || ask < bid {
            return Err(EvalError::InvalidConfig(format!(
                "tick CSV line {}: non-finite or inverted bid/ask",
                line_no + 1
            )));
        }
        ticks.push(Tick {
            timestamp_ms,
            bid,
            ask,
        });
    }
    ticks.sort_by_key(|tick| tick.timestamp_ms);
    Ok(TickDataset { ticks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::Bar;

    fn bar(open: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar {
            timestamp_ms: 0,
            open,
            high,
            low,
            close,
            tick_volume: 1,
            real_volume: 1,
            spread_points: None,
        }
    }

    #[test]
    fn bullish_bar_visits_low_before_high() {
        let path = ohlc_everytick_path(&bar(1.0, 1.4, 0.9, 1.2));
        assert_eq!(path[0].bid, 1.0);
        assert_eq!(path[1].bid, 0.9);
        assert_eq!(path[2].bid, 1.4);
        assert_eq!(path[3].bid, 1.2);
    }

    #[test]
    fn bearish_bar_visits_high_before_low() {
        let path = ohlc_everytick_path(&bar(1.2, 1.4, 0.9, 1.0));
        assert_eq!(path[1].bid, 1.4);
        assert_eq!(path[2].bid, 0.9);
    }

    #[test]
    fn everytick_prefers_first_touch_on_path() {
        // Open 1.0, dips to 0.95 (hits stop 0.96), then runs to 1.1 (would hit TP 1.05).
        let hit = everytick_stop_hit_first(
            PositionSide::Long,
            0.96,
            1.05,
            &bar(1.0, 1.1, 0.95, 1.08),
            0.0,
        );
        assert_eq!(hit, Some(true));
    }

    #[test]
    fn parse_tick_csv_bid_ask() {
        let data = parse_tick_csv(
            "timestamp_ms,bid,ask\n1000,1.1000,1.1002\n2000,1.0995,1.0997\n",
            None,
        )
        .unwrap();
        assert_eq!(data.len(), 2);
        assert_eq!(data.ticks[1].bid, 1.0995);
        assert_eq!(data.ticks[1].ask, 1.0997);
    }

    #[test]
    fn tick_file_prefers_first_touch() {
        let window = [
            Tick {
                timestamp_ms: 1,
                bid: 1.0,
                ask: 1.0002,
            },
            Tick {
                timestamp_ms: 2,
                bid: 0.95,
                ask: 0.9502,
            },
            Tick {
                timestamp_ms: 3,
                bid: 1.08,
                ask: 1.0802,
            },
        ];
        let hit = tick_file_stop_hit_first(PositionSide::Long, 0.96, 1.05, &window);
        assert_eq!(hit, Some(true));
    }

    #[test]
    fn ticks_in_bar_window_slices_half_open() {
        let ticks = TickDataset {
            ticks: vec![
                Tick {
                    timestamp_ms: 100,
                    bid: 1.0,
                    ask: 1.0,
                },
                Tick {
                    timestamp_ms: 150,
                    bid: 1.1,
                    ask: 1.1,
                },
                Tick {
                    timestamp_ms: 200,
                    bid: 1.2,
                    ask: 1.2,
                },
            ],
        };
        let prev = Bar {
            timestamp_ms: 100,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            tick_volume: 1,
            real_volume: 0,
            spread_points: None,
        };
        let cur = Bar {
            timestamp_ms: 200,
            ..prev.clone()
        };
        let window = ticks_in_bar_window(&ticks, &prev, &cur);
        assert_eq!(window.len(), 2);
        assert_eq!(window[0].timestamp_ms, 150);
        assert_eq!(window[1].timestamp_ms, 200);
    }
}
