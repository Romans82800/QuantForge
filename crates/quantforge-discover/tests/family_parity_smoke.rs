//! Smoke-test every Search Family for both-side fire + export wiring.
//!
//! This is not a full MT5 tick-parity suite. It catches the class of bug we
//! just hit on SupplyDemandReclaim: a custom indicator silently NaN on one
//! side so Rust only trades longs while the EA trades both.

use quantforge_broker::SymbolSpecification;
use quantforge_core::FloatPolicy;
use quantforge_data::{BarDataset, SourceTimezone};
use quantforge_discover::{SearchFamily, generate_seed_for_family};
use quantforge_eval::{CostModel, ScoutConfig, evaluate_strategy};
use quantforge_export_mql5::{ExportStyle, Mql5ExportConfig, generate_bundle};
use quantforge_ir::{IndicatorExpr, NumericExpr, Side};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn pack_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../ICMarkets_EST7_2020_present")
}

fn custom_indicators(strategy: &quantforge_ir::StrategyIr) -> BTreeSet<&'static str> {
    let mut names = BTreeSet::new();
    let mut visit = |expr: &NumericExpr| match expr {
        NumericExpr::Indicator { value } => match value {
            IndicatorExpr::SwingBaseZoneHigh { .. } | IndicatorExpr::SwingBaseZoneLow { .. } => {
                names.insert("swing_zone");
            }
            IndicatorExpr::LiquiditySweepScore { .. } => {
                names.insert("liquidity_sweep");
            }
            IndicatorExpr::SessionRangeHigh { .. } | IndicatorExpr::SessionRangeLow { .. } => {
                names.insert("session_range");
            }
            IndicatorExpr::AtrPercentile { .. } => {
                names.insert("atr_percentile");
            }
            IndicatorExpr::BodyRangeRatio { .. } => {
                names.insert("body_range");
            }
            IndicatorExpr::CloseLocationInBar { .. } => {
                names.insert("close_location");
            }
            IndicatorExpr::RateOfChange { .. } => {
                names.insert("roc");
            }
            IndicatorExpr::ZScore { .. } => {
                names.insert("zscore");
            }
            _ => {}
        },
        _ => {}
    };
    fn walk_bool(expr: &quantforge_ir::BoolExpr, visit: &mut impl FnMut(&NumericExpr)) {
        match expr {
            quantforge_ir::BoolExpr::Compare { left, right, .. }
            | quantforge_ir::BoolExpr::CrossAbove { left, right }
            | quantforge_ir::BoolExpr::CrossBelow { left, right } => {
                visit(left);
                visit(right);
            }
            quantforge_ir::BoolExpr::Between {
                value,
                lower,
                upper,
            } => {
                visit(value);
                visit(lower);
                visit(upper);
            }
            quantforge_ir::BoolExpr::And { children }
            | quantforge_ir::BoolExpr::Or { children } => {
                for child in children {
                    walk_bool(child, visit);
                }
            }
            quantforge_ir::BoolExpr::Not { child } => walk_bool(child, visit),
        }
    }
    for side in [&strategy.entry.long, &strategy.entry.short] {
        if let Some(expr) = side {
            walk_bool(expr, &mut visit);
        }
    }
    for filter in &strategy.filters {
        walk_bool(filter, &mut visit);
    }
    names
}

