use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, SymbolSpecification, TradeMode};
use quantforge_core::STRATEGY_IR_VERSION;
use quantforge_ir::{
    BoolExpr, ComparisonOp, EntrySignals, ManagePolicy, NumericExpr, PriceField, ProtectiveStops,
    RiskPolicy, Side, StopLossPolicy, StrategyIr, StrategyMeta, TakeProfitPolicy,
};
use quantforge_quality::{ChallengeReport, SealedFinalReport, SelectionBiasLevel};
use serde::Deserialize;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Deserialize)]
struct ChallengeArtifact {
    report: ChallengeReport,
}

#[derive(Deserialize)]
struct SealedArtifact {
    report: SealedFinalReport,
}

fn write_market_data(path: &Path, count: usize) {
    let mut output = String::from(
        "<DATE>\t<TIME>\t<OPEN>\t<HIGH>\t<LOW>\t<CLOSE>\t<TICKVOL>\t<VOL>\t<SPREAD>\n",
    );
    for index in 0..count {
        let hour = index / 60;
        let minute = index % 60;
        let open = 100.0 + index as f64 * 2.0;
        writeln!(
            output,
            "2024.01.08\t{hour:02}:{minute:02}:00\t{open:.2}\t{:.2}\t{:.2}\t{:.2}\t100\t0\t0",
            open + 2.0,
            open - 0.1,
            open + 1.0
        )
        .unwrap();
    }
    fs::write(path, output).unwrap();
}

fn broker() -> SymbolSpecification {
    SymbolSpecification {
        profile_name: "challenge-cli-fixture".into(),
        symbol: "TEST".into(),
        digits: 2,
        point: 1.0,
        tick_size: 1.0,
        tick_value: 1.0,
        contract_size: 1.0,
        volume_min: 0.01,
        volume_step: 0.01,
        volume_max: 100.0,
        stops_level_points: 0,
        freeze_level_points: 0,
        filling_modes: vec![FillingMode::FillOrKill],
        trade_mode: TradeMode::Full,
        margin_initial_per_lot: None,
        swap_mode: SwapMode::Disabled,
        swap_long: 0.0,
        swap_short: 0.0,
        triple_swap_day: DayOfWeek::Wednesday,
        swap_multipliers: Vec::new(),
        sessions: Vec::new(),
        timezone: "Etc/UTC".into(),
        account_currency: "USD".into(),
        base_currency: "USD".into(),
        profit_currency: "USD".into(),
        margin_currency: "USD".into(),
        synthetic_spreads: Vec::new(),
    }
}

fn strategy() -> StrategyIr {
    StrategyIr {
        id: "challenge-cli-always-long".into(),
        version: STRATEGY_IR_VERSION,
        entry: EntrySignals {
            long: Some(BoolExpr::Compare {
                comparison: ComparisonOp::GreaterThan,
                left: NumericExpr::Price {
                    field: PriceField::Close,
                    shift: 1,
                },
                right: NumericExpr::Constant { value: 0.0 },
            }),
            short: None,
            order: Default::default(),
        },
        exit: None,
        filters: Vec::new(),
        side: Side::LongOnly,
        risk: RiskPolicy::FixedCurrency { amount: 1.0 },
        stops: ProtectiveStops {
            stop_loss: StopLossPolicy::FixedPoints { points: 1.0 },
            take_profit: TakeProfitPolicy::RiskMultiple { multiple: 1.0 },
        },
        manage: ManagePolicy::default(),
        meta: StrategyMeta {
            thesis_hint: "CLI Challenge integration fixture".into(),
            complexity: 0,
            export_safe: true,
        },
    }
}

