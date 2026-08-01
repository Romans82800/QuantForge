use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, SymbolSpecification, TradeMode};
use quantforge_core::{ContentHash, FloatPolicy, STRATEGY_IR_VERSION, stable_json_hash};
use quantforge_data::DataQualityReport;
use quantforge_discover::{
    BehaviorDescriptor, Databank, DiscoverConfig, DiscoverTelemetry, Elite, EvidenceComponents,
    GateConfig, LongShortSkewBucket, NicheKey, ThreeLevelBucket, niche_label,
};
use quantforge_export_mql5::{Mql5ExportConfig, TesterConfig, generate_bundle};
use quantforge_ir::{
    BoolExpr, ComparisonOp, EntrySignals, ManagePolicy, NumericExpr, PriceField, ProtectiveStops,
    RiskPolicy, Side, StopLossPolicy, StrategyIr, StrategyMeta, TakeProfitPolicy,
};
use quantforge_parity::{
    DiffReport, IndicatorFieldReport, IndicatorParityConfig, IndicatorParityReport,
    IndicatorReferenceMetadata, Mt5TesterMetadata, ParityRun, ParityTolerances, compare_runs,
};
use quantforge_quality::{ChallengeReport, DataSplitPlan, SealedFinalReport};
use quantforge_storage::{RunManifest, RunRecipe};
use quantforge_tick::{JudgeConfig, JudgeResult, JudgeTelemetry};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[derive(Deserialize)]
struct SplitArtifact {
    plan: DataSplitPlan,
    data_quality: DataQualityReport,
}

#[derive(Deserialize)]
struct ChallengeArtifact {
    report: ChallengeReport,
}

#[derive(Deserialize)]
struct SealedArtifact {
    report: SealedFinalReport,
}

fn write_json(path: &Path, value: &impl serde::Serialize) {
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
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
        profile_name: "assembly-cli-fixture".into(),
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
        id: "assembly-cli-always-long".into(),
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
        exit_long: None,
        exit_short: None,
        filters: Vec::new(),
        side: Side::LongOnly,
        risk: RiskPolicy::FixedCurrency {
            amount: quantforge_discover::FIXED_RISK_PER_TRADE,
        },
        stops: ProtectiveStops {
            stop_loss: StopLossPolicy::FixedPoints { points: 1.0 },
            take_profit: TakeProfitPolicy::RiskMultiple { multiple: 1.0 },
        },
        manage: ManagePolicy::default(),
        meta: StrategyMeta {
            thesis_hint: "evidence assembly integration fixture".into(),
            complexity: 0,
            export_safe: true,
        },
    }
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

fn manifest(command: &str, recipe: RunRecipe) -> RunManifest {
    RunManifest::new(command, recipe).unwrap()
}

