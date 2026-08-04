//! One-shot IS/OOS1/OOS2 report for an evolve databank.
//! cargo run -p quantforge-cli --example partition_report -- <databank> <h1> <metadata> <broker>

use quantforge_broker::SymbolSpecification;
use quantforge_data::{BarDataset, Mt5ExportMetadata};
use quantforge_discover::Databank;
use quantforge_eval::evaluate_strategy;
use quantforge_quality::DataSplitPlan;
use serde_json::Value;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let databank_path = args.next().ok_or("databank path required")?;
    let h1_path = args.next().ok_or("H1 path required")?;
    let metadata_path = args.next().ok_or("metadata path required")?;
    let broker_path = args.next().ok_or("broker path required")?;

    let root: Value = serde_json::from_slice(&fs::read(&databank_path)?)?;
    let bank: Databank = serde_json::from_value(root["databank"].clone())?;
    let metadata = Mt5ExportMetadata::load(&metadata_path)?;
    let timezone = metadata.source_timezone()?;
    let dataset = BarDataset::load_mt5(Path::new(&h1_path), timezone)?;
    let broker: SymbolSpecification = serde_json::from_slice(&fs::read(&broker_path)?)?;
    let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2)?;
    let scout = bank.config.scout.clone();

    let is_end = plan.development.end_timestamp_ms_exclusive;
    let oos1_end = plan.validation.end_timestamp_ms_exclusive;
    let oos2_end = plan.sealed_final.end_timestamp_ms_exclusive;

    println!("databank\t{}", databank_path);
    println!(
        "bars\tIS={}\tOOS1={}\tOOS2={}",
        plan.development.bar_count, plan.validation.bar_count, plan.sealed_final.bar_count
    );
    println!(
        "evaluations\t{}\tgenerations\t{}\telites\t{}",
        bank.evaluation_count,
        bank.completed_generations,
        bank.elites.len()
    );
    println!(
        "id\tconditions\tis_exp\toos1_exp\toos1x\toos2_exp\tis_ret%\toos1_ret%\toos2_ret%\tgate"
    );

    let mut rows = Vec::new();
    for elite in &bank.elites {
        let result = evaluate_strategy(&elite.strategy, &dataset, &broker, &scout)?;
        let is_trades: Vec<_> = result
            .trades
            .iter()
            .filter(|trade| trade.entry_timestamp_ms < is_end)
            .collect();
        let oos1_trades: Vec<_> = result
            .trades
            .iter()
            .filter(|trade| {
                trade.entry_timestamp_ms >= is_end && trade.entry_timestamp_ms < oos1_end
            })
            .collect();
        let oos2_trades: Vec<_> = result
            .trades
            .iter()
            .filter(|trade| {
                trade.entry_timestamp_ms >= oos1_end && trade.entry_timestamp_ms < oos2_end
            })
            .collect();
        let is_exp = mean(&is_trades);
        let oos1_exp = mean(&oos1_trades);
        let oos2_exp = mean(&oos2_trades);
        let ratio = if is_exp > 0.0 {
            Some(oos1_exp / is_exp)
        } else {
            None
        };
        let pass = is_exp > 0.0 && oos1_exp > 0.0 && oos1_exp >= 0.7 * is_exp;
        let conditions = format!("entry{}", elite.niche.entry_conditions);
        rows.push((
            elite.strategy.id.clone(),
            conditions,
            is_exp,
            oos1_exp,
            ratio,
            oos2_exp,
            segment_return(&result.equity, scout.initial_balance, None, Some(is_end)),
            segment_return(
                &result.equity,
                scout.initial_balance,
                Some(is_end),
                Some(oos1_end),
            ),
            segment_return(
                &result.equity,
                scout.initial_balance,
                Some(oos1_end),
                Some(oos2_end),
            ),
            pass,
            elite.evidence.total,
        ));
    }
    rows.sort_by(|a, b| b.10.partial_cmp(&a.10).unwrap());
    for row in rows.iter() {
        println!(
            "{}\t{}\t{:.2}\t{:.2}\t{}\t{:.2}\t{:.2}\t{:.2}\t{:.2}\t{}",
            row.0,
            row.1,
            row.2,
            row.3,
            row.4
                .map(|value| format!("{value:.2}"))
                .unwrap_or_else(|| "—".into()),
            row.5,
            row.6,
            row.7,
            row.8,
            if row.9 { "PASS" } else { "FAIL" }
        );
    }
    let total_pass = rows.iter().filter(|row| row.9).count();
    println!(
        "oos1_gate_pass\t{}\tof\t{}\t({:.1}%)",
        total_pass,
        rows.len(),
        100.0 * total_pass as f64 / rows.len().max(1) as f64
    );
    Ok(())
}

fn mean(trades: &[&quantforge_eval::Trade]) -> f64 {
    if trades.is_empty() {
        0.0
    } else {
        trades.iter().map(|trade| trade.net_profit).sum::<f64>() / trades.len() as f64
    }
}

fn segment_return(
    equity: &[quantforge_eval::EquityPoint],
    initial_balance: f64,
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> f64 {
    let start_equity = start_ms
        .and_then(|boundary| {
            equity
                .iter()
                .rev()
                .find(|point| point.timestamp_ms < boundary)
                .map(|point| point.equity)
        })
        .unwrap_or(initial_balance);
    let end_equity = end_ms
        .and_then(|boundary| {
            equity
                .iter()
                .rev()
                .find(|point| point.timestamp_ms < boundary)
                .map(|point| point.equity)
        })
        .or_else(|| equity.last().map(|point| point.equity))
        .unwrap_or(start_equity);
    if start_equity.abs() < 1e-12 {
        0.0
    } else {
        ((end_equity - start_equity) / start_equity) * 100.0
    }
}