#[test]
fn family_parity_smoke_both_sides_and_export() {
    let root = pack_root();
    let data_path = root.join("ICMarketsSC-Demo_AUDUSD_H1_2020_present.tsv");
    let broker_path = root.join("AUDUSD.broker.json");
    if !data_path.exists() || !broker_path.exists() {
        eprintln!("skip: AUDUSD pack not present at {}", root.display());
        return;
    }

    let dataset = BarDataset::load_mt5(
        &data_path,
        "ICMarkets/EST+7".parse::<SourceTimezone>().unwrap(),
    )
    .expect("load AUDUSD H1");
    let broker: SymbolSpecification =
        serde_json::from_str(&std::fs::read_to_string(&broker_path).unwrap()).unwrap();
    let config = ScoutConfig {
        initial_balance: 100_000.0,
        costs: CostModel {
            commission_per_lot_round_turn: 7.0,
            ..CostModel::default()
        },
        ..ScoutConfig::default()
    };

    let mut failures = Vec::new();
    println!(
        "{:<22} {:>7} {:>6} {:>6} {:>7}  indicators",
        "family", "trades", "long", "short", "ret%"
    );

    for family in SearchFamily::ALL {
        let mut total_trades = 0usize;
        let mut total_long = 0usize;
        let mut total_short = 0usize;
        let mut best_ret = f64::NEG_INFINITY;
        let mut indicators = BTreeSet::new();
        let mut export_ok = true;

        // Several seeds so rare short genes still get a chance to fire.
        for sequence in 0..8u64 {
            let mut strategy = generate_seed_for_family(42, sequence, family);
            strategy.manage.max_one_entry_per_day = false;
            strategy.manage.flatten_end_of_day = false;
            indicators.extend(custom_indicators(&strategy));

            let result = evaluate_strategy(&strategy, &dataset, &broker, &config)
                .unwrap_or_else(|error| panic!("{family:?} seq={sequence}: {error}"));
            total_trades += result.metrics.trade_count;
            total_long += result
                .trades
                .iter()
                .filter(|trade| trade.side == quantforge_eval::PositionSide::Long)
                .count();
            total_short += result
                .trades
                .iter()
                .filter(|trade| trade.side == quantforge_eval::PositionSide::Short)
                .count();
            best_ret = best_ret.max(result.metrics.return_percent);

            if sequence == 0 {
                let fingerprint = strategy
                    .structural_fingerprint(FloatPolicy::default())
                    .unwrap();
                let export = generate_bundle(
                    &strategy,
                    &broker,
                    &Mql5ExportConfig {
                        expert_name: format!("smoke_{}", family.label()),
                        export_style: ExportStyle::Sqx,
                        magic: 42_000 + sequence as u64,
                        ..Mql5ExportConfig::default()
                    },
                )
                .unwrap_or_else(|error| panic!("export {family:?}: {error}"));
                export_ok = export.source.contains("sqValid")
                    && export.source.contains("QFLongSignal")
                    && export.source.contains("QFShortSignal")
                    && !export.source.contains("@@");
                assert_eq!(
                    export.evidence.strategy_fingerprint, fingerprint,
                    "export fingerprint drift for {family:?}"
                );
                if indicators.contains("swing_zone") {
                    export_ok &= export.source.contains("ready_delay")
                        || export.source.contains("sqSwingBaseZone");
                }
                if indicators.contains("liquidity_sweep") {
                    export_ok &= export.source.contains("LiquiditySweep")
                        || export.source.contains("sqLiquiditySweep");
                }
                if indicators.contains("session_range") {
                    export_ok &= export.source.contains("SessionRange")
                        || export.source.contains("QFSessionRange");
                }
            }
        }

        let both_sides_ok = matches!(
            (
                generate_seed_for_family(42, 0, family).side,
                total_long > 0,
                total_short > 0
            ),
            (Side::Both, true, true)
                | (Side::LongOnly, true, _)
                | (Side::ShortOnly, _, true)
                | (Side::Both, false, false) // no fills across seeds is soft
        );
        // Hard fail only when side=Both and one side is totally silent while
        // the other printed many trades — the SupplyDemand failure mode.
        let asymmetric = matches!(generate_seed_for_family(42, 0, family).side, Side::Both)
            && ((total_long >= 50 && total_short == 0) || (total_short >= 50 && total_long == 0));

        println!(
            "{:<22} {:>7} {:>6} {:>6} {:>7.1}  {:?} export={}",
            family.label(),
            total_trades,
            total_long,
            total_short,
            best_ret,
            indicators,
            if export_ok { "ok" } else { "BAD" }
        );

        if asymmetric {
            failures.push(format!(
                "{:?}: one-sided smoke (long={total_long} short={total_short}) — likely dead indicator path",
                family
            ));
        }
        if !export_ok {
            failures.push(format!("{:?}: export helpers incomplete", family));
        }
        if !both_sides_ok && total_trades > 0 {
            // informational only for LongOnly grammars
        }
        let _ = both_sides_ok;
    }

    assert!(
        failures.is_empty(),
        "family parity smoke failures:\n{}",
        failures.join("\n")
    );
}
