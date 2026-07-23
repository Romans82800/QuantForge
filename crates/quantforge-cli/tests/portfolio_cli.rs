use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, SymbolSpecification, TradeMode};
use quantforge_portfolio::{PORTFOLIO_PROTOCOL_VERSION, PortfolioReport};
use quantforge_storage::RunManifest;
use serde::Deserialize;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

#[derive(Deserialize)]
struct PortfolioArtifact {
    manifest: RunManifest,
    report: PortfolioReport,
}

fn broker() -> SymbolSpecification {
    SymbolSpecification {
        profile_name: "portfolio-cli-fixture".into(),
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

fn write_market_data(path: &Path, count: usize) {
    let mut output = String::from(
        "<DATE>\t<TIME>\t<OPEN>\t<HIGH>\t<LOW>\t<CLOSE>\t<TICKVOL>\t<VOL>\t<SPREAD>\n",
    );
    for index in 0..count {
        let hour = index / 60;
        let minute = index % 60;
        let cycle = (index % 40) as f64;
        let open = 100.0 + index as f64 * 0.2 + (cycle - 20.0).abs() * 0.05;
        writeln!(
            output,
            "2024.01.08\t{hour:02}:{minute:02}:00\t{open:.4}\t{:.4}\t{:.4}\t{:.4}\t100\t0\t0",
            open + 0.4,
            open - 0.4,
            open + if index % 2 == 0 { 0.15 } else { -0.10 }
        )
        .unwrap();
    }
    fs::write(path, output).unwrap();
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_quantforge"))
        .args(args)
        .output()
        .unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn portfolio_cli_packs_a_real_databank_and_refuses_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("bars.tsv");
    let broker_path = directory.path().join("broker.json");
    let databank = directory.path().join("databank.json");
    let portfolio = directory.path().join("portfolio.json");
    write_market_data(&data, 500);
    fs::write(&broker_path, serde_json::to_vec_pretty(&broker()).unwrap()).unwrap();

    assert_success(&run(&[
        "evolve",
        data.to_str().unwrap(),
        "--source-timezone",
        "Etc/UTC",
        "--broker",
        broker_path.to_str().unwrap(),
        "--databank",
        databank.to_str().unwrap(),
        "--generations",
        "1",
        "--initial",
        "80",
        "--batch",
        "20",
        "--minimum-trades",
        "1",
        "--maximum-drawdown-percent",
        "100",
        "--minimum-return-percent=-100",
        "--minimum-profit-factor",
        "0",
        "--commission-per-lot-round-turn",
        "0",
        "--initial-balance",
        "100",
    ]));

    let portfolio_args = [
        "portfolio",
        databank.to_str().unwrap(),
        "--broker",
        broker_path.to_str().unwrap(),
        "--maximum-pairwise-correlation",
        "1",
        "--maximum-weight-per-strategy",
        "1",
        "--maximum-family-exposure",
        "1",
        "--maximum-strategies",
        "1",
        "--minimum-return-percent=-100",
        "--stress-trials",
        "50",
        "--stress-block-length",
        "3",
        "--out",
        portfolio.to_str().unwrap(),
    ];
    assert_success(&run(&portfolio_args));
    let artifact: PortfolioArtifact =
        serde_json::from_slice(&fs::read(&portfolio).unwrap()).unwrap();
    artifact.manifest.validate().unwrap();
    assert_eq!(artifact.manifest.command, "portfolio");
    assert_eq!(artifact.report.protocol_version, PORTFOLIO_PROTOCOL_VERSION);
    assert_eq!(artifact.report.selected.len(), 1);
    assert_eq!(artifact.report.selected[0].weight, 1.0);
    assert_eq!(artifact.report.stress.trial_results.len(), 50);

    let repeated = run(&portfolio_args);
    assert!(!repeated.status.success());
}
