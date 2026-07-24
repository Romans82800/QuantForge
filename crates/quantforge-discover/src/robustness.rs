//! M1 robustness battery on IS (before OOS1 pick) for Discover databank promotion.

use quantforge_broker::SymbolSpecification;
use quantforge_data::{BarDataset, bar_content_hash};
use quantforge_ir::StrategyIr;
use quantforge_quality::{monte_carlo_from_trade_profits, perturb_strategy_parameters};
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RobustnessReject {
    M1Fidelity,
    WalkForward,
    MonteCarlo,
    ParamNeighborhood,
}

pub(crate) struct RobustnessConfig {
    pub folds: usize,
    pub monte_carlo_trials: usize,
    pub neighborhood_samples: usize,
    pub seed: u64,
    pub initial_balance: f64,
    pub costs: quantforge_eval::CostModel,
    pub minimum_fold_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
    pub minimum_passing_fold_fraction: f64,
    pub minimum_neighborhood_survival_fraction: f64,
    pub parameter_perturbation_fraction: f64,
}

/// M1 baseline → WFO folds → trade-block MC → ±param neighborhood on an IS window.
pub(crate) fn run_m1_predeposit_robustness(
    strategy: &StrategyIr,
    is_decision: &BarDataset,
    m1_dataset: &BarDataset,
    broker: &SymbolSpecification,
    config: &RobustnessConfig,
) -> Result<(), RobustnessReject> {
    let judge = JudgeConfig {
        initial_balance: config.initial_balance,
        costs: config.costs.clone(),
        allow_execution_gaps: true,
    };
    let baseline = evaluate_strategy_m1(strategy, is_decision, m1_dataset, broker, &judge)
        .map_err(|_| RobustnessReject::M1Fidelity)?;
    if !metrics_pass(
        &baseline.metrics,
        config.minimum_fold_trades.max(5),
        config,
    ) {
        return Err(RobustnessReject::M1Fidelity);
    }

    let mut passing_folds = 0usize;
    for fold in 0..config.folds {
        let start = is_decision.bars.len() * fold / config.folds;
        let end = is_decision.bars.len() * (fold + 1) / config.folds;
        if end <= start + 1 {
            continue;
        }
        let lookback = 120usize;
        let slice_start = start.saturating_sub(lookback);
        let decision_slice = slice_dataset(is_decision, slice_start, end);
        let start_ms = is_decision.bars[start].timestamp_ms;
        let end_ms = is_decision.bars[end - 1].timestamp_ms;
        let m1_slice = slice_m1_covering(m1_dataset, start_ms, end_ms);
        let fold_result =
            evaluate_strategy_m1(strategy, &decision_slice, &m1_slice, broker, &judge)
                .map_err(|_| RobustnessReject::WalkForward)?;
        let fold_trades = fold_result
            .trades
            .iter()
            .filter(|trade| {
                trade.entry_timestamp_ms >= start_ms && trade.entry_timestamp_ms <= end_ms
            })
            .count();
        if fold_trades >= config.minimum_fold_trades
            && fold_result.metrics.return_percent > config.minimum_return_percent
            && effective_pf(&fold_result.metrics) >= config.minimum_profit_factor
            && fold_result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent
        {
            passing_folds += 1;
        }
    }
    let fold_fraction = passing_folds as f64 / config.folds.max(1) as f64;
    if fold_fraction + 1e-12 < config.minimum_passing_fold_fraction {
        return Err(RobustnessReject::WalkForward);
    }

    let profits: Vec<_> = baseline
        .trades
        .iter()
        .map(|trade| trade.net_profit)
        .collect();
    let mc = monte_carlo_from_trade_profits(
        &profits,
        config.initial_balance,
        config.monte_carlo_trials,
        5,
        config.seed,
        0.0,
        config.maximum_drawdown_percent.max(35.0),
    );
    if !mc.passed || mc.median_net_profit < 0.0 {
        return Err(RobustnessReject::MonteCarlo);
    }

    let mut surviving = 0usize;
    for sample in 0..config.neighborhood_samples {
        let Ok(neighbor) = perturb_strategy_parameters(
            strategy,
            config.parameter_perturbation_fraction,
            sample,
            config.seed,
        ) else {
            continue;
        };
        let Ok(result) =
            evaluate_strategy_m1(&neighbor, is_decision, m1_dataset, broker, &judge)
        else {
            continue;
        };
        let return_ratio = if baseline.metrics.return_percent > 0.0 {
            result.metrics.return_percent / baseline.metrics.return_percent
        } else {
            1.0
        };
        let trade_ratio = if baseline.metrics.trade_count == 0 {
            0.0
        } else {
            result.metrics.trade_count as f64 / baseline.metrics.trade_count as f64
        };
        let dd_limit = if baseline.metrics.max_drawdown_percent > 0.0 {
            baseline.metrics.max_drawdown_percent * 1.5
        } else {
            config.maximum_drawdown_percent
        };
        if result.metrics.return_percent > config.minimum_return_percent
            && return_ratio >= 0.5
            && result.metrics.max_drawdown_percent <= dd_limit
            && result.metrics.max_drawdown_percent <= config.maximum_drawdown_percent
            && trade_ratio >= 0.5
        {
            surviving += 1;
        }
    }
    let survival = surviving as f64 / config.neighborhood_samples.max(1) as f64;
    if survival + 1e-12 < config.minimum_neighborhood_survival_fraction {
        return Err(RobustnessReject::ParamNeighborhood);
    }
    Ok(())
}

fn metrics_pass(
    metrics: &quantforge_eval::BacktestMetrics,
    minimum_trades: usize,
    config: &RobustnessConfig,
) -> bool {
    metrics.trade_count >= minimum_trades
        && metrics.return_percent > config.minimum_return_percent
        && effective_pf(metrics) >= config.minimum_profit_factor
        && metrics.max_drawdown_percent <= config.maximum_drawdown_percent
}

fn effective_pf(metrics: &quantforge_eval::BacktestMetrics) -> f64 {
    metrics
        .profit_factor
        .unwrap_or(if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::INFINITY
        } else {
            0.0
        })
}

fn slice_dataset(source: &BarDataset, start: usize, end: usize) -> BarDataset {
    let bars = source.bars[start..end].to_vec();
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

fn slice_m1_covering(m1: &BarDataset, start_ms: i64, end_ms: i64) -> BarDataset {
    let pad_ms = 7 * 24 * 60 * 60 * 1000;
    let from = start_ms.saturating_sub(pad_ms);
    let bars: Vec<_> = m1
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms >= from && bar.timestamp_ms <= end_ms)
        .cloned()
        .collect();
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: m1.delimiter,
        source_timezone: m1.source_timezone.clone(),
        bars,
    }
}
