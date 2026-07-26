//! Completed-bar position management helpers shared by Scout and M1 Judge.
//!
//! Trail / break-even / partials must not use pre-entry extremes (lookahead).
//! On the entry bar/minute only the close is eligible; later bars use high/low.

use crate::model::PositionSide;
use quantforge_broker::SymbolSpecification;
use quantforge_data::Bar;

/// Round a price to the broker digit grid (MT5 `NormalizeDouble(_, digits)`).
pub fn normalize_price(price: f64, broker: &SymbolSpecification) -> f64 {
    if !price.is_finite() {
        return price;
    }
    let scale = 10f64.powi(i32::from(broker.digits));
    if !scale.is_finite() || scale == 0.0 {
        return price;
    }
    (price * scale).round() / scale
}

/// Tick-safe comparison: `price` has reached `level` from below (long TP / short SL).
pub fn price_reaches_from_below(price: f64, level: f64, broker: &SymbolSpecification) -> bool {
    normalize_price(price, broker) + 1.0e-12 >= normalize_price(level, broker)
}

/// Tick-safe comparison: `price` has reached `level` from above (long SL / short TP).
pub fn price_reaches_from_above(price: f64, level: f64, broker: &SymbolSpecification) -> bool {
    normalize_price(price, broker) <= normalize_price(level, broker) + 1.0e-12
}

/// Favorable price sample from one completed decision bar (Scout / OHLC path).
///
/// - Bars before entry are ignored.
/// - The entry bar contributes only its **close** (high/low path vs fill is unknown).
/// - Later bars contribute high (long) or low+spread (short).
pub fn favorable_sample_from_decision_bar(
    side: PositionSide,
    completed_bar: &Bar,
    completed_spread_price: f64,
    entry_index: usize,
    completed_index: usize,
) -> Option<f64> {
    if completed_index < entry_index {
        return None;
    }
    let price = if completed_index == entry_index {
        completed_bar.close
    } else {
        match side {
            PositionSide::Long => completed_bar.high,
            PositionSide::Short => completed_bar.low,
        }
    };
    Some(match side {
        PositionSide::Long => price,
        PositionSide::Short => price + completed_spread_price,
    })
}

/// Favorable price sample from M1 bars covering one completed decision window.
///
/// Minutes before entry are ignored. The entry minute contributes only its close;
/// later minutes contribute high/low. `completed_spread_price` is applied for shorts.
pub fn favorable_sample_from_m1_window(
    side: PositionSide,
    execution_bars: &[Bar],
    entry_timestamp_ms: i64,
    completed_spread_price: f64,
) -> Option<f64> {
    let mut best: Option<f64> = None;
    for bar in execution_bars {
        if bar.timestamp_ms < entry_timestamp_ms {
            continue;
        }
        let raw = if bar.timestamp_ms == entry_timestamp_ms {
            bar.close
        } else {
            match side {
                PositionSide::Long => bar.high,
                PositionSide::Short => bar.low,
            }
        };
        let sample = match side {
            PositionSide::Long => raw,
            PositionSide::Short => raw + completed_spread_price,
        };
        best = Some(match (side, best) {
            (PositionSide::Long, Some(current)) => current.max(sample),
            (PositionSide::Short, Some(current)) => current.min(sample),
            (_, None) => sample,
        });
    }
    best
}

/// Ratchet a peak favorable price (max for long, min for short).
pub fn ratchet_favorable_peak(
    side: PositionSide,
    peak: Option<f64>,
    sample: f64,
) -> Option<f64> {
    if !sample.is_finite() {
        return peak;
    }
    Some(match (side, peak) {
        (PositionSide::Long, Some(current)) => current.max(sample),
        (PositionSide::Short, Some(current)) => current.min(sample),
        (_, None) => sample,
    })
}

pub fn favorable_r(side: PositionSide, favorable_price: f64, entry_price: f64, risk: f64) -> f64 {
    if risk <= 0.0 || !risk.is_finite() {
        return 0.0;
    }
    match side {
        PositionSide::Long => (favorable_price - entry_price) / risk,
        PositionSide::Short => (entry_price - favorable_price) / risk,
    }
}

