use quantforge_data::Bar;
use quantforge_sqx::{
    atr_percentile_series, liquidity_sweep_score_series, rate_of_change_series, rsi_series,
    session_range_series, swing_base_zone_series, zscore_series,
};
use quantforge_broker::BrokerClock;

fn bar(ts: i64, o: f64, h: f64, l: f64, c: f64) -> Bar {
    Bar {
        timestamp_ms: ts,
        open: o,
        high: h,
        low: l,
        close: c,
        tick_volume: 0,
        real_volume: 0,
        spread_points: None,
    }
}

#[test]
fn sqx_rsi_matches_java_seed_and_wilder() {
    let values: Vec<f64> = (0..30).map(|index| 100.0 + index as f64).collect();
    let rsi = rsi_series(&values, 14);
    assert!(rsi[14] > 99.0);
    assert!(rsi[29] > 99.0);
    let flat = vec![1.0; 20];
    let flat_rsi = rsi_series(&flat, 14);
    assert_eq!(flat_rsi[14], 50.0);
}

#[test]
fn sqx_roc_is_percent_change() {
    let values = vec![100.0, 110.0, 99.0];
    let roc = rate_of_change_series(&values, 1);
    assert!((roc[2] - (-10.0)).abs() < 1e-9);
}

#[test]
fn sqx_zscore_uses_population_stdev() {
    let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let z = zscore_series(&values, 5);
    assert!((z[4] - 2.0_f64.sqrt()).abs() < 1e-9);
    assert_eq!(z[2], 0.0);
}

#[test]
fn sqx_atr_percentile_is_bounded() {
    let bars: Vec<Bar> = (0..40)
        .map(|index| bar(index * 3_600_000, 1.0, 1.0 + index as f64 * 0.01, 1.0, 1.0))
        .collect();
    let series = atr_percentile_series(&bars, 5, 20);
    assert!(series[39] >= 0.0 && series[39] <= 100.0);
}

#[test]
fn sqx_liquidity_sweep_emits_discrete_scores() {
    let bars: Vec<Bar> = (0..30)
        .map(|index| {
            let swing = if index % 7 == 0 { 2.0 } else { 0.2 };
            bar(index * 3_600_000, 1.0, 1.0 + swing, 1.0 - swing * 0.2, 1.0)
        })
        .collect();
    let scores = liquidity_sweep_score_series(&bars, 5);
    assert!(scores.iter().any(|value| *value == 1.0 || *value == -1.0 || *value == 0.0));
}

#[test]
fn sqx_session_range_carries_forward() {
    let clock = BrokerClock::parse("Etc/UTC").unwrap();
    let bars: Vec<Bar> = (0..48)
        .map(|index| bar(index * 3_600_000, 1.0, 1.0 + (index as f64 * 0.01), 0.9, 1.0))
        .collect();
    let highs = session_range_series(&bars, &clock, 0, 4, true);
    let lows = session_range_series(&bars, &clock, 0, 4, false);
    assert!(highs[10].is_finite());
    assert!(lows[10].is_finite());
    assert!(highs[10] >= lows[10]);
}

#[test]
fn sqx_swing_zone_high_is_at_least_low() {
    let bars: Vec<Bar> = (0..40)
        .map(|index| {
            let wiggle = ((index % 9) as f64 - 4.0).abs();
            bar(index * 3_600_000, 1.0, 1.2 + wiggle * 0.02, 0.8 - wiggle * 0.02, 1.0)
        })
        .collect();
    let highs = swing_base_zone_series(&bars, 2, 2, 3, true);
    let lows = swing_base_zone_series(&bars, 2, 2, 3, false);
    for index in 20..40 {
        if highs[index].is_finite() && lows[index].is_finite() {
            assert!(highs[index] >= lows[index]);
        }
    }
}

#[test]
fn sqx_swing_zone_forms_when_base_exceeds_swing_right() {
    // Short-side reclaim genes often use base_bars > swing_right. The zone must
    // still become finite once the post-pivot base is complete.
    let mut bars = Vec::new();
    for index in 0..30 {
        let (high, low) = if index == 10 {
            (3.0, 0.5)
        } else if (11..=13).contains(&index) {
            (1.5, 0.8)
        } else {
            (1.2, 1.0)
        };
        bars.push(bar(index * 3_600_000, 1.1, high, low, 1.1));
    }
    let lows = swing_base_zone_series(&bars, 2, 2, 3, false);
    assert!(
        lows[13..].iter().any(|value| value.is_finite()),
        "zone low must form when base_bars(3) > swing_right(2)"
    );
}
