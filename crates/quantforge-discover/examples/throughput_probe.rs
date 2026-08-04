//! Times candidate evaluation with and without the shared indicator cache.
//!
//! Run with: `cargo run --release -p quantforge-discover --example throughput_probe`

use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, SymbolSpecification, TradeMode};
use quantforge_core::ContentHash;
use quantforge_data::{Bar, BarDataset};
use quantforge_discover::generate_seed;
use quantforge_eval::{
    IndicatorBufferCache, ScoutConfig, evaluate_strategy, evaluate_strategy_cached,
};
use std::time::Instant;

fn bars(count: usize) -> Vec<Bar> {
    (0..count)
        .map(|index| {
            let drift = index as f64 * 0.004;
            let base = 1.1000
                + drift
                + (index as f64 * 0.017).sin() * 0.02
                + (index as f64 * 0.0031).cos() * 0.05;
            let span = 0.0008 + ((index % 11) as f64) * 0.00012;
            Bar {
                // 09:00 UTC start, hourly.
                timestamp_ms: 1_704_000_000_000 + index as i64 * 3_600_000,
                open: base,
                high: base + span,
                low: base - span,
                close: base + span * if index % 2 == 0 { 0.4 } else { -0.3 },
                tick_volume: 500 + (index % 97) as u64,
                real_volume: 0,
                spread_points: Some(8),
            }
        })
        .collect()
}

fn broker() -> SymbolSpecification {
    SymbolSpecification {
        profile_name: "Probe".into(),
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
        swap_mode: SwapMode::Disabled,
        swap_long: 0.0,
        swap_short: 0.0,
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

fn main() {
    let bar_count = 40_000;
    let candidate_count = 400;
    let dataset = BarDataset {
        data_hash: ContentHash::sha256(b"throughput-probe"),
        source_rows: bar_count,
        bars: bars(bar_count),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: '\t',
        source_timezone: "Etc/UTC".into(),
    };
    let broker = broker();
    let config = ScoutConfig::default();
    let candidates: Vec<_> = (0..candidate_count)
        .map(|index| generate_seed(4242, index as u64))
        .collect();

    // Warm the OS/page cache so the first pass is not penalised.
    let _ = evaluate_strategy(&candidates[0], &dataset, &broker, &config);

    let started = Instant::now();
    let mut uncached_trades = 0usize;
    for strategy in &candidates {
        if let Ok(result) = evaluate_strategy(strategy, &dataset, &broker, &config) {
            uncached_trades += result.metrics.trade_count;
        }
    }
    let uncached = started.elapsed();

    let cache = IndicatorBufferCache::new(dataset.bars.len());
    let started = Instant::now();
    let mut cached_trades = 0usize;
    for strategy in &candidates {
        if let Ok(result) = evaluate_strategy_cached(strategy, &dataset, &broker, &config, &cache) {
            cached_trades += result.metrics.trade_count;
        }
    }
    let cached = started.elapsed();

    println!("bars={bar_count} candidates={candidate_count}");
    println!(
        "no shared cache : {:>8.2?}  ({:.0} evals/hour, {uncached_trades} trades)",
        uncached,
        candidate_count as f64 / uncached.as_secs_f64() * 3600.0
    );
    println!(
        "shared cache    : {:>8.2?}  ({:.0} evals/hour, {cached_trades} trades)",
        cached,
        candidate_count as f64 / cached.as_secs_f64() * 3600.0
    );
    println!(
        "speedup         : {:.2}x   buffers retained: {}",
        uncached.as_secs_f64() / cached.as_secs_f64(),
        cache.len()
    );
    assert_eq!(
        uncached_trades, cached_trades,
        "the cache must not change results"
    );

    probe_indicator_lookup();
}

/// The per-bar indicator lookup, old key versus new. This runs once per indicator
/// read per bar, so it dominates the inner loop.
fn probe_indicator_lookup() {
    use quantforge_ir::{IndicatorExpr, PriceField};
    use std::collections::{BTreeMap, HashMap};

    let expressions: Vec<IndicatorExpr> = (0..8)
        .map(|index| IndicatorExpr::Ema {
            source: PriceField::Close,
            period: 10 + index * 5,
            shift: 1 + (index % 3) as u16,
        })
        .collect();
    let reps = 2_000_000;

    let mut json_keyed: BTreeMap<String, f64> = BTreeMap::new();
    for expression in &expressions {
        json_keyed.insert(serde_json::to_string(expression).unwrap(), 1.0);
    }
    let started = Instant::now();
    let mut sink = 0.0;
    for index in 0..reps {
        let expression = &expressions[index % expressions.len()];
        let key = serde_json::to_string(expression).unwrap();
        sink += json_keyed.get(&key).copied().unwrap_or_default();
    }
    let json = started.elapsed();

    let mut typed: HashMap<IndicatorExpr, f64> = HashMap::new();
    for expression in &expressions {
        typed.insert(expression.buffer_key(), 1.0);
    }
    let started = Instant::now();
    for index in 0..reps {
        let expression = &expressions[index % expressions.len()];
        sink += typed
            .get(&expression.buffer_key())
            .copied()
            .unwrap_or_default();
    }
    let typed_elapsed = started.elapsed();

    println!("\nindicator lookup, {reps} reads (checksum {sink:.0})");
    println!(
        "serialized key  : {:>8.2?}  ({:.0} ns/read)",
        json,
        json.as_nanos() as f64 / reps as f64
    );
    println!(
        "typed key       : {:>8.2?}  ({:.0} ns/read)",
        typed_elapsed,
        typed_elapsed.as_nanos() as f64 / reps as f64
    );
    println!(
        "speedup         : {:.1}x",
        json.as_secs_f64() / typed_elapsed.as_secs_f64()
    );
}
