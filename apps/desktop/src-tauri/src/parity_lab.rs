use crate::data_lab::{display_path, load_bound_broker, load_data_source, build_decision_from_m1};
use crate::workflow::{
    ChallengeArtifact, IndicatorParityArtifact, JudgeArtifact, ParityArtifact, ScoutArtifactInput,
    ensure_new, manifest, read_json, recipe_path, write_json_new, write_text_new,
};
use quantforge_data::{DataQualityReport, QualityGrade};
use quantforge_eval::CostModel;
use quantforge_export_mql5::{Mql5ExportConfig, TesterConfig, generate_bundle};
use quantforge_ir::StrategyIr;
use quantforge_parity::{
    ParityRun, ParityTolerances, compare_runs, load_mt5_tester_metadata,
    load_mt5_tester_run_in_timezone,
};
use quantforge_tick::{JudgeConfig, evaluate_strategy_m1};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeRequest {
    decision_data_path: String,
    decision_metadata_path: Option<String>,
    decision_source_timezone: Option<String>,
    m1_data_path: String,
    m1_metadata_path: Option<String>,
    m1_source_timezone: Option<String>,
    split_plan_path: Option<String>,
    strategy_path: String,
    broker_path: String,
    output_path: String,
    commission_per_lot_round_turn: f64,
    slippage_points_per_side: f64,
    fallback_spread_points: Option<f64>,
    max_spread_points: Option<f64>,
    initial_balance: f64,
    /// Broker-local hour from which entries may be placed (inclusive).
    entry_window_start_hour: Option<u32>,
    /// Broker-local hour from which entries stop being placed (exclusive).
    entry_window_end_hour: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeView {
    output_path: String,
    grade: &'static str,
    trades: usize,
    return_percent: f64,
    profit_factor: Option<f64>,
    maximum_drawdown_percent: f64,
    decision_bars: usize,
    m1_bars: usize,
    pending_orders_filled: usize,
    partial_exits: usize,
    break_even_moves: usize,
    trailing_moves: usize,
    end_of_day_flattens: usize,
    verified_no_tick_gap_events: usize,
    verified_no_tick_minutes: usize,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    strategy_path: String,
    broker_path: String,
    output_directory: String,
    expert_name: String,
    expert_directory: String,
    timeframe: String,
    magic: u64,
    deviation_points: u32,
    max_spread_points: Option<f64>,
    slippage_points_per_side: f64,
    commission_per_lot_round_turn: f64,
    deposit: f64,
    currency: String,
    leverage: u32,
    tester_model: u8,
    /// Broker-local hour from which entries may be placed (inclusive).
    entry_window_start_hour: Option<u32>,
    /// Broker-local hour from which entries stop being placed (exclusive).
    entry_window_end_hour: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportView {
    output_directory: String,
    source_path: String,
    settings_path: String,
    tester_path: String,
    evidence_path: String,
    strategy_fingerprint: String,
    source_hash: String,
    symbol: String,
    timeframe: String,
    live_trading_default: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityRequest {
    reference_path: String,
    evidence_path: String,
    mq5_path: String,
    mt5_deals_path: String,
    mt5_equity_path: String,
    mt5_metadata_path: String,
    /// Same timezone token used for bar ingestion (e.g. ICMarkets/EST+7).
    #[serde(default)]
    broker_timezone: Option<String>,
    output_path: String,
    initial_balance: f64,
    trade_count_relative: f64,
    trade_count_absolute: usize,
    net_profit_relative: f64,
    max_drawdown_relative: f64,
    max_equity_divergence_percent: f64,
    trade_timestamp_tolerance_ms: i64,
    minimum_aligned_trade_fraction: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParityView {
    output_path: String,
    passed: bool,
    grade: &'static str,
    reference_engine: String,
    external_engine: String,
    reference_trades: usize,
    external_trades: usize,
    aligned_trades: usize,
    required_aligned_trades: usize,
    net_profit_delta_relative: f64,
    drawdown_delta_relative: f64,
    equity_divergence_percent: f64,
    protective_orders_present: bool,
    reference_win_rate: f64,
    external_win_rate: f64,
    reference_winning_trades: usize,
    external_winning_trades: usize,
    reference_profit_factor: Option<f64>,
    external_profit_factor: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorParityRequest {
    reference_path: String,
    output_path: String,
    warmup_rows: usize,
    absolute_epsilon: f64,
    relative_epsilon: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorParityView {
    output_path: String,
    passed: bool,
    symbol: String,
    timeframe: String,
    source_rows: usize,
    compared_rows: usize,
    field_count: usize,
    mismatch_count: usize,
}

#[tauri::command]
pub async fn run_m1_judge(request: JudgeRequest) -> Result<JudgeView, String> {
    tauri::async_runtime::spawn_blocking(move || run_m1_judge_sync(&request))
        .await
        .map_err(|error| format!("M1 Judge task failed: {error}"))?
}

fn run_m1_judge_sync(request: &JudgeRequest) -> Result<JudgeView, String> {
    let out = ensure_new(&request.output_path, "Judge artifact")?;
    let decision = load_data_source(
        &request.decision_data_path,
        request.decision_metadata_path.as_deref(),
        request.decision_source_timezone.as_deref(),
    )?;
    let m1 = load_data_source(
        &request.m1_data_path,
        request.m1_metadata_path.as_deref(),
        request.m1_source_timezone.as_deref(),
    )?;
    let built_decision = build_decision_from_m1(&m1.dataset, Some(&decision.dataset))?;
    let decision_dataset = request
        .split_plan_path
        .as_deref()
        .map(|path| validation_partition(&built_decision, path))
        .transpose()?
        .unwrap_or(built_decision);
    let decision_quality = DataQualityReport::analyze(&decision_dataset);
    let m1_quality = DataQualityReport::analyze(&m1.dataset);
    if decision_quality.grade == QualityGrade::Fail || m1_quality.grade == QualityGrade::Fail {
        return Err(format!(
            "Judge input quality failed (decision={:?}, M1={:?})",
            decision_quality.grade, m1_quality.grade
        ));
    }
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
    let broker = load_bound_broker(&request.broker_path, decision.metadata.as_ref())?;
    load_bound_broker(&request.broker_path, m1.metadata.as_ref())?;
    let strategy_fingerprint = strategy
        .structural_fingerprint(quantforge_core::FloatPolicy::default())
        .map_err(|error| error.to_string())?;
    let broker_hash = broker.content_hash().map_err(|error| error.to_string())?;
    let config = JudgeConfig {
        initial_balance: request.initial_balance,
        costs: CostModel {
            fallback_spread_points: request.fallback_spread_points,
            adverse_slippage_points_per_side: request.slippage_points_per_side,
            commission_per_lot_round_turn: request.commission_per_lot_round_turn,
            max_spread_points: request.max_spread_points,
            include_costs_in_risk: true,
            fill_simulation: Default::default(),
        },
        allow_execution_gaps: false,
        indicator_engine: quantforge_eval::IndicatorEngine::Sqx,
        entry_window: crate::discover::entry_window(
            request.entry_window_start_hour,
            request.entry_window_end_hour,
        ),
    };
    let result = evaluate_strategy_m1(&strategy, &decision_dataset, &m1.dataset, &broker, &config)
        .map_err(|error| error.to_string())?;
    let combined_data_hash = quantforge_core::stable_json_hash(&BTreeMap::from([
        ("decision", &decision_dataset.data_hash),
        ("m1", &m1.dataset.data_hash),
    ]))
    .map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "judge",
        Some(combined_data_hash),
        Some(broker_hash),
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            (
                "decision_source".into(),
                recipe_path(&request.decision_data_path),
            ),
            ("m1_source".into(), recipe_path(&request.m1_data_path)),
            ("strategy".into(), recipe_path(&request.strategy_path)),
            ("broker".into(), recipe_path(&request.broker_path)),
            (
                "split_plan".into(),
                request
                    .split_plan_path
                    .as_deref()
                    .map(recipe_path)
                    .unwrap_or(serde_json::Value::Null),
            ),
            ("strategy_fingerprint".into(), json!(&strategy_fingerprint)),
            (
                "judge_config".into(),
                serde_json::to_value(&config).map_err(|error| error.to_string())?,
            ),
            (
                "decision_data_hash".into(),
                json!(&decision_dataset.data_hash),
            ),
            ("m1_data_hash".into(), json!(&m1.dataset.data_hash)),
            ("decision_quality".into(), json!(decision_quality.grade)),
            ("m1_quality".into(), json!(m1_quality.grade)),
        ]),
    )?;
    let artifact = JudgeArtifact {
        manifest: run_manifest,
        strategy_fingerprint,
        decision_source: display_path(Path::new(&request.decision_data_path)),
        m1_source: display_path(Path::new(&request.m1_data_path)),
        strategy: display_path(Path::new(&request.strategy_path)),
        broker: display_path(Path::new(&request.broker_path)),
        decision_metadata_hash: decision.metadata.map(|metadata| metadata.metadata_hash),
        m1_metadata_hash: m1.metadata.map(|metadata| metadata.metadata_hash),
        decision_data_quality: decision_quality,
        m1_data_quality: m1_quality,
        result,
    };
    write_json_new(&out, &artifact)?;
    let telemetry = &artifact.result.telemetry;
    Ok(JudgeView {
        output_path: display_path(&out),
        grade: "accepted",
        trades: artifact.result.metrics.trade_count,
        return_percent: artifact.result.metrics.return_percent,
        profit_factor: artifact.result.metrics.profit_factor,
        maximum_drawdown_percent: artifact.result.metrics.max_drawdown_percent,
        decision_bars: telemetry.decision_bars_replayed,
        m1_bars: telemetry.m1_bars_replayed,
        pending_orders_filled: telemetry.pending_orders_filled,
        partial_exits: telemetry.partial_exits_executed,
        break_even_moves: telemetry.break_even_moves,
        trailing_moves: telemetry.trailing_stop_moves,
        end_of_day_flattens: telemetry.end_of_day_flattens,
        verified_no_tick_gap_events: telemetry.verified_no_tick_gap_events,
        verified_no_tick_minutes: telemetry.verified_no_tick_minutes,
    })
}

fn validation_partition(
    dataset: &quantforge_data::BarDataset,
    split_path: &str,
) -> Result<quantforge_data::BarDataset, String> {
    let split: crate::workflow::SplitPlanArtifact = read_json(split_path)?;
    crate::workflow::verify_split(&split)?;
    if split.plan.full_data_hash != dataset.data_hash {
        return Err("decision source does not match the selected split plan".into());
    }
    let segment = &split.plan.validation;
    let bars: Vec<_> = dataset
        .bars
        .iter()
        .filter(|bar| {
            bar.timestamp_ms >= segment.start_timestamp_ms
                && bar.timestamp_ms < segment.end_timestamp_ms_exclusive
        })
        .cloned()
        .collect();
    if bars.len() != segment.bar_count
        || quantforge_data::bar_content_hash(&bars) != segment.data_hash
    {
        return Err("validation bars do not reproduce the split-plan segment".into());
    }
    Ok(quantforge_data::BarDataset {
        data_hash: segment.data_hash.clone(),
        source_rows: bars.len(),
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: dataset.delimiter,
        source_timezone: dataset.source_timezone.clone(),
        bars,
    })
}

#[tauri::command]
pub async fn export_mql5(request: ExportRequest) -> Result<ExportView, String> {
    tauri::async_runtime::spawn_blocking(move || export_mql5_sync(&request))
        .await
        .map_err(|error| format!("MQL5 export task failed: {error}"))?
}

fn export_mql5_sync(request: &ExportRequest) -> Result<ExportView, String> {
    let out = PathBuf::from(&request.output_directory);
    if out.exists() {
        return Err(format!(
            "export directory already exists and will not be replaced: {}",
            out.display()
        ));
    }
    let strategy: StrategyIr = read_json(&request.strategy_path)?;
    let broker = load_bound_broker(&request.broker_path, None)?;
    let window = crate::discover::entry_window(
        request.entry_window_start_hour,
        request.entry_window_end_hour,
    );
    // A blank name means "derive one", so exporting several strategies in a row
    // no longer produces a folder of identically named experts.
    let expert_name = if request.expert_name.trim().is_empty() {
        quantforge_export_mql5::suggested_expert_name(&broker.symbol, &strategy.id, request.magic)
    } else {
        request.expert_name.trim().to_string()
    };
    let config = Mql5ExportConfig {
        expert_name: expert_name.clone(),
        expert_directory: request.expert_directory.clone(),
        timeframe: request.timeframe.clone(),
        magic: request.magic,
        deviation_points: request.deviation_points,
        max_spread_points: request.max_spread_points,
        estimated_slippage_points_per_side: request.slippage_points_per_side,
        commission_per_lot_round_turn: request.commission_per_lot_round_turn,
        allow_live_trading_default: false,
        export_style: quantforge_export_mql5::ExportStyle::Sqx,
        entry_window_start_hour: window.start_hour,
        entry_window_end_hour: window.end_hour,
        tester: TesterConfig {
            from_date: None,
            to_date: None,
            deposit: request.deposit,
            currency: request.currency.clone(),
            leverage: request.leverage,
            model: request.tester_model,
        },
    };
    let bundle = generate_bundle(&strategy, &broker, &config).map_err(|error| error.to_string())?;
    fs::create_dir_all(&out).map_err(|error| format!("cannot create export directory: {error}"))?;
    let source = out.join(format!("{expert_name}.mq5"));
    let settings = out.join(format!("{expert_name}.set"));
    let tester = out.join(format!("{expert_name}.tester.ini"));
    let evidence = out.join(format!("{expert_name}.evidence.json"));
    write_text_new(&source, &bundle.source)?;
    write_text_new(&settings, &bundle.set_file)?;
    write_text_new(&tester, &bundle.tester_ini)?;
    write_json_new(&evidence, &bundle.evidence)?;
    for support in &bundle.support_files {
        let path = out.join(&support.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create export support directory: {error}"))?;
        }
        write_text_new(&path, &support.contents)
            .map_err(|error| format!("cannot write export support file: {error}"))?;
    }
    Ok(ExportView {
        output_directory: display_path(&out),
        source_path: display_path(&source),
        settings_path: display_path(&settings),
        tester_path: display_path(&tester),
        evidence_path: display_path(&evidence),
        strategy_fingerprint: bundle.evidence.strategy_fingerprint.as_str().into(),
        source_hash: bundle.evidence.source_hash.as_str().into(),
        symbol: bundle.evidence.symbol,
        timeframe: bundle.evidence.timeframe,
        live_trading_default: bundle.evidence.live_trading_default,
    })
}

#[tauri::command]
pub async fn compare_external_parity(request: ParityRequest) -> Result<ParityView, String> {
    tauri::async_runtime::spawn_blocking(move || compare_external_parity_sync(&request))
        .await
        .map_err(|error| format!("external parity task failed: {error}"))?
}

fn compare_external_parity_sync(request: &ParityRequest) -> Result<ParityView, String> {
    let out = ensure_new(&request.output_path, "parity artifact")?;
    let reference_bytes = fs::read(&request.reference_path)
        .map_err(|error| format!("cannot read parity reference: {error}"))?;
    let (reference_manifest, reference_fingerprint, reference) = if let Ok(judge) =
        serde_json::from_slice::<JudgeArtifact>(&reference_bytes)
    {
        (
            judge.manifest,
            judge.strategy_fingerprint,
            ParityRun::from_judge(&judge.result),
        )
    } else if let Ok(scout) = serde_json::from_slice::<ScoutArtifactInput>(&reference_bytes) {
        (
            scout.manifest,
            scout.strategy_fingerprint,
            ParityRun::from_scout(&scout.result),
        )
    } else {
        let challenge: ChallengeArtifact = serde_json::from_slice(&reference_bytes)
            .map_err(|error| {
                format!("reference is neither Judge, Scout, nor Challenge JSON: {error}")
            })?;
        challenge
            .report
            .validate_integrity()
            .map_err(|error| error.to_string())?;
        (
            challenge.manifest,
            challenge.report.binding.strategy_fingerprint,
            ParityRun::from_scout(&challenge.report.baseline),
        )
    };
    let mut evidence: quantforge_export_mql5::ExportEvidenceCard =
        read_json(&request.evidence_path)?;
    if reference_fingerprint != evidence.strategy_fingerprint
        || reference_manifest.recipe.broker_spec_hash.as_ref() != Some(&evidence.broker_spec_hash)
    {
        return Err("reference result and export evidence are bound to different inputs".into());
    }
    let source = fs::read(&request.mq5_path)
        .map_err(|error| format!("cannot read generated MQL5 source: {error}"))?;
    if quantforge_core::ContentHash::sha256(&source) != evidence.source_hash {
        return Err("MQL5 source hash does not match its evidence card".into());
    }
    let source_text = String::from_utf8(source).map_err(|error| error.to_string())?;
    let protective_calls = source_text.contains("g_trade.Buy(volume,_Symbol,0.0,stop,target")
        && source_text.contains("g_trade.Sell(volume,_Symbol,0.0,stop,target");
    evidence.mandatory_stop_loss &= protective_calls;
    evidence.mandatory_take_profit &= protective_calls;
    let metadata =
        load_mt5_tester_metadata(&request.mt5_metadata_path).map_err(|error| error.to_string())?;
    metadata
        .validate_evidence(&evidence)
        .map_err(|error| error.to_string())?;
    let broker_timezone = request
        .broker_timezone
        .clone()
        .or_else(|| metadata.properties.get("broker_timezone").cloned());
    let external = load_mt5_tester_run_in_timezone(
        &request.mt5_deals_path,
        &request.mt5_equity_path,
        request.initial_balance,
        broker_timezone.as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let tolerances = ParityTolerances {
        trade_count_relative: request.trade_count_relative,
        trade_count_absolute: request.trade_count_absolute,
        net_profit_relative: request.net_profit_relative,
        max_drawdown_relative: request.max_drawdown_relative,
        max_equity_divergence_percent: request.max_equity_divergence_percent,
        trade_timestamp_tolerance_ms: request.trade_timestamp_tolerance_ms,
        minimum_aligned_trade_fraction: request.minimum_aligned_trade_fraction,
    };
    let report = compare_runs(&reference, &external, &evidence, tolerances)
        .map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "parity",
        reference_manifest.recipe.data_hash.clone(),
        Some(evidence.broker_spec_hash.clone()),
        reference_manifest.recipe.grammar_version.clone(),
        None,
        BTreeMap::from([
            ("scout_result".into(), recipe_path(&request.reference_path)),
            ("evidence".into(), recipe_path(&request.evidence_path)),
            ("mq5".into(), recipe_path(&request.mq5_path)),
            ("mt5_deals".into(), recipe_path(&request.mt5_deals_path)),
            ("mt5_equity".into(), recipe_path(&request.mt5_equity_path)),
            (
                "mt5_metadata".into(),
                recipe_path(&request.mt5_metadata_path),
            ),
            ("broker_timezone".into(), json!(&broker_timezone)),
            (
                "strategy_fingerprint".into(),
                json!(&evidence.strategy_fingerprint),
            ),
            ("source_hash".into(), json!(&evidence.source_hash)),
            (
                "protocol".into(),
                json!(quantforge_parity::PARITY_PROTOCOL_VERSION),
            ),
        ]),
    )?;
    let artifact = ParityArtifact {
        manifest: run_manifest,
        evidence,
        reference,
        external,
        mt5_metadata: metadata,
        report,
    };
    write_json_new(&out, &artifact)?;
    Ok(ParityView {
        output_path: display_path(&out),
        passed: artifact.report.passed,
        grade: if artifact.report.passed {
            "parity-passed"
        } else {
            "accepted"
        },
        reference_engine: artifact.reference.engine.clone(),
        external_engine: artifact.external.engine.clone(),
        reference_trades: artifact.reference.metrics.trade_count,
        external_trades: artifact.external.metrics.trade_count,
        aligned_trades: artifact.report.aligned_trade_count,
        required_aligned_trades: artifact.report.required_aligned_trade_count,
        net_profit_delta_relative: artifact.report.net_profit_delta_relative,
        drawdown_delta_relative: artifact.report.max_drawdown_delta_relative,
        equity_divergence_percent: artifact.report.max_equity_path_divergence_percent,
        protective_orders_present: artifact.report.protective_orders_present,
        reference_win_rate: artifact.report.reference_win_rate,
        external_win_rate: artifact.report.external_win_rate,
        reference_winning_trades: artifact.report.reference_winning_trades,
        external_winning_trades: artifact.report.external_winning_trades,
        reference_profit_factor: artifact.report.reference_profit_factor,
        external_profit_factor: artifact.report.external_profit_factor,
    })
}

#[tauri::command]
pub async fn compare_indicator_parity(
    request: IndicatorParityRequest,
) -> Result<IndicatorParityView, String> {
    tauri::async_runtime::spawn_blocking(move || compare_indicator_parity_sync(&request))
        .await
        .map_err(|error| format!("indicator parity task failed: {error}"))?
}

fn compare_indicator_parity_sync(
    request: &IndicatorParityRequest,
) -> Result<IndicatorParityView, String> {
    let out = ensure_new(&request.output_path, "indicator parity artifact")?;
    let report = quantforge_parity::compare_indicator_reference(
        &request.reference_path,
        quantforge_parity::IndicatorParityConfig {
            warmup_rows: request.warmup_rows,
            absolute_epsilon: request.absolute_epsilon,
            relative_epsilon: request.relative_epsilon,
        },
    )
    .map_err(|error| error.to_string())?;
    let run_manifest = manifest(
        "indicator-parity",
        Some(report.reference_hash.clone()),
        None,
        Some(quantforge_discover::GRAMMAR_VERSION.into()),
        None,
        BTreeMap::from([
            ("reference".into(), recipe_path(&request.reference_path)),
            (
                "protocol".into(),
                json!(quantforge_parity::INDICATOR_PARITY_PROTOCOL_VERSION),
            ),
            (
                "terminal_build".into(),
                json!(report.metadata.terminal_build),
            ),
            ("broker".into(), json!(&report.metadata.broker)),
            ("server".into(), json!(&report.metadata.server)),
            ("symbol".into(), json!(&report.metadata.symbol)),
            ("timeframe".into(), json!(&report.metadata.timeframe)),
            ("period".into(), json!(report.metadata.period)),
            (
                "tolerances".into(),
                serde_json::to_value(&report.config).map_err(|error| error.to_string())?,
            ),
        ]),
    )?;
    let artifact = IndicatorParityArtifact {
        manifest: run_manifest,
        report,
    };
    write_json_new(&out, &artifact)?;
    let mismatch_count = artifact
        .report
        .indicators
        .values()
        .map(|field| field.mismatch_count)
        .sum();
    Ok(IndicatorParityView {
        output_path: display_path(&out),
        passed: artifact.report.passed,
        symbol: artifact.report.metadata.symbol.clone(),
        timeframe: artifact.report.metadata.timeframe.clone(),
        source_rows: artifact.report.source_rows,
        compared_rows: artifact.report.compared_rows,
        field_count: artifact.report.indicators.len(),
        mismatch_count,
    })
}