#[test]
fn challenge_and_one_shot_sealed_final_form_a_machine_readable_sequence() {
    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("bars.tsv");
    let strategy_path = directory.path().join("strategy.json");
    let broker_path = directory.path().join("broker.json");
    let split_path = directory.path().join("split.json");
    let challenge_path = directory.path().join("challenge.json");
    write_market_data(&data_path, 500);
    fs::write(
        &strategy_path,
        serde_json::to_vec_pretty(&strategy()).unwrap(),
    )
    .unwrap();
    fs::write(&broker_path, serde_json::to_vec_pretty(&broker()).unwrap()).unwrap();

    let split = Command::new(env!("CARGO_BIN_EXE_quantforge"))
        .args([
            "split-plan",
            data_path.to_str().unwrap(),
            "--source-timezone",
            "Etc/UTC",
            "--out",
            split_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        split.status.success(),
        "{}",
        String::from_utf8_lossy(&split.stderr)
    );

    let challenge = Command::new(env!("CARGO_BIN_EXE_quantforge"))
        .args([
            "challenge",
            data_path.to_str().unwrap(),
            "--source-timezone",
            "Etc/UTC",
            "--strategy",
            strategy_path.to_str().unwrap(),
            "--broker",
            broker_path.to_str().unwrap(),
            "--split-plan",
            split_path.to_str().unwrap(),
            "--evaluations-touched",
            "2000",
            "--folds",
            "4",
            "--purge-bars",
            "5",
            "--embargo-bars",
            "5",
            "--minimum-validation-bars",
            "50",
            "--minimum-baseline-trades",
            "20",
            "--minimum-fold-trades",
            "5",
            "--monte-carlo-trials",
            "100",
            "--neighborhood-samples",
            "8",
            "--commission-per-lot-round-turn",
            "0",
            "--initial-balance",
            "100",
            "--out",
            challenge_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        challenge.status.success(),
        "{}",
        String::from_utf8_lossy(&challenge.stderr)
    );

    let artifact: ChallengeArtifact =
        serde_json::from_slice(&fs::read(&challenge_path).unwrap()).unwrap();
    assert!(artifact.report.passed);
    assert_eq!(artifact.report.validation_bar_count, 100);
    assert_eq!(artifact.report.purged_folds.len(), 4);
    assert_eq!(artifact.report.cost_shocks.points.len(), 4);
    assert_eq!(artifact.report.monte_carlo.trials, 100);
    assert_eq!(artifact.report.parameter_neighborhood.neighbors.len(), 8);
    assert_eq!(
        artifact.report.multiple_testing.warning_level,
        SelectionBiasLevel::Elevated
    );

    let sealed_root = directory.path().join("sealed");
    let run_sealed = || {
        Command::new(env!("CARGO_BIN_EXE_quantforge"))
            .args([
                "sealed-final",
                data_path.to_str().unwrap(),
                "--source-timezone",
                "Etc/UTC",
                "--strategy",
                strategy_path.to_str().unwrap(),
                "--broker",
                broker_path.to_str().unwrap(),
                "--split-plan",
                split_path.to_str().unwrap(),
                "--challenge",
                challenge_path.to_str().unwrap(),
                "--sealed-root",
                sealed_root.to_str().unwrap(),
                "--commission-per-lot-round-turn",
                "0",
                "--initial-balance",
                "100",
            ])
            .output()
            .unwrap()
    };
    let sealed = run_sealed();
    assert!(
        sealed.status.success(),
        "{}",
        String::from_utf8_lossy(&sealed.stderr)
    );
    let candidate_directory = fs::read_dir(&sealed_root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_type().unwrap().is_dir())
        .unwrap()
        .path();
    let files: Vec<_> = fs::read_dir(&candidate_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 2);
    let sealed_path = files
        .iter()
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".sealed-final.json")
        })
        .unwrap();
    let sealed_artifact: SealedArtifact =
        serde_json::from_slice(&fs::read(sealed_path).unwrap()).unwrap();
    assert!(sealed_artifact.report.passed);
    assert!(sealed_artifact.report.shortlisted_before_open);
    assert!(!sealed_artifact.report.used_in_selection_score);

    let repeated = run_sealed();
    assert!(!repeated.status.success());
    assert_eq!(fs::read_dir(candidate_directory).unwrap().count(), 2);
}
