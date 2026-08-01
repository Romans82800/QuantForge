//! SQX-style What-If trade filters (cross-check post-processing).
//!
//! These do not re-simulate the engine; they filter a finished trade blotter and
//! recompute headline metrics — matching SQX WhatIf / CrossCheckWhatIf surfaces.

use quantforge_eval::{BacktestMetrics, EquityPoint, Trade, calculate_metrics};
use serde::{Deserialize, Serialize};

/// One What-If filter applied in order to the trade list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WhatIfFilter {
    /// Drop the top `percent` of trades by absolute net profit (best winners).
    ExcludePctBiggestPl { percent: f64 },
    /// Drop the bottom `percent` of trades by net profit (worst losers).
    ExcludePctLowestPl { percent: f64 },
    ExcludeShortTrades,
    ExcludeLongTrades,
    /// Keep trade indices `0, n, 2n, …` (1-based every Nth in SQX naming).
    TakeEveryNthTrade { n: usize },
    /// Cap fills sharing the same UTC calendar day of entry.
    TakeMaxTradesPerDay { max: usize },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhatIfReport {
    pub protocol_version: String,
    pub original_trade_count: usize,
    pub filtered_trade_count: usize,
    pub removed_trade_count: usize,
    pub filters: Vec<WhatIfFilter>,
    pub metrics: BacktestMetrics,
    pub trades: Vec<Trade>,
}

pub const WHAT_IF_PROTOCOL_VERSION: &str = "what-if-v1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WhatIfError {
    #[error("invalid what-if config: {0}")]
    InvalidConfig(String),
}

/// Apply What-If filters and recompute metrics from the surviving trades.
///
/// Equity is reconstructed as a step path from cumulative net profit so drawdown
/// and return stay consistent with the filtered blotter (not the original equity).
pub fn apply_what_if(
    trades: &[Trade],
    initial_balance: f64,
    filters: &[WhatIfFilter],
) -> Result<WhatIfReport, WhatIfError> {
    if !initial_balance.is_finite() || initial_balance <= 0.0 {
        return Err(WhatIfError::InvalidConfig(
            "initial_balance must be finite and > 0".into(),
        ));
    }
    for filter in filters {
        validate_filter(filter)?;
    }
    let mut kept = trades.to_vec();
    for filter in filters {
        kept = apply_one(&kept, filter);
    }
    let equity = equity_from_trades(&kept, initial_balance);
    let ending = equity.last().map(|p| p.balance).unwrap_or(initial_balance);
    let metrics = calculate_metrics(initial_balance, ending, &kept, &equity);
    Ok(WhatIfReport {
        protocol_version: WHAT_IF_PROTOCOL_VERSION.into(),
        original_trade_count: trades.len(),
        filtered_trade_count: kept.len(),
        removed_trade_count: trades.len().saturating_sub(kept.len()),
        filters: filters.to_vec(),
        metrics,
        trades: kept,
    })
}

