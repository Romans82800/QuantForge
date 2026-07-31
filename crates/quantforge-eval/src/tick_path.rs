//! Synthetic EveryTick path generation from OHLC bars.
//!
//! True tick-file replay is a multi-week effort. This module ships the
//! foundation MT5/SQX use when real ticks are absent: a deterministic
//! Open → extreme1 → extreme2 → Close walk used to resolve same-bar
//! stop/target collisions under [`crate::SameBarPolicy::EveryTickOhlc`].

use quantforge_data::Bar;

use crate::PositionSide;

/// One synthetic bid tick on an OHLC-derived EveryTick path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyntheticTick {
    /// Fraction through the bar `[0, 1]` (ordering only; not wall-clock).
    pub path_fraction: f64,
    pub bid: f64,
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
}
