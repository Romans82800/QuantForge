//! Development-only calendar-year R. Never reads sealed/OOS2 trades.

use chrono::Datelike;
use quantforge_eval::Trade;
use serde::{Deserialize, Serialize};

const MIN_TRADES_PER_FOLD: usize = 5;
const MAX_YEAR_PNL_SHARE: f64 = 0.55;
const MAX_POOLED_OVER_MEDIAN: f64 = 2.5;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FoldRStats {
    pub fold_count: usize,
    pub median_fold_r: f64,
    pub fold_spread: f64,
    pub pooled_r: f64,
    pub max_year_share: f64,
    pub has_negative_fold: bool,
    pub usable: bool,
}

impl Default for FoldRStats {
    fn default() -> Self {
        Self {
            fold_count: 0,
            median_fold_r: 0.0,
            fold_spread: 0.0,
            pooled_r: 0.0,
            max_year_share: 1.0,
            has_negative_fold: false,
            usable: false,
        }
    }
}

impl FoldRStats {
    /// Rank score: modest, stable R. Lucky single-year miracles lose.
    pub fn rank_r(&self) -> f64 {
        if !self.usable {
            return self.pooled_r;
        }
        self.median_fold_r - 0.5 * self.fold_spread
    }

    pub fn passes_stability(&self) -> bool {
        if !self.usable {
            return self.pooled_r > 0.0;
        }
        !self.has_negative_fold
            && self.median_fold_r > 0.0
            && self.max_year_share <= MAX_YEAR_PNL_SHARE
            && self.pooled_r <= self.median_fold_r * MAX_POOLED_OVER_MEDIAN + 0.05
    }
}

pub fn calendar_year_fold_r(trades: &[Trade]) -> FoldRStats {
    let pooled_r = if trades.is_empty() {
        0.0
    } else {
        trades.iter().map(|trade| trade.r_multiple).sum::<f64>() / trades.len() as f64
    };
    let mut by_year: std::collections::BTreeMap<i32, (f64, f64, usize)> =
        std::collections::BTreeMap::new();
    for trade in trades {
        let year = utc_year(trade.entry_timestamp_ms);
        let slot = by_year.entry(year).or_insert((0.0, 0.0, 0));
        slot.0 += trade.r_multiple;
        slot.1 += trade.net_profit.abs();
        slot.2 += 1;
    }
    let mut fold_means: Vec<f64> = Vec::new();
    let mut abs_pnl: Vec<f64> = Vec::new();
    let mut has_negative_fold = false;
    for (_year, (sum_r, year_abs, count)) in &by_year {
        if *count < MIN_TRADES_PER_FOLD {
            continue;
        }
        let mean = *sum_r / *count as f64;
        if mean < 0.0 {
            has_negative_fold = true;
        }
        fold_means.push(mean);
        abs_pnl.push(*year_abs);
    }
    let usable = fold_means.len() >= 2;
    let median_fold_r = if usable {
        median(&mut fold_means.clone())
    } else {
        pooled_r
    };
    let fold_spread = if usable { iqr(&mut fold_means) } else { 0.0 };
    let total_abs: f64 = abs_pnl.iter().sum();
    let max_year_share = if total_abs > 1.0e-9 {
        abs_pnl.iter().copied().fold(0.0_f64, f64::max) / total_abs
    } else {
        1.0
    };
    FoldRStats {
        fold_count: fold_means.len(),
        median_fold_r,
        fold_spread,
        pooled_r,
        max_year_share,
        has_negative_fold,
        usable,
    }
}

fn utc_year(timestamp_ms: i64) -> i32 {
    chrono::DateTime::from_timestamp_millis(timestamp_ms)
        .map(|time| time.date_naive().year())
        .unwrap_or(1970)
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}

fn iqr(values: &mut [f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let q1 = percentile(values, 0.25);
    let q3 = percentile(values, 0.75);
    (q3 - q1).abs()
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_eval::{ExitReason, PositionSide, Trade};

    fn trade(year: i32, r: f64) -> Trade {
        let ts = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, year, 6, 15, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        Trade {
            side: PositionSide::Long,
            entry_timestamp_ms: ts,
            exit_timestamp_ms: ts + 3_600_000,
            entry_price: 70.0,
            exit_price: 70.0,
            volume: 1.0,
            initial_stop_loss: 69.0,
            initial_take_profit: 72.0,
            gross_profit: r * 1_000.0,
            commission: 0.0,
            swap: 0.0,
            net_profit: r * 1_000.0,
            bars_held: 1,
            exit_reason: ExitReason::TakeProfit,
            r_multiple: r,
        }
    }

    fn many(year: i32, r: f64, count: usize) -> Vec<Trade> {
        (0..count).map(|_| trade(year, r)).collect()
    }

    #[test]
    fn lucky_year_loses_to_stable_modest_r() {
        let mut lucky = many(2020, 0.80, 20);
        lucky.extend(many(2021, -0.05, 8));
        lucky.extend(many(2022, -0.05, 8));
        let mut stable = many(2020, 0.10, 12);
        stable.extend(many(2021, 0.10, 12));
        stable.extend(many(2022, 0.10, 12));
        let lucky_stats = calendar_year_fold_r(&lucky);
        let stable_stats = calendar_year_fold_r(&stable);
        assert!(lucky_stats.usable && stable_stats.usable);
        assert!(stable_stats.rank_r() > lucky_stats.rank_r());
        assert!(!lucky_stats.passes_stability());
        assert!(stable_stats.passes_stability());
    }
}
