//! Post-Discover Holding shrink by daily closed-trade P/L correlation.
//!
//! This is not a Discover start gate. Run it on an open archive before the
//! on-demand battery so correlated clones do not each pay for CPCV/MC.

use crate::archive::refresh_fingerprint_coverage_map;
use crate::model::{Databank, Elite};
use chrono::Datelike;
use quantforge_broker::BrokerClock;
use quantforge_eval::Trade;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
pub struct HoldingCorrShrinkReport {
    pub kept: usize,
    pub dropped: usize,
    pub max_correlation: f64,
}

pub fn daily_pnl_from_trades(trades: &[Trade], timezone: &str) -> BTreeMap<i32, f64> {
    let mut days = BTreeMap::new();
    let Ok(clock) = BrokerClock::parse(timezone) else {
        return days;
    };
    for trade in trades {
        let Ok(local) = clock.local_datetime(trade.exit_timestamp_ms) else {
            continue;
        };
        let key = local.year() * 10_000 + local.month() as i32 * 100 + local.day() as i32;
        *days.entry(key).or_default() += trade.net_profit;
    }
    days
}

pub fn align_daily_pnl(maps: &[BTreeMap<i32, f64>]) -> Vec<Vec<f64>> {
    let mut days = BTreeSet::new();
    for map in maps {
        days.extend(map.keys().copied());
    }
    let days: Vec<i32> = days.into_iter().collect();
    maps.iter()
        .map(|map| {
            days.iter()
                .map(|day| map.get(day).copied().unwrap_or(0.0))
                .collect()
        })
        .collect()
}

pub fn pearson(left: &[f64], right: &[f64]) -> f64 {
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
        let left_delta = left - left_mean;
        let right_delta = right - right_mean;
        covariance += left_delta * right_delta;
        left_variance += left_delta * left_delta;
        right_variance += right_delta * right_delta;
    }
    let denominator = (left_variance * right_variance).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        (covariance / denominator).clamp(-1.0, 1.0)
    }
}

pub fn greedy_keep_indices(
    scores: &[f64],
    aligned: &[Vec<f64>],
    max_correlation: f64,
) -> Vec<usize> {
    let mut order: Vec<usize> = (0..scores.len()).collect();
    order.sort_by(|&left, &right| scores[right].total_cmp(&scores[left]));
    let mut kept = Vec::new();
    for index in order {
        let series = aligned.get(index).map(Vec::as_slice).unwrap_or(&[]);
        let too_close = kept.iter().any(|&peer| {
            pearson(series, aligned.get(peer).map(Vec::as_slice).unwrap_or(&[])) > max_correlation
        });
        if !too_close {
            kept.push(index);
        }
    }
    kept
}

fn holding_rank_score(elite: &Elite) -> f64 {
    elite.fold_r.median_fold_r * 1_000.0 + elite.evidence.total + elite.is_expectancy * 0.01
}

pub fn apply_holding_daily_corr_shrink(
    bank: &mut Databank,
    daily_pnl: &[BTreeMap<i32, f64>],
    max_correlation: f64,
) -> HoldingCorrShrinkReport {
    let aligned = align_daily_pnl(daily_pnl);
    let scores: Vec<f64> = bank.holding.iter().map(holding_rank_score).collect();
    let keep = greedy_keep_indices(&scores, &aligned, max_correlation);
    let original = std::mem::take(&mut bank.holding);
    let before = original.len();
    bank.holding = keep
        .into_iter()
        .filter_map(|index| original.get(index).cloned())
        .collect();
    refresh_fingerprint_coverage_map(&bank.holding, &mut bank.holding_coverage_map);
    HoldingCorrShrinkReport {
        kept: bank.holding.len(),
        dropped: before.saturating_sub(bank.holding.len()),
        max_correlation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_daily_path_is_dropped_and_the_stronger_name_is_kept() {
        let scores = [0.2, 0.8];
        let aligned = vec![vec![1.0, 2.0, 3.0, 4.0], vec![1.0, 2.0, 3.0, 4.0]];
        let keep = greedy_keep_indices(&scores, &aligned, 0.5);
        assert_eq!(keep, vec![1]);
    }

    #[test]
    fn uncorrelated_daily_paths_both_survive_a_0_5_cap() {
        let scores = [0.4, 0.3];
        let aligned = vec![vec![1.0, 0.0, 1.0, 0.0], vec![0.0, 1.0, 0.0, 1.0]];
        let keep = greedy_keep_indices(&scores, &aligned, 0.5);
        assert_eq!(keep.len(), 2);
    }
}