fn validate_filter(filter: &WhatIfFilter) -> Result<(), WhatIfError> {
    match filter {
        WhatIfFilter::ExcludePctBiggestPl { percent }
        | WhatIfFilter::ExcludePctLowestPl { percent } => {
            if !(0.0..=100.0).contains(percent) || !percent.is_finite() {
                return Err(WhatIfError::InvalidConfig(
                    "percent must be in [0, 100]".into(),
                ));
            }
        }
        WhatIfFilter::TakeEveryNthTrade { n } if *n == 0 => {
            return Err(WhatIfError::InvalidConfig(
                "take_every_nth_trade.n must be >= 1".into(),
            ));
        }
        WhatIfFilter::TakeMaxTradesPerDay { max } if *max == 0 => {
            return Err(WhatIfError::InvalidConfig(
                "take_max_trades_per_day.max must be >= 1".into(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn apply_one(trades: &[Trade], filter: &WhatIfFilter) -> Vec<Trade> {
    match *filter {
        WhatIfFilter::ExcludeShortTrades => trades
            .iter()
            .filter(|t| t.side != quantforge_eval::PositionSide::Short)
            .cloned()
            .collect(),
        WhatIfFilter::ExcludeLongTrades => trades
            .iter()
            .filter(|t| t.side != quantforge_eval::PositionSide::Long)
            .cloned()
            .collect(),
        WhatIfFilter::TakeEveryNthTrade { n } => trades
            .iter()
            .enumerate()
            .filter(|(index, _)| index % n == 0)
            .map(|(_, trade)| trade.clone())
            .collect(),
        WhatIfFilter::TakeMaxTradesPerDay { max } => {
            let mut counts = std::collections::BTreeMap::<i64, usize>::new();
            let mut out = Vec::new();
            for trade in trades {
                let day = trade.entry_timestamp_ms.div_euclid(86_400_000);
                let used = counts.entry(day).or_insert(0);
                if *used < max {
                    *used += 1;
                    out.push(trade.clone());
                }
            }
            out
        }
        WhatIfFilter::ExcludePctBiggestPl { percent } => {
            exclude_by_rank(trades, percent, true)
        }
        WhatIfFilter::ExcludePctLowestPl { percent } => {
            exclude_by_rank(trades, percent, false)
        }
    }
}

fn exclude_by_rank(trades: &[Trade], percent: f64, drop_best: bool) -> Vec<Trade> {
    if trades.is_empty() || percent <= 0.0 {
        return trades.to_vec();
    }
    let drop_count = ((trades.len() as f64) * percent / 100.0).ceil() as usize;
    let drop_count = drop_count.min(trades.len());
    if drop_count == 0 {
        return trades.to_vec();
    }
    let mut order: Vec<usize> = (0..trades.len()).collect();
    order.sort_by(|&a, &b| {
        let cmp = trades[a]
            .net_profit
            .partial_cmp(&trades[b].net_profit)
            .unwrap_or(std::cmp::Ordering::Equal);
        if drop_best {
            cmp.reverse()
        } else {
            cmp
        }
    });
    let mut drop = vec![false; trades.len()];
    for index in order.into_iter().take(drop_count) {
        drop[index] = true;
    }
    trades
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop[*i])
        .map(|(_, t)| t.clone())
        .collect()
}

fn equity_from_trades(trades: &[Trade], initial_balance: f64) -> Vec<EquityPoint> {
    let mut balance = initial_balance;
    let mut points = Vec::with_capacity(trades.len().saturating_add(1));
    points.push(EquityPoint {
        timestamp_ms: trades
            .first()
            .map(|t| t.entry_timestamp_ms.saturating_sub(1))
            .unwrap_or(0),
        balance,
        equity: balance,
    });
    for trade in trades {
        balance += trade.net_profit;
        points.push(EquityPoint {
            timestamp_ms: trade.exit_timestamp_ms,
            balance,
            equity: balance,
        });
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_eval::{ExitReason, PositionSide};

    fn trade(side: PositionSide, profit: f64, entry_ms: i64) -> Trade {
        Trade {
            side,
            entry_timestamp_ms: entry_ms,
            exit_timestamp_ms: entry_ms + 60_000,
            entry_price: 1.0,
            exit_price: 1.0,
            volume: 1.0,
            initial_stop_loss: 0.0,
            initial_take_profit: 0.0,
            gross_profit: profit,
            commission: 0.0,
            swap: 0.0,
            net_profit: profit,
            bars_held: 1,
            exit_reason: ExitReason::TakeProfit,
        }
    }

    #[test]
    fn exclude_biggest_winners_and_shorts() {
        let trades = vec![
            trade(PositionSide::Long, 100.0, 0),
            trade(PositionSide::Short, 50.0, 1),
            trade(PositionSide::Long, -20.0, 2),
            trade(PositionSide::Long, 10.0, 3),
        ];
        let report = apply_what_if(
            &trades,
            10_000.0,
            &[
                WhatIfFilter::ExcludePctBiggestPl { percent: 25.0 },
                WhatIfFilter::ExcludeShortTrades,
            ],
        )
        .unwrap();
        assert_eq!(report.original_trade_count, 4);
        assert_eq!(report.filtered_trade_count, 2);
        assert!(report.trades.iter().all(|t| t.side == PositionSide::Long));
        assert!(report.trades.iter().all(|t| t.net_profit < 100.0));
    }

    #[test]
    fn take_every_second_trade() {
        let trades: Vec<_> = (0..6)
            .map(|i| trade(PositionSide::Long, 1.0, i as i64 * 60_000))
            .collect();
        let report = apply_what_if(
            &trades,
            1_000.0,
            &[WhatIfFilter::TakeEveryNthTrade { n: 2 }],
        )
        .unwrap();
        assert_eq!(report.filtered_trade_count, 3);
    }
}
