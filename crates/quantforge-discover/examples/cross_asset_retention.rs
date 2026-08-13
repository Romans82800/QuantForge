use quantforge_broker::SymbolSpecification;
use quantforge_data::{
    BarDataset, Mt5ExportMetadata, QuoteBarDataset, bar_content_hash, build_timeframe_from_m1,
    build_timeframe_from_m1_with_quotes, infer_median_interval_ms,
};
use quantforge_discover::{
    Databank, RobustnessConfig, RobustnessReject, development_cpcv_diagnostic,
    run_m1_predeposit_robustness,
};
use quantforge_eval::{BacktestMetrics, evaluate_strategy};
use quantforge_quality::DataSplitPlan;
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1_with_quotes};
use serde::Deserialize;
use std::{env, error::Error, fs, path::Path};

#[derive(Deserialize)]
struct Artifact {
    databank: Databank,
}

#[derive(Default)]
struct Summary {
    rows: usize,
    retention_passes: usize,
    trade_ratio: Vec<f64>,
    return_error: Vec<f64>,
    drawdown_error: Vec<f64>,
}

impl Summary {
    fn push(&mut self, scout: &BacktestMetrics, m1: &BacktestMetrics, minimum: f64) {
        self.rows += 1;
        self.retention_passes += usize::from(retention_passes(scout, m1, minimum));
        self.trade_ratio.push(if scout.trade_count == 0 {
            1.0
        } else {
            m1.trade_count as f64 / scout.trade_count as f64
        });
        self.return_error
            .push((m1.return_percent - scout.return_percent).abs());
        self.drawdown_error
            .push((m1.max_drawdown_percent - scout.max_drawdown_percent).abs());
    }

