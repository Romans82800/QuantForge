//! Identical-parameter H1 screen across a pack of FX symbols.

use crate::archive::passes_gate_config;
use crate::model::{GateConfig, SymbolScreenResult};
use quantforge_broker::SymbolSpecification;
use quantforge_data::BarDataset;
use quantforge_eval::{ScoutConfig, ScoutResult, evaluate_strategy};
use quantforge_ir::StrategyIr;

/// One additional market for the cross-symbol screen (H1 only).
#[derive(Debug, Clone)]
pub struct PackSymbol {
    pub symbol: String,
    pub dataset: BarDataset,
    pub broker: SymbolSpecification,
}

/// FX majors/crosses used for the default multi-symbol gate.
pub const DEFAULT_FX_PACK: &[&str] = &[
    "AUDUSD", "EURGBP", "EURJPY", "EURNZD", "GBPJPY", "GBPUSD", "NZDUSD", "USDCHF", "USDJPY",
];

/// Non-gating asset classes that may be reported as out-of-universe evidence.
pub const DISPLAY_ONLY_SYMBOLS: &[&str] = &["XAUUSD", "US500"];

#[derive(Debug, Clone)]
pub struct MultiSymbolScreen {
    pub results: Vec<SymbolScreenResult>,
    pub passing: usize,
    pub pooled_profits: Vec<f64>,
}

fn screen_result(symbol: &str, result: &ScoutResult, gates: &GateConfig) -> SymbolScreenResult {
    SymbolScreenResult {
        symbol: symbol.into(),
        passed: passes_gate_config(result, gates) && result.metrics.net_profit > 0.0,
        trade_count: result.metrics.trade_count,
        return_percent: result.metrics.return_percent,
        profit_factor: result.metrics.profit_factor,
        net_profit: result.metrics.net_profit,
    }
}

/// Evaluate `strategy` with identical parameters on every pack symbol.
///
/// `primary_result` is the already-computed H1 scout on the discovery symbol;
/// pack members are evaluated fresh.
pub fn screen_multi_symbol(
    strategy: &StrategyIr,
    primary_symbol: &str,
    primary_result: &ScoutResult,
    pack: &[PackSymbol],
    scout: &ScoutConfig,
    gates: &GateConfig,
) -> MultiSymbolScreen {
    let mut results = Vec::with_capacity(pack.len() + 1);
    let mut pooled_profits: Vec<f64> = primary_result
        .trades
        .iter()
        .map(|trade| trade.net_profit)
        .collect();

    let primary = screen_result(primary_symbol, primary_result, gates);
    if primary.passed {
        // primary profits already in pool
    } else {
        pooled_profits.clear();
    }
    results.push(primary);

    for market in pack {
        match evaluate_strategy(strategy, &market.dataset, &market.broker, scout) {
            Ok(result) => {
                let row = screen_result(&market.symbol, &result, gates);
                if row.passed {
                    pooled_profits.extend(result.trades.iter().map(|trade| trade.net_profit));
                }
                results.push(row);
            }
            Err(_) => {
                results.push(SymbolScreenResult {
                    symbol: market.symbol.clone(),
                    passed: false,
                    trade_count: 0,
                    return_percent: 0.0,
                    profit_factor: None,
                    net_profit: 0.0,
                });
            }
        }
    }

    let passing = results.iter().filter(|result| result.passed).count();
    MultiSymbolScreen {
        results,
        passing,
        pooled_profits,
    }
}