fn write_databank(
    path: &Path,
    strategy: &StrategyIr,
    broker_hash: &ContentHash,
    split: &SplitArtifact,
    challenge: &ChallengeReport,
) {
    let fingerprint = strategy
        .structural_fingerprint(FloatPolicy::default())
        .unwrap();
    let scout = challenge.config.scout.clone();
    let config = DiscoverConfig {
        initial_candidates: 1,
        batch_size: 1,
        correlation_threshold: 0.88,
        novelty_weight: 1.0,
        tournament_size: 1,
        structural_mutation_probability: 0.1,
        seed: 42,
        universal_grammar: Default::default(),
        run_mode: quantforge_discover::DiscoverRunMode::FullHarvest,
        early_stop_pot_elites: None,
        target_databank_elites: None,
        trial_budget_warning: quantforge_discover::TRIAL_BUDGET_WARNING,
        gates: GateConfig {
            minimum_trades: challenge.baseline.metrics.trade_count,
            maximum_drawdown_percent: challenge.baseline.metrics.max_drawdown_percent,
            minimum_return_percent: challenge.baseline.metrics.return_percent - 1.0,
            minimum_profit_factor: 0.0,
            minimum_recovery_factor: 0.0,
        },
        deposit_gates: GateConfig {
            minimum_trades: challenge.baseline.metrics.trade_count,
            maximum_drawdown_percent: challenge.baseline.metrics.max_drawdown_percent,
            minimum_return_percent: challenge.baseline.metrics.return_percent - 1.0,
            minimum_profit_factor: 0.0,
            minimum_recovery_factor: 0.0,
        },
        precision: quantforge_discover::PrecisionGateConfig {
            minimum_return_retention: 0.0,
        },
        search_ranges: quantforge_discover::SearchRangeProfile::default(),
        oos1_expectancy_retention: 0.0,
        require_m1_precision: false,
        simple_exits: false,
        allow_break_even: false,
        allow_trailing_stops: false,
        allow_partial_exits: false,
        allow_market_entries: true,
        allow_stop_entries: false,
        allow_limit_entries: false,
        allow_stop_limit_entries: false,
        flatten_at_22: false,
        end_of_day_hour: 23,
        max_one_entry_per_day: false,
        mutate_after_elites: 0,
        random_fill_fraction: 0.0,
        worker_threads: 1,
        require_m1_robustness: false,
        robustness_folds: 3,
        robustness_monte_carlo_trials: 50,
        robustness_neighborhood_samples: 2,
        robustness_perturbation_fraction: 0.20,
        robustness_parameter_change_probability: 0.5,
        minimum_neighborhood_survival_fraction: 0.0,
        calendar_year_folds: false,
        minimum_deflated_trade_sharpe: None,
        multi_symbol_minimum_pass: 0,
        scout,
    };
    let bucket = |value: f64, first: f64, second: f64| {
        if value < first {
            ThreeLevelBucket::Low
        } else if value < second {
            ThreeLevelBucket::Medium
        } else {
            ThreeLevelBucket::High
        }
    };
    let niche = NicheKey {
        entry_conditions: 3,
        trade_frequency: ThreeLevelBucket::High,
        hold_time: ThreeLevelBucket::Low,
        drawdown: bucket(challenge.baseline.metrics.max_drawdown_percent, 5.0, 15.0),
        win_rate: bucket(challenge.baseline.metrics.win_rate, 35.0, 55.0),
        long_short_skew: LongShortSkewBucket::LongHeavy,
    };
    let elite = Elite {
        strategy: strategy.clone(),
        structural_fingerprint: fingerprint.clone(),
        descriptor: BehaviorDescriptor {
            entry_conditions: 3,
            exit_conditions: 1,
            trades_per_1000_bars: 100.0,
            average_bars_held: 1.0,
            drawdown_percent: challenge.baseline.metrics.max_drawdown_percent,
            win_rate_percent: challenge.baseline.metrics.win_rate,
            long_short_skew: 1.0,
        },
        niche: niche.clone(),
        evidence: EvidenceComponents {
            return_component: 1.0,
            profit_factor_component: 0.0,
            trade_count_bonus: 0.0,
            drawdown_penalty: 0.0,
            complexity_penalty: 0.0,
            total: 1.0,
        },
        novelty: 1.0,
        complexity: 1,
        metrics: challenge.baseline.metrics.clone(),
        is_expectancy: challenge.baseline.metrics.expectancy,
        oos1_expectancy: None,
        oos1_expectancy_ratio: None,
        observed_trade_sharpe: None,
        expected_max_lucky_sharpe: None,
        deflated_trade_sharpe: None,
        multi_symbol_results: Vec::new(),
        gate_results: Vec::new(),
        robustness: None,
        equity_signature: Vec::new(),
        discovered_generation: 0,
    };
    let databank = Databank {
        schema_version: quantforge_discover::DATABANK_SCHEMA_VERSION,
        grammar_version: quantforge_discover::GRAMMAR_VERSION.into(),
        data_hash: split.plan.development.data_hash.clone(),
        execution_data_hash: split.plan.development.data_hash.clone(),
        broker_spec_hash: broker_hash.clone(),
        config: config.clone(),
        completed_generations: 1,
        evaluation_count: challenge.config.evaluations_touched,
        elites: vec![elite],
        coverage_map: BTreeMap::from([(niche_label(&niche), fingerprint)]),
        accepted_pool: Vec::new(),
        accepted_coverage_map: BTreeMap::new(),
        telemetry: DiscoverTelemetry::default(),
    };
    let run_manifest = manifest(
        "evolve",
        RunRecipe {
            data_hash: Some(databank.data_hash.clone()),
            broker_spec_hash: Some(broker_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: Some(config.seed),
            config: BTreeMap::from([("discover_config".into(), json!(&config))]),
            override_flags: Vec::new(),
        },
    );
    write_json(
        path,
        &json!({
            "manifest": run_manifest,
            "source": "fixture-development",
            "broker": "fixture-broker",
            "metadata_hash": null,
            "data_quality": split.data_quality,
            "coverage": 1,
            "qd_score": 1.0,
            "databank": databank
        }),
    );
}

fn write_judge(
    path: &Path,
    fingerprint: &ContentHash,
    broker_hash: &ContentHash,
    split: &SplitArtifact,
    challenge: &ChallengeReport,
) {
    let config = JudgeConfig {
        entry_window: quantforge_eval::EntryWindow::default(),
        initial_balance: challenge.config.scout.initial_balance,
        costs: challenge.config.scout.costs.clone(),
        allow_execution_gaps: false,
        indicator_engine: challenge.config.scout.indicator_engine,
    };
    let m1_hash = ContentHash::sha256("fixture-m1-validation");
    let combined_hash = stable_json_hash(&BTreeMap::from([
        ("decision", &split.plan.validation.data_hash),
        ("m1", &m1_hash),
    ]))
    .unwrap();
    let run_manifest = manifest(
        "judge",
        RunRecipe {
            data_hash: Some(combined_hash),
            broker_spec_hash: Some(broker_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                ("strategy_fingerprint".into(), json!(fingerprint)),
                ("judge_config".into(), json!(&config)),
                (
                    "decision_data_hash".into(),
                    json!(&split.plan.validation.data_hash),
                ),
                ("m1_data_hash".into(), json!(&m1_hash)),
                ("decision_quality".into(), json!(split.data_quality.grade)),
                ("m1_quality".into(), json!(split.data_quality.grade)),
            ]),
            override_flags: Vec::new(),
        },
    );
    let result = JudgeResult {
        engine: quantforge_tick::ENGINE_TIER.into(),
        decision_interval_ms: 900_000,
        execution_interval_ms: 60_000,
        trades: challenge.baseline.trades.clone(),
        equity: challenge.baseline.equity.clone(),
        metrics: challenge.baseline.metrics.clone(),
        telemetry: JudgeTelemetry::default(),
    };
    write_json(
        path,
        &json!({
            "manifest": run_manifest,
            "strategy_fingerprint": fingerprint,
            "decision_data_quality": split.data_quality,
            "m1_data_quality": split.data_quality,
            "result": result
        }),
    );
}

