//! Stationary-bootstrap synthetic series for Discover noise-floor calibration.

use crate::multi_symbol::PackSymbol;
use crate::{DiscoverConfig, evolve_new_with_pack};
use quantforge_broker::SymbolSpecification;
use quantforge_data::{Bar, BarDataset, bar_content_hash, build_timeframe_from_m1};
use quantforge_eval::BacktestMetrics;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermutationNullConfig {
    pub trials: usize,
    pub mean_block_length: usize,
    pub seed: u64,
    pub discover: DiscoverConfig,
    pub generations: u64,
}

impl Default for PermutationNullConfig {
    fn default() -> Self {
        Self {
            trials: 8,
            mean_block_length: 24 * 60, // ~1 trading day of M1
            seed: 7,
            discover: DiscoverConfig {
                initial_candidates: 200,
                batch_size: 100,
                require_m1_precision: false,
                require_m1_robustness: false,
                multi_symbol_minimum_pass: 0,
                minimum_deflated_trade_sharpe: None,
                worker_threads: 1,
                ..DiscoverConfig::default()
            },
            generations: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermutationNullReport {
    pub trials: usize,
    pub seed: u64,
    pub mean_block_length: usize,
    pub best_profit_factor: Vec<f64>,
    pub best_return_drawdown: Vec<f64>,
    pub best_expectancy: Vec<f64>,
    pub best_trade_sharpe: Vec<f64>,
    pub p95_profit_factor: f64,
    pub p95_return_drawdown: f64,
    pub p95_expectancy: f64,
    pub p95_trade_sharpe: f64,
}

/// Stationary bootstrap of M1 log-return path, rebuilding OHLC around the
/// synthetic close path while preserving each bar's spread stamp.
pub fn stationary_bootstrap_bars(
    source: &[Bar],
    mean_block_length: usize,
    seed: u64,
) -> Vec<Bar> {
    assert!(!source.is_empty());
    let mean_block = mean_block_length.max(1);
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let returns: Vec<f64> = source
        .windows(2)
        .map(|pair| {
            let prev = pair[0].close.max(1.0e-12);
            let next = pair[1].close.max(1.0e-12);
            (next / prev).ln()
        })
        .collect();
    if returns.is_empty() {
        return source.to_vec();
    }

    let mut synthetic_returns = Vec::with_capacity(returns.len());
    while synthetic_returns.len() < returns.len() {
        let start = rng.gen_range(0..returns.len());
        let mut length = 1usize;
        while length < mean_block * 4 && rng.r#gen::<f64>() > 1.0 / mean_block as f64 {
            length += 1;
        }
        for offset in 0..length {
            if synthetic_returns.len() >= returns.len() {
                break;
            }
            synthetic_returns.push(returns[(start + offset) % returns.len()]);
        }
    }

    let mut close = source[0].close.max(1.0e-12);
    let mut bars = Vec::with_capacity(source.len());
    bars.push(source[0].clone());
    for (index, ret) in synthetic_returns.iter().enumerate() {
        let prev_close = close;
        close = (prev_close * ret.exp()).max(1.0e-12);
        let template = &source[index + 1];
        let open = prev_close;
        let high = open.max(close) * (1.0 + 0.0001);
        let low = open.min(close) * (1.0 - 0.0001);
        bars.push(Bar {
            timestamp_ms: template.timestamp_ms,
            open,
            high,
            low,
            close,
            tick_volume: template.tick_volume,
            real_volume: template.real_volume,
            spread_points: template.spread_points,
        });
    }
    bars
}

fn synthetic_dataset(source: &BarDataset, mean_block_length: usize, seed: u64) -> BarDataset {
    let bars = stationary_bootstrap_bars(&source.bars, mean_block_length, seed);
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: source.delimiter,
        source_timezone: source.source_timezone.clone(),
        bars,
    }
}

/// Run Discover against stationary-bootstrap synthetic M1 (and rebuilt H1) and
/// publish the best-of-run metric distribution for gate calibration.
pub fn run_permutation_null(
    m1: &BarDataset,
    broker: &SymbolSpecification,
    pack_m1: &[(String, BarDataset, SymbolSpecification)],
    config: &PermutationNullConfig,
) -> Result<PermutationNullReport, crate::DiscoverError> {
    let mut best_profit_factor = Vec::with_capacity(config.trials);
    let mut best_return_drawdown = Vec::with_capacity(config.trials);
    let mut best_expectancy = Vec::with_capacity(config.trials);
    let mut best_trade_sharpe = Vec::with_capacity(config.trials);

    for trial in 0..config.trials {
        let trial_seed = config.seed.wrapping_add(trial as u64 * 1_000_003);
        let synth_m1 = synthetic_dataset(m1, config.mean_block_length, trial_seed);
        let synth_h1 = build_timeframe_from_m1(&synth_m1, 3_600_000, None).map_err(|error| {
            crate::DiscoverError::InvalidConfig(format!("synthetic H1 rebuild failed: {error}"))
        })?;
        let mut pack = Vec::with_capacity(pack_m1.len());
        for (index, (symbol, dataset, market_broker)) in pack_m1.iter().enumerate() {
            let synth = synthetic_dataset(
                dataset,
                config.mean_block_length,
                trial_seed.wrapping_add(17 + index as u64),
            );
            let h1 = build_timeframe_from_m1(&synth, 3_600_000, None).map_err(|error| {
                crate::DiscoverError::InvalidConfig(format!(
                    "synthetic pack H1 rebuild failed: {error}"
                ))
            })?;
            pack.push(PackSymbol {
                symbol: symbol.clone(),
                dataset: h1,
                broker: market_broker.clone(),
            });
        }

        let mut discover = config.discover.clone();
        discover.seed = trial_seed;
        // Noise floor must not pay for M1 robustness — H1 search only.
        discover.require_m1_precision = false;
        discover.require_m1_robustness = false;
        discover.calendar_year_folds = false;

        let bank = evolve_new_with_pack(
            &synth_h1,
            None,
            &synth_m1,
            broker,
            &pack,
            &broker.symbol,
            discover,
            config.generations,
        )?;

        let best = bank
            .accepted_pool
            .iter()
            .chain(bank.elites.iter())
            .map(|elite| &elite.metrics)
            .max_by(|left, right| {
                left.net_profit
                    .partial_cmp(&right.net_profit)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        let (pf, ret_dd, expectancy, sharpe) = match best {
            Some(metrics) => metrics_tuple(metrics),
            None => (0.0, 0.0, 0.0, 0.0),
        };
        best_profit_factor.push(pf);
        best_return_drawdown.push(ret_dd);
        best_expectancy.push(expectancy);
        best_trade_sharpe.push(sharpe);
    }

    Ok(PermutationNullReport {
        trials: config.trials,
        seed: config.seed,
        mean_block_length: config.mean_block_length,
        p95_profit_factor: percentile(&mut best_profit_factor.clone(), 0.95),
        p95_return_drawdown: percentile(&mut best_return_drawdown.clone(), 0.95),
        p95_expectancy: percentile(&mut best_expectancy.clone(), 0.95),
        p95_trade_sharpe: percentile(&mut best_trade_sharpe.clone(), 0.95),
        best_profit_factor,
        best_return_drawdown,
        best_expectancy,
        best_trade_sharpe,
    })
}

fn metrics_tuple(metrics: &BacktestMetrics) -> (f64, f64, f64, f64) {
    let pf = metrics.profit_factor.unwrap_or(0.0);
    let ret_dd = if metrics.max_drawdown_percent > 1.0e-9 {
        metrics.return_percent / metrics.max_drawdown_percent
    } else if metrics.return_percent > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let sharpe = metrics.sharpe_ratio.unwrap_or(0.0);
    (pf, ret_dd, metrics.expectancy, sharpe)
}

fn percentile(values: &mut [f64], probability: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let index = (probability * (values.len() - 1) as f64).round() as usize;
    values[index]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bars(count: usize) -> Vec<Bar> {
        (0..count)
            .map(|index| {
                let open = 100.0 + (index as f64 * 0.01).sin();
                Bar {
                    timestamp_ms: index as i64 * 60_000,
                    open,
                    high: open + 0.1,
                    low: open - 0.1,
                    close: open + 0.02,
                    tick_volume: 10,
                    real_volume: 0,
                    spread_points: Some(5),
                }
            })
            .collect()
    }

    #[test]
    fn stationary_bootstrap_preserves_length_and_spreads() {
        let source = bars(500);
        let synth = stationary_bootstrap_bars(&source, 60, 11);
        assert_eq!(synth.len(), source.len());
        assert_eq!(synth[0].timestamp_ms, source[0].timestamp_ms);
        assert_eq!(synth.last().unwrap().timestamp_ms, source.last().unwrap().timestamp_ms);
        for (left, right) in source.iter().zip(synth.iter()) {
            assert_eq!(left.spread_points, right.spread_points);
            assert_eq!(left.timestamp_ms, right.timestamp_ms);
        }
    }
}
