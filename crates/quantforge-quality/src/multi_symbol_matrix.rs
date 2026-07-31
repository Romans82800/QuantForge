//! Cross-symbol retest matrix (SQX CrossCheckRetestOnAdditionalMarkets stand-in).

use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::{ScoutConfig, evaluate_strategy};
use quantforge_ir::StrategyIr;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MULTI_SYMBOL_MATRIX_PROTOCOL: &str = "multi-symbol-matrix-v1";

#[derive(Debug, Error)]
pub enum MultiSymbolMatrixError {
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone)]
pub struct MatrixSymbolInput {
    pub symbol: String,
    pub dataset: BarDataset,
    pub broker: SymbolSpecification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiSymbolMatrixRow {
    pub symbol: String,
    pub passed: bool,
    pub trade_count: usize,
    pub return_percent: f64,
    pub profit_factor: Option<f64>,
    pub max_drawdown_percent: f64,
    pub net_profit: f64,
    pub win_rate: f64,
    pub expectancy: f64,
    /// Downsampled equity-return signature for pairwise correlation.
    #[serde(default)]
    pub equity_signature: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairwiseSymbolCorrelation {
    pub left: String,
    pub right: String,
    pub correlation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MultiSymbolMatrixReport {
    pub protocol: String,
    pub strategy_id: String,
    pub symbols: Vec<MultiSymbolMatrixRow>,
    pub passing_count: usize,
    pub required_pass: usize,
    pub matrix_passed: bool,
    pub pairwise_correlations: Vec<PairwiseSymbolCorrelation>,
    pub maximum_pairwise_correlation: f64,
    pub mean_return_percent: f64,
    pub mean_net_profit: f64,
}

/// Evaluate identical strategy parameters across a symbol pack and emit a matrix.
pub fn run_multi_symbol_matrix(
    strategy: &StrategyIr,
    markets: &[MatrixSymbolInput],
    scout: &ScoutConfig,
    required_pass: usize,
    minimum_net_profit: f64,
) -> Result<MultiSymbolMatrixReport, MultiSymbolMatrixError> {
    if markets.is_empty() {
        return Err(MultiSymbolMatrixError::Message(
            "at least one symbol is required".into(),
        ));
    }
    let mut rows = Vec::with_capacity(markets.len());
    for market in markets {
        match evaluate_strategy(strategy, &market.dataset, &market.broker, scout) {
            Ok(result) => {
                let signature = equity_signature(&result.equity, result.metrics.initial_balance, 64);
                let passed = result.metrics.net_profit > minimum_net_profit
                    && result.metrics.trade_count > 0;
                rows.push(MultiSymbolMatrixRow {
                    symbol: market.symbol.clone(),
                    passed,
                    trade_count: result.metrics.trade_count,
                    return_percent: result.metrics.return_percent,
                    profit_factor: result.metrics.profit_factor,
                    max_drawdown_percent: result.metrics.max_drawdown_percent,
                    net_profit: result.metrics.net_profit,
                    win_rate: result.metrics.win_rate,
                    expectancy: result.metrics.expectancy,
                    equity_signature: signature,
                });
            }
            Err(error) => {
                rows.push(MultiSymbolMatrixRow {
                    symbol: market.symbol.clone(),
                    passed: false,
                    trade_count: 0,
                    return_percent: 0.0,
                    profit_factor: None,
                    max_drawdown_percent: 0.0,
                    net_profit: 0.0,
                    win_rate: 0.0,
                    expectancy: 0.0,
                    equity_signature: Vec::new(),
                });
                let _ = error; // row already records failure
            }
        }
    }

    let mut pairwise = Vec::new();
    let mut max_corr = 0.0_f64;
    for left in 0..rows.len() {
        for right in (left + 1)..rows.len() {
            if rows[left].equity_signature.len() < 2 || rows[right].equity_signature.len() < 2 {
                continue;
            }
            let corr = correlation(&rows[left].equity_signature, &rows[right].equity_signature);
            max_corr = max_corr.max(corr);
            pairwise.push(PairwiseSymbolCorrelation {
                left: rows[left].symbol.clone(),
                right: rows[right].symbol.clone(),
                correlation: corr,
            });
        }
    }
    pairwise.sort_by(|a, b| b.correlation.total_cmp(&a.correlation));

    let passing_count = rows.iter().filter(|row| row.passed).count();
    let mean_return_percent = if rows.is_empty() {
        0.0
    } else {
        rows.iter().map(|r| r.return_percent).sum::<f64>() / rows.len() as f64
    };
    let mean_net_profit = if rows.is_empty() {
        0.0
    } else {
        rows.iter().map(|r| r.net_profit).sum::<f64>() / rows.len() as f64
    };

    Ok(MultiSymbolMatrixReport {
        protocol: MULTI_SYMBOL_MATRIX_PROTOCOL.into(),
        strategy_id: strategy.id.clone(),
        symbols: rows,
        passing_count,
        required_pass,
        matrix_passed: passing_count >= required_pass.max(1),
        pairwise_correlations: pairwise,
        maximum_pairwise_correlation: max_corr,
        mean_return_percent,
        mean_net_profit,
    })
}

fn equity_signature(
    equity: &[quantforge_eval::EquityPoint],
    initial_balance: f64,
    target_points: usize,
) -> Vec<f64> {
    if equity.is_empty() {
        return Vec::new();
    }
    let mut previous = initial_balance;
    let deltas: Vec<f64> = equity
        .iter()
        .map(|point| {
            let delta = point.equity - previous;
            previous = point.equity;
            delta
        })
        .collect();
    let chunk_size = deltas.len().div_ceil(target_points).max(1);
    deltas
        .chunks(chunk_size)
        .take(target_points)
        .map(|chunk| chunk.iter().sum::<f64>() / chunk.len() as f64)
        .collect()
}

fn correlation(left: &[f64], right: &[f64]) -> f64 {
    let length = left.len().min(right.len());
    if length < 2 {
        return 0.0;
    }
    let left = &left[..length];
    let right = &right[..length];
    let left_mean = left.iter().sum::<f64>() / length as f64;
    let right_mean = right.iter().sum::<f64>() / length as f64;
    let mut covariance = 0.0;
    let mut left_variance = 0.0;
    let mut right_variance = 0.0;
    for (left, right) in left.iter().zip(right) {
        let ld = left - left_mean;
        let rd = right - right_mean;
        covariance += ld * rd;
        left_variance += ld * ld;
        right_variance += rd * rd;
    }
    let denom = (left_variance * right_variance).sqrt();
    if denom <= f64::EPSILON {
        0.0
    } else {
        (covariance / denom).clamp(-1.0, 1.0).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::BarDataset;
    use quantforge_eval::ScoutConfig;
    use std::path::PathBuf;

    fn fixtures() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }

    #[test]
    fn matrix_runs_on_fixture_symbol_twice() {
        let strategy: StrategyIr = serde_json::from_str(
            &std::fs::read_to_string(fixtures().join("EURUSD_fixture_strategy.json")).unwrap(),
        )
        .unwrap();
        let broker: SymbolSpecification = serde_json::from_str(
            &std::fs::read_to_string(fixtures().join("EURUSD_fixture_broker.json")).unwrap(),
        )
        .unwrap();
        let data = fixtures().join("EURUSD_M15_sample.tsv");
        let dataset =
            BarDataset::load_mt5(&data, "Etc/UTC".parse().unwrap()).unwrap();
        let markets = vec![
            MatrixSymbolInput {
                symbol: "EURUSD".into(),
                dataset: dataset.clone(),
                broker: broker.clone(),
            },
            MatrixSymbolInput {
                symbol: "EURUSD_B".into(),
                dataset,
                broker,
            },
        ];
        let report = run_multi_symbol_matrix(&strategy, &markets, &ScoutConfig::default(), 1, 0.0)
            .unwrap();
        assert_eq!(report.symbols.len(), 2);
        assert_eq!(report.protocol, MULTI_SYMBOL_MATRIX_PROTOCOL);
        assert!(report.pairwise_correlations.len() <= 1);
    }
}