fn write_parity(
    path: &Path,
    strategy: &StrategyIr,
    broker: &SymbolSpecification,
    split: &SplitArtifact,
    challenge: &ChallengeReport,
) {
    let config = Mql5ExportConfig {
        entry_window_start_hour: 2,
        entry_window_end_hour: 19,
        expert_name: "AssemblyFixture".into(),
        expert_directory: "QuantForge".into(),
        timeframe: "M15".into(),
        magic: 42,
        deviation_points: 10,
        max_spread_points: challenge.config.scout.costs.max_spread_points,
        estimated_slippage_points_per_side: challenge
            .config
            .scout
            .costs
            .adverse_slippage_points_per_side,
        commission_per_lot_round_turn: challenge.config.scout.costs.commission_per_lot_round_turn,
        allow_live_trading_default: false,
        export_style: quantforge_export_mql5::ExportStyle::Sqx,
        tester: TesterConfig {
            from_date: None,
            to_date: None,
            deposit: challenge.config.scout.initial_balance,
            currency: broker.account_currency.clone(),
            leverage: 100,
            model: 1,
        },
    };
    let bundle = generate_bundle(strategy, broker, &config).unwrap();
    let reference = ParityRun::from_scout(&challenge.baseline);
    let mut external = reference.clone();
    external.engine = "mt5-strategy-tester".into();
    let tolerances = ParityTolerances::default();
    let report: DiffReport =
        compare_runs(&reference, &external, &bundle.evidence, tolerances).unwrap();
    let metadata = Mt5TesterMetadata {
        properties: BTreeMap::from([
            (
                "strategy_fingerprint".into(),
                bundle.evidence.strategy_fingerprint.to_string(),
            ),
            (
                "broker_spec_hash".into(),
                bundle.evidence.broker_spec_hash.to_string(),
            ),
            ("symbol".into(), broker.symbol.clone()),
            ("timeframe".into(), "PERIOD_M15".into()),
            ("terminal_build".into(), "5000".into()),
            ("server".into(), "Fixture-Server".into()),
            ("magic".into(), bundle.evidence.config.magic.to_string()),
            (
                "deviation_points".into(),
                bundle.evidence.config.deviation_points.to_string(),
            ),
            (
                "max_spread_points".into(),
                bundle
                    .evidence
                    .config
                    .max_spread_points
                    .unwrap_or(0.0)
                    .to_string(),
            ),
            (
                "estimated_slippage_points_per_side".into(),
                bundle
                    .evidence
                    .config
                    .estimated_slippage_points_per_side
                    .to_string(),
            ),
            (
                "commission_per_lot_round_turn".into(),
                bundle
                    .evidence
                    .config
                    .commission_per_lot_round_turn
                    .to_string(),
            ),
            (
                "initial_deposit".into(),
                bundle.evidence.config.tester.deposit.to_string(),
            ),
        ]),
    };
    let run_manifest = manifest(
        "parity",
        RunRecipe {
            data_hash: Some(split.plan.validation.data_hash.clone()),
            broker_spec_hash: Some(bundle.evidence.broker_spec_hash.clone()),
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                (
                    "strategy_fingerprint".into(),
                    json!(&bundle.evidence.strategy_fingerprint),
                ),
                ("source_hash".into(), json!(&bundle.evidence.source_hash)),
                (
                    "protocol".into(),
                    json!(quantforge_parity::PARITY_PROTOCOL_VERSION),
                ),
            ]),
            override_flags: Vec::new(),
        },
    );
    write_json(
        path,
        &json!({
            "manifest": run_manifest,
            "evidence": bundle.evidence,
            "reference": reference,
            "external": external,
            "mt5_metadata": metadata,
            "report": report
        }),
    );
}

