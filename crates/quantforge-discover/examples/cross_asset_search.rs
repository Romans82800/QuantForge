use quantforge_broker::SymbolSpecification;
use quantforge_data::{
    BarDataset, Mt5ExportMetadata, QuoteBarDataset, bar_content_hash,
    build_timeframe_from_m1_with_quotes, infer_median_interval_ms,
};
use quantforge_discover::{Databank, evolve_new_with_pack_and_quotes};
use quantforge_quality::DataSplitPlan;
use serde::Deserialize;
use std::{env, error::Error, fs, path::Path};

#[derive(Deserialize)]
struct Artifact {
    databank: Databank,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 10 {
        return Err(
            "usage: cross_asset_search ARTIFACT H1 M1 M1_METADATA QUOTES BROKER GENERATIONS MUTATE_AFTER COMMISSION".into(),
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
    let decision =
        build_timeframe_from_m1_with_quotes(&m1, &quotes, broker.point, interval, Some(&grid))?;
    let plan = DataSplitPlan::chronological(&decision, 0.2, 0.2)?;
    let development = slice(&decision, 0, plan.development.bar_count);
    let oos1 = slice(
        &decision,
        plan.development.bar_count,
        plan.development.bar_count + plan.validation.bar_count,
    );
    let generations: u64 = args[7].parse()?;
    let mut config = artifact.databank.config;
    config.mutate_after_elites = args[8].parse()?;
    config.scout.costs.commission_per_lot_round_turn = args[9].parse()?;
    let bank = evolve_new_with_pack_and_quotes(
        &development,
        Some(&oos1),
        &m1,
        Some(&quotes),
        &broker,
        &[],
        &broker.symbol,
        config,
        generations,
    )?;
    let t = &bank.telemetry;
    println!(
        "{}: evals={} pot={} databank={} promotions={}/{}",
        broker.symbol,
        bank.evaluation_count,
        bank.accepted_pool.len(),
        bank.elites.len(),
        t.promotions_completed,
        t.promotions_enqueued,
    );
    println!(
        "rejects scout={} deposit={} ambiguous={} m1={} cpcv={} mc={} param={} oos1={} corr={} eval={}",
        t.rejected_gate,
        t.rejected_deposit_gate,
        t.rejected_ambiguous,
        t.rejected_m1_fidelity,
        t.rejected_walk_forward,
        t.rejected_monte_carlo,
        t.rejected_param_neighborhood,
        t.rejected_oos1,
        t.rejected_correlated,
        t.rejected_evaluation,
    );
    Ok(())
}

fn slice(dataset: &BarDataset, start: usize, end: usize) -> BarDataset {
    let bars = dataset.bars[start..end].to_vec();
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