/// True when a stop at `candidate` would already be triggered at the new bar open.
///
/// Matches MT5: `PositionModify` rejects stops that are marketable at the current
/// bid/ask, so Rust must not invent an immediate gap-exit from such a clamp.
pub fn stop_would_trigger_at_open(
    side: PositionSide,
    candidate: f64,
    bar_open: f64,
    bar_spread: f64,
) -> bool {
    if !candidate.is_finite() {
        return true;
    }
    match side {
        PositionSide::Long => bar_open <= candidate + 1.0e-12,
        PositionSide::Short => bar_open + bar_spread >= candidate - 1.0e-12,
    }
}

/// Clamp a stop candidate to the broker stops-level, or `None` if it cannot be
/// placed without already triggering at the bar open (MT5 modify reject).
///
/// - Long: `min(raw, open - stops_level)`; reject if still marketable.
/// - Short: `max(raw, open + spread + stops_level)`; reject if still marketable.
pub fn placeable_stop_candidate(
    side: PositionSide,
    raw: f64,
    bar_open: f64,
    bar_spread: f64,
    minimum_distance: f64,
) -> Option<f64> {
    if !raw.is_finite() || !bar_open.is_finite() || !bar_spread.is_finite() {
        return None;
    }
    let minimum_distance = minimum_distance.max(0.0);
    let candidate = match side {
        PositionSide::Long => raw.min(bar_open - minimum_distance),
        PositionSide::Short => raw.max(bar_open + bar_spread + minimum_distance),
    };
    if stop_would_trigger_at_open(side, candidate, bar_open, bar_spread) {
        None
    } else {
        Some(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::Bar;

    fn bar(ts: i64, open: f64, high: f64, low: f64, close: f64) -> Bar {
        Bar {
            timestamp_ms: ts,
            open,
            high,
            low,
            close,
            tick_volume: 1,
            real_volume: 0,
            spread_points: None,
        }
    }

    #[test]
    fn entry_bar_uses_close_not_high() {
        let completed = bar(60_000, 100.0, 110.0, 99.0, 101.0);
        let sample = favorable_sample_from_decision_bar(
            PositionSide::Long,
            &completed,
            0.0,
            1,
            1,
        )
        .unwrap();
        assert_eq!(sample, 101.0);
    }

    #[test]
    fn later_bar_uses_high() {
        let completed = bar(120_000, 100.0, 110.0, 99.0, 101.0);
        let sample = favorable_sample_from_decision_bar(
            PositionSide::Long,
            &completed,
            0.0,
            1,
            2,
        )
        .unwrap();
        assert_eq!(sample, 110.0);
    }

    #[test]
    fn m1_ignores_pre_entry_high() {
        let minutes = vec![
            bar(300_000, 100.0, 110.0, 100.0, 109.0), // before fill
            bar(360_000, 98.0, 99.0, 97.0, 98.0),     // fill minute → close only
            bar(420_000, 98.0, 102.0, 98.0, 101.0),   // after → high
        ];
        let sample =
            favorable_sample_from_m1_window(PositionSide::Long, &minutes, 360_000, 0.0).unwrap();
        // max(close@fill=98, high@after=102) = 102, not 110
        assert_eq!(sample, 102.0);
    }

    #[test]
    fn marketable_stop_at_open_is_rejected_when_stops_level_is_zero() {
        // Trail wants 104, open is 103 → clamp to 103 == open → reject (MT5).
        assert_eq!(
            placeable_stop_candidate(PositionSide::Long, 104.0, 103.0, 0.0, 0.0),
            None
        );
        // Raw trail 102 with open 104 → placeable.
        assert_eq!(
            placeable_stop_candidate(PositionSide::Long, 102.0, 104.0, 0.0, 0.0),
            Some(102.0)
        );
        // Short: raw below ask clamps to ask → reject.
        assert_eq!(
            placeable_stop_candidate(PositionSide::Short, 99.0, 100.0, 0.1, 0.0),
            None
        );
        assert_eq!(
            placeable_stop_candidate(PositionSide::Short, 101.0, 100.0, 0.1, 0.0),
            Some(101.0)
        );
    }
}