fn write_indicator_parity(path: &Path, broker: &SymbolSpecification) {
    let config = IndicatorParityConfig {
        warmup_rows: 1_000,
        absolute_epsilon: 1.0e-10,
        relative_epsilon: 1.0e-9,
    };
    let field = IndicatorFieldReport {
        passed: true,
        compared_rows: 1,
        mismatch_count: 0,
        max_absolute_error: 0.0,
        max_relative_error: 0.0,
        first_mismatch_row: None,
        first_mismatch_timestamp_ms: None,
    };
    let indicators = [
        "sma",
        "ema",
        "wma",
        "rsi",
        "atr",
        "donchian_high",
        "donchian_low",
        "highest_close",
        "lowest_close",
        "standard_deviation",
        "zscore",
        "percentile_in_range",
        "rate_of_change",
    ]
    .into_iter()
    .map(|name| (name.into(), field.clone()))
    .collect();
    let report = IndicatorParityReport {
        protocol_version: quantforge_parity::INDICATOR_PARITY_PROTOCOL_VERSION.into(),
        passed: true,
        reference_hash: ContentHash::sha256("fixture-indicator-reference"),
        metadata: IndicatorReferenceMetadata {
            terminal_build: 5000,
            broker: "Fixture Broker".into(),
            server: "Fixture-Server".into(),
            symbol: broker.symbol.clone(),
            timeframe: "PERIOD_M15".into(),
            period: 14,
        },
        source_rows: 1_001,
        compared_rows: 1,
        config: config.clone(),
        indicators,
    };
    let run_manifest = manifest(
        "indicator-parity",
        RunRecipe {
            data_hash: Some(report.reference_hash.clone()),
            broker_spec_hash: None,
            grammar_version: Some(quantforge_discover::GRAMMAR_VERSION.into()),
            seed: None,
            config: BTreeMap::from([
                (
                    "protocol".into(),
                    json!(quantforge_parity::INDICATOR_PARITY_PROTOCOL_VERSION),
                ),
                (
                    "terminal_build".into(),
                    json!(report.metadata.terminal_build),
                ),
                ("tolerances".into(), json!(&config)),
            ]),
            override_flags: Vec::new(),
        },
    );
    write_json(
        path,
        &json!({
            "manifest": run_manifest,
            "report": report
        }),
    );
}