    fn report(&mut self, label: &str) {
        println!(
            "{label}: retention {}/{} ({:.1}%), median trade ratio {:.3}, |return gap| {:.2}pp, |DD gap| {:.2}pp",
            self.retention_passes,
            self.rows,
            100.0 * self.retention_passes as f64 / self.rows.max(1) as f64,
            median(&mut self.trade_ratio),
            median(&mut self.return_error),
            median(&mut self.drawdown_error),
        );
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values.get(values.len() / 2).copied().unwrap_or_default()
}

fn retention_passes(h1: &BacktestMetrics, m1: &BacktestMetrics, minimum: f64) -> bool {
    let return_ok = if h1.return_percent > 0.0 {
        m1.return_percent >= minimum * h1.return_percent
    } else {
        m1.return_percent >= h1.return_percent
    };
    let trade_ok = h1.trade_count == 0 && m1.trade_count == 0
        || h1.trade_count > 0 && m1.trade_count as f64 >= 0.8 * h1.trade_count as f64;
    let dd_ok = h1.max_drawdown_percent == 0.0 && m1.max_drawdown_percent == 0.0
        || h1.max_drawdown_percent > 0.0 && m1.max_drawdown_percent < 1.3 * h1.max_drawdown_percent;
    return_ok && trade_ok && dd_ok
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 8 {
        return Err(
            "usage: cross_asset_retention ARTIFACT H1 M1 M1_METADATA QUOTES BROKER SAMPLE".into(),
        );
    }
    let artifact: Artifact = serde_json::from_slice(&fs::read(&args[1])?)?;
    let metadata = Mt5ExportMetadata::load(&args[4])?;
    let timezone = metadata.source_timezone()?;
    let h1 = BarDataset::load_mt5(&args[2], timezone)?;
    let m1 = BarDataset::load_mt5(&args[3], timezone)?;
    let quotes = QuoteBarDataset::load_csv(Path::new(&args[5]))?;
    quotes.validate_against(&m1)?;
    let broker: SymbolSpecification = serde_json::from_slice(&fs::read(&args[6])?)?;
    let interval = infer_median_interval_ms(&h1.bars).unwrap_or(3_600_000);
    let grid: Vec<_> = h1.bars.iter().map(|bar| bar.timestamp_ms).collect();
    let zero = build_timeframe_from_m1(&m1, interval, Some(&grid))?;
    let quote =
        build_timeframe_from_m1_with_quotes(&m1, &quotes, broker.point, interval, Some(&grid))?;
    let zero_plan = DataSplitPlan::chronological(&zero, 0.2, 0.2)?;
    let quote_plan = DataSplitPlan::chronological(&quote, 0.2, 0.2)?;
    let zero_development = slice_development(&zero, zero_plan.development.bar_count);
    let quote_development = slice_development(&quote, quote_plan.development.bar_count);
    let sample: usize = args[7].parse()?;
    let stride = (artifact.databank.accepted_pool.len() / sample.max(1)).max(1);
    let judge = JudgeConfig {
        initial_balance: artifact.databank.config.scout.initial_balance,
        costs: artifact.databank.config.scout.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: artifact.databank.config.scout.indicator_engine,
        entry_window: artifact.databank.config.scout.entry_window,
    };
    let mut zero_summary = Summary::default();
    let mut quote_summary = Summary::default();
    let mut robust_rejects = std::collections::BTreeMap::<String, usize>::new();
    let mut cpcv_samples = Vec::new();
    for candidate in artifact
        .databank
        .accepted_pool
        .iter()
        .step_by(stride)
        .take(sample)
    {
        let zero_result = evaluate_strategy(
            &candidate.strategy,
            &zero_development,
            &broker,
            &artifact.databank.config.scout,
        )?;
        let quote_result = evaluate_strategy(
            &candidate.strategy,
            &quote_development,
            &broker,
            &artifact.databank.config.scout,
        )?;
        let m1_result = evaluate_strategy_m1_with_quotes(
            &candidate.strategy,
            &quote_development,
            &m1,
            &quotes,
            &broker,
            &judge,
        )?;
        zero_summary.push(
            &zero_result.metrics,
            &m1_result.metrics,
            artifact.databank.config.precision.minimum_return_retention,
        );
        quote_summary.push(
            &quote_result.metrics,
            &m1_result.metrics,
            artifact.databank.config.precision.minimum_return_retention,
        );
        let ranges = &artifact.databank.config.search_ranges;
        let robustness = RobustnessConfig {
            folds: artifact.databank.config.robustness_folds,
            monte_carlo_trials: artifact.databank.config.robustness_monte_carlo_trials,
            monte_carlo_block_length: artifact.databank.config.robustness_monte_carlo_block_length,
            monte_carlo_skip_trade_probability: artifact
                .databank
                .config
                .robustness_monte_carlo_skip_trade_probability,
            monte_carlo_minimum_p80_profit_retention: artifact
                .databank
                .config
                .robustness_monte_carlo_p80_profit_retention,
            monte_carlo_max_drawdown_ratio: artifact
                .databank
                .config
                .robustness_monte_carlo_max_drawdown_ratio,
            neighborhood_samples: artifact.databank.config.robustness_neighborhood_samples,
            seed: artifact.databank.config.seed,
            initial_balance: artifact.databank.config.scout.initial_balance,
            costs: artifact.databank.config.scout.costs.clone(),
            entry_window: artifact.databank.config.scout.entry_window,
            minimum_return_retention: artifact.databank.config.precision.minimum_return_retention,
            minimum_fold_trades: artifact
                .databank
                .config
                .deposit_gates
                .minimum_trades
                .clamp(1, 2),
            minimum_return_percent: artifact
                .databank
                .config
                .deposit_gates
                .minimum_return_percent,
            minimum_profit_factor: artifact
                .databank
                .config
                .deposit_gates
                .minimum_profit_factor
                .min(1.0),
            maximum_drawdown_percent: artifact
                .databank
                .config
                .deposit_gates
                .maximum_drawdown_percent
                .max(30.0),
            minimum_passing_fold_fraction: 0.6,
            minimum_neighborhood_survival_fraction: artifact
                .databank
                .config
                .minimum_neighborhood_survival_fraction,
            parameter_perturbation_fraction: artifact
                .databank
                .config
                .robustness_perturbation_fraction,
            adx_period_min: ranges.indicator_period.minimum.round().max(2.0) as u16,
            adx_period_max: ranges.indicator_period.maximum.round().max(2.0) as u16,
            adx_period_step: ranges.indicator_period.step.round().max(1.0) as u16,
            adx_threshold_min: ranges.adx_threshold.minimum,
            adx_threshold_max: ranges.adx_threshold.maximum,
            adx_threshold_step: ranges.adx_threshold.step,
            indicator_engine: artifact.databank.config.scout.indicator_engine,
            calendar_year_folds: artifact.databank.config.calendar_year_folds,
        };
        let result = run_m1_predeposit_robustness(
            &candidate.strategy,
            &quote_development,
            &m1,
            Some(&quotes),
            &broker,
            &robustness,
            &quote_result.metrics,
        );
        let label = match result {
            Ok(_) => "passed",
            Err(RobustnessReject::M1Fidelity) => "m1_fidelity",
            Err(RobustnessReject::WalkForward) => "development_cpcv",
            Err(RobustnessReject::MonteCarlo) => "monte_carlo",
            Err(RobustnessReject::ParamNeighborhood) => "parameter_plateau",
        };
        if label == "development_cpcv" && cpcv_samples.len() < 3 {
            let rows = development_cpcv_diagnostic(
                &candidate.strategy,
                &quote_development,
                &m1,
                Some(&quotes),
                &broker,
                &robustness,
            )
            .map_err(|error| format!("CPCV diagnostic failed: {error:?}"))?;
            cpcv_samples.push(
                rows.iter()
                    .map(|row| {
                        format!(
                            "{}:{}tr/{:.2}%/PF{:.2}/{}",
                            row.test_groups
                                .iter()
                                .map(usize::to_string)
                                .collect::<Vec<_>>()
                                .join("+"),
                            row.trades_in_fold,
                            row.metrics.return_percent,
                            row.metrics.profit_factor.unwrap_or(0.0),
                            if row.passed { "pass" } else { "fail" },
                        )
                    })
                    .collect::<Vec<_>>(),
            );
        }
        *robust_rejects.entry(label.into()).or_default() += 1;
    }
    println!(
        "{}: {} accepted-pool strategies, sampled {}",
        broker.symbol,
        artifact.databank.accepted_pool.len(),
        zero_summary.rows
    );
    zero_summary.report("zero-spread decision scout");
    quote_summary.report("quote-spread decision scout");
    println!("quote-aware robustness funnel: {robust_rejects:?}");
    for (index, rows) in cpcv_samples.iter().enumerate() {
        println!("CPCV rejection sample {}: {}", index + 1, rows.join(", "));
    }
    Ok(())
}

fn slice_development(dataset: &BarDataset, end: usize) -> BarDataset {
    let bars = dataset.bars[..end].to_vec();
    BarDataset {
        data_hash: bar_content_hash(&bars),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: dataset.delimiter,
        source_timezone: dataset.source_timezone.clone(),
        bars,
    }
}
