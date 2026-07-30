//! Emit one Discover institutional seed as Strategy IR JSON.
//!
//! Usage:
//!   cargo run -p quantforge-discover --example emit_family_strategy -- \
//!     --family trend_pullback --sequence 0 --mode pending --out strategy.ir.json

use quantforge_discover::{SearchFamily, generate_seed_for_family};
use quantforge_ir::EntryOrderPolicy;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut family_name = String::from("trend_pullback");
    let mut sequence = 0u64;
    let mut seed = 42u64;
    let mut mode = String::from("pending");
    let mut out = PathBuf::from("strategy.ir.json");

    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--family" => {
                i += 1;
                family_name = args.get(i).cloned().unwrap_or_default();
            }
            "--sequence" => {
                i += 1;
                sequence = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(sequence);
            }
            "--seed" => {
                i += 1;
                seed = args.get(i).and_then(|v| v.parse().ok()).unwrap_or(seed);
            }
            "--mode" => {
                i += 1;
                mode = args.get(i).cloned().unwrap_or(mode);
            }
            "--out" => {
                i += 1;
                out = PathBuf::from(args.get(i).cloned().unwrap_or_default());
            }
            other => {
                eprintln!("unknown arg: {other}");
                return ExitCode::FAILURE;
            }
        }
        i += 1;
    }

    let family = match family_name.as_str() {
        "trend_pullback" | "TrendPullback" => SearchFamily::TrendPullback,
        "momentum_burst" | "MomentumBurst" => SearchFamily::MomentumBurst,
        "donchian_breakout" | "DonchianBreakout" => SearchFamily::DonchianBreakout,
        "mean_reversion_band" | "MeanReversionBand" => SearchFamily::MeanReversionBand,
        "zscore_reversion" | "ZScoreReversion" => SearchFamily::ZScoreReversion,
        "session_orb" | "SessionOrb" => SearchFamily::SessionOrb,
        "impulse_candle" | "ImpulseCandle" => SearchFamily::ImpulseCandle,
        "vol_squeeze_break" | "VolSqueezeBreak" => SearchFamily::VolSqueezeBreak,
        "supply_demand_reclaim" | "SupplyDemandReclaim" => SearchFamily::SupplyDemandReclaim,
        "sweep_reclaim" | "SweepReclaim" => SearchFamily::SweepReclaim,
        "universal" | "UniversalGrammar" => SearchFamily::Universal,
        _ => {
            eprintln!("unknown family: {family_name}");
            return ExitCode::FAILURE;
        }
    };

    let mut strategy = generate_seed_for_family(seed, sequence, family);
    // Parity windows need enough fills; institutional once/day is too sparse on 1y.
    strategy.manage.max_one_entry_per_day = false;
    strategy.manage.flatten_end_of_day = false;

    match mode.as_str() {
        "pending" => {
            if matches!(strategy.entry.order, EntryOrderPolicy::Market) {
                eprintln!("seed unexpectedly Market; wanted pending");
                return ExitCode::FAILURE;
            }
        }
        "market" => {
            strategy.entry.order = EntryOrderPolicy::Market;
        }
        other => {
            eprintln!("unknown mode: {other} (expected pending|market)");
            return ExitCode::FAILURE;
        }
    }

    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let json = serde_json::to_string_pretty(&strategy).expect("serialize strategy");
    fs::write(&out, json).expect("write strategy");
    eprintln!(
        "wrote {} family={} mode={mode} order={:?}",
        out.display(),
        family.label(),
        strategy.entry.order
    );
    ExitCode::SUCCESS
}