struct AssemblyInputs<'a> {
    strategy: &'a Path,
    broker: &'a Path,
    split: &'a Path,
    databank: &'a Path,
    challenge: &'a Path,
    judge: &'a Path,
    parity: &'a Path,
    indicator: &'a Path,
    sealed: &'a Path,
    incubation: &'a Path,
}

fn assembled_command<'a>(inputs: &AssemblyInputs<'a>, out: &'a Path) -> Vec<&'a str> {
    vec![
        "assemble-evidence",
        "--strategy",
        inputs.strategy.to_str().unwrap(),
        "--broker",
        inputs.broker.to_str().unwrap(),
        "--split-plan",
        inputs.split.to_str().unwrap(),
        "--databank",
        inputs.databank.to_str().unwrap(),
        "--challenge",
        inputs.challenge.to_str().unwrap(),
        "--judge",
        inputs.judge.to_str().unwrap(),
        "--parity",
        inputs.parity.to_str().unwrap(),
        "--indicator-parity",
        inputs.indicator.to_str().unwrap(),
        "--sealed-final",
        inputs.sealed.to_str().unwrap(),
        "--incubation",
        inputs.incubation.to_str().unwrap(),
        "--out-dir",
        out.to_str().unwrap(),
    ]
}

#[test]
fn assembler_rejects_semantic_tampering_and_bundle_detects_later_byte_changes() {
    let directory = tempfile::tempdir().unwrap();
    let data_path = directory.path().join("bars.tsv");
    let strategy_path = directory.path().join("strategy.json");
    let broker_path = directory.path().join("broker.json");
    let split_path = directory.path().join("split.json");
    let challenge_path = directory.path().join("challenge.json");
    let databank_path = directory.path().join("databank.json");
    let judge_path = directory.path().join("judge.json");
    let parity_path = directory.path().join("parity.json");
    let indicator_path = directory.path().join("indicator.json");
    let sealed_root = directory.path().join("sealed");
    let incubation_root = directory.path().join("incubation");
    let strategy = strategy();
    let broker = broker();
    write_market_data(&data_path, 500);
    write_json(&strategy_path, &strategy);
    write_json(&broker_path, &broker);

    assert_success(&run(&[
        "split-plan",
        data_path.to_str().unwrap(),
        "--source-timezone",
        "Etc/UTC",
        "--out",
        split_path.to_str().unwrap(),
    ]));
    assert_success(&run(&[
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
    ]));
    assert_success(&run(&[
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
    ]));

    let split: SplitArtifact = serde_json::from_slice(&fs::read(&split_path).unwrap()).unwrap();
    let challenge: ChallengeArtifact =
        serde_json::from_slice(&fs::read(&challenge_path).unwrap()).unwrap();
    let broker_hash = broker.content_hash().unwrap();
    let fingerprint = strategy
        .structural_fingerprint(FloatPolicy::default())
        .unwrap();
    let split_hash = split.plan.content_hash().unwrap();
    assert_success(&run(&[
        "incubation-start",
        "--strategy",
        strategy_path.to_str().unwrap(),
        "--broker",
        broker_path.to_str().unwrap(),
        "--split-plan",
        split_path.to_str().unwrap(),
        "--root",
        incubation_root.to_str().unwrap(),
        "--start-date",
        "2026-01-01",
        "--initial-balance",
        "100",
        "--minimum-observation-days",
        "2",
        "--minimum-total-trades",
        "2",
        "--maximum-consecutive-zero-trade-days",
        "1",
    ]));
    let incubation_start = incubation_root
        .join(fingerprint.as_str())
        .join(split_hash.as_str())
        .join("incubation-start.json");
    for (date, balance) in [("2026-01-01", "101"), ("2026-01-02", "102")] {
        assert_success(&run(&[
            "incubation-record",
            "--start",
            incubation_start.to_str().unwrap(),
            "--date",
            date,
            "--ending-balance",
            balance,
            "--maximum-drawdown-percent",
            "1",
            "--trade-count",
            "1",
        ]));
    }
    assert_success(&run(&[
        "incubation-finalize",
        "--start",
        incubation_start.to_str().unwrap(),
    ]));
    let incubation_final = incubation_start
        .parent()
        .unwrap()
        .join("incubation-final.json");
    assert!(incubation_final.is_file());
    assert!(
        !run(&[
            "incubation-record",
            "--start",
            incubation_start.to_str().unwrap(),
            "--date",
            "2026-01-03",
            "--ending-balance",
            "103",
            "--maximum-drawdown-percent",
            "1",
            "--trade-count",
            "1",
        ])
        .status
        .success()
    );
    assert!(
        !run(&[
            "incubation-finalize",
            "--start",
            incubation_start.to_str().unwrap(),
        ])
        .status
        .success()
    );
    write_databank(
        &databank_path,
        &strategy,
        &broker_hash,
        &split,
        &challenge.report,
    );
    write_judge(
        &judge_path,
        &fingerprint,
        &broker_hash,
        &split,
        &challenge.report,
    );
    write_parity(&parity_path, &strategy, &broker, &split, &challenge.report);
    write_indicator_parity(&indicator_path, &broker);

    let sealed_path: PathBuf = fs::read_dir(&sealed_root)
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|entry| entry.file_type().unwrap().is_dir())
        .unwrap()
        .path()
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".sealed-final.json")
        })
        .unwrap();
    let sealed: SealedArtifact = serde_json::from_slice(&fs::read(&sealed_path).unwrap()).unwrap();
    assert!(sealed.report.passed);
    let assembly_inputs = AssemblyInputs {
        strategy: &strategy_path,
        broker: &broker_path,
        split: &split_path,
        databank: &databank_path,
        challenge: &challenge_path,
        judge: &judge_path,
        parity: &parity_path,
        indicator: &indicator_path,
        sealed: &sealed_path,
        incubation: &incubation_final,
    };

    let original_parity = fs::read(&parity_path).unwrap();
    let mut tampered_parity: Value = serde_json::from_slice(&original_parity).unwrap();
    tampered_parity["external"]["engine"] = json!("m1-judge");
    write_json(&parity_path, &tampered_parity);
    let bad_out = directory.path().join("bad-assembly");
    let rejected = run(&assembled_command(&assembly_inputs, &bad_out));
    assert!(!rejected.status.success());
    assert!(!bad_out.join("certification-bundle.json").exists());
    fs::write(&parity_path, original_parity).unwrap();

    let good_out = directory.path().join("assembled");
    assert_success(&run(&assembled_command(&assembly_inputs, &good_out)));
    let bundle_path = good_out.join("certification-bundle.json");
    assert!(good_out.join("validation-attestation.json").exists());
    assert!(good_out.join("certification-evidence.json").exists());
    assert!(bundle_path.exists());
    let assembled_evidence: Value =
        serde_json::from_slice(&fs::read(good_out.join("certification-evidence.json")).unwrap())
            .unwrap();
    assert_eq!(assembled_evidence["incubation"]["passed"], json!(true));

    let original_judge = fs::read(&judge_path).unwrap();
    let mut tampered_judge: Value = serde_json::from_slice(&original_judge).unwrap();
    tampered_judge["result"]["engine"] = json!("ohlc-scout");
    write_json(&judge_path, &tampered_judge);
    let vault = directory.path().join("vault");
    let changed_after_assembly = run(&[
        "certify",
        "--strategy",
        strategy_path.to_str().unwrap(),
        "--broker",
        broker_path.to_str().unwrap(),
        "--split-plan",
        split_path.to_str().unwrap(),
        "--bundle",
        bundle_path.to_str().unwrap(),
        "--vault",
        vault.to_str().unwrap(),
        "--require-incubation",
    ]);
    assert!(!changed_after_assembly.status.success());
    assert!(!vault.exists());
    fs::write(&judge_path, original_judge).unwrap();

    assert_success(&run(&[
        "certify",
        "--strategy",
        strategy_path.to_str().unwrap(),
        "--broker",
        broker_path.to_str().unwrap(),
        "--split-plan",
        split_path.to_str().unwrap(),
        "--bundle",
        bundle_path.to_str().unwrap(),
        "--vault",
        vault.to_str().unwrap(),
        "--require-incubation",
    ]));
    assert_eq!(
        fs::read_dir(vault.join(fingerprint.as_str()))
            .unwrap()
            .count(),
        1
    );

    let vault_entry = fs::read_dir(vault.join(fingerprint.as_str()))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let deployment = directory.path().join("deployment-pack");
    let deploy_args = [
        "deploy",
        "--vault-entry",
        vault_entry.to_str().unwrap(),
        "--out",
        deployment.to_str().unwrap(),
    ];
    assert_success(&run(&deploy_args));
    for relative in [
        "AssemblyFixture.mq5",
        "AssemblyFixture.set",
        "AssemblyFixture.tester.ini",
        "strategy.ir.json",
        "broker-spec.json",
        "export-evidence.json",
        "risk-pack.json",
        "CHANGELOG.md",
        "deployment-manifest.json",
    ] {
        assert!(deployment.join(relative).is_file(), "missing {relative}");
    }
    let deployment_manifest: Value =
        serde_json::from_slice(&fs::read(deployment.join("deployment-manifest.json")).unwrap())
            .unwrap();
    assert_eq!(deployment_manifest["grade"], json!("deployed"));
    assert_eq!(deployment_manifest["live_trading_default"], json!(false));
    let risk_pack: Value =
        serde_json::from_slice(&fs::read(deployment.join("risk-pack.json")).unwrap()).unwrap();
    assert_eq!(risk_pack["live_trading_default"], json!(false));
    assert!(risk_pack["incubation_artifact_hash"].is_string());
    assert_eq!(
        risk_pack["export_config"]["allow_live_trading_default"],
        json!(false)
    );
    assert!(
        fs::read_to_string(deployment.join("AssemblyFixture.set"))
            .unwrap()
            .contains("AllowLiveTrading=false")
    );
    assert!(!run(&deploy_args).status.success());

    let parity_before_deploy_tamper = fs::read(&parity_path).unwrap();
    let mut parity_after_certification: Value =
        serde_json::from_slice(&parity_before_deploy_tamper).unwrap();
    parity_after_certification["external"]["engine"] = json!("changed-after-certification");
    write_json(&parity_path, &parity_after_certification);
    let rejected_deployment = directory.path().join("rejected-deployment");
    let rejected = run(&[
        "deploy",
        "--vault-entry",
        vault_entry.to_str().unwrap(),
        "--out",
        rejected_deployment.to_str().unwrap(),
    ]);
    assert!(!rejected.status.success());
    assert!(!rejected_deployment.exists());

    fs::write(&parity_path, parity_before_deploy_tamper).unwrap();
    let original_incubation = fs::read(&incubation_final).unwrap();
    let mut tampered_incubation: Value = serde_json::from_slice(&original_incubation).unwrap();
    tampered_incubation["report"]["passed"] = json!(false);
    write_json(&incubation_final, &tampered_incubation);
    let rejected_incubation_deployment = directory.path().join("rejected-incubation-deployment");
    let rejected = run(&[
        "deploy",
        "--vault-entry",
        vault_entry.to_str().unwrap(),
        "--out",
        rejected_incubation_deployment.to_str().unwrap(),
    ]);
    assert!(!rejected.status.success());
    assert!(!rejected_incubation_deployment.exists());
}
