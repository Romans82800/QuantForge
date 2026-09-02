use quantforge_discover::{
    MetaCandidate, MetaFeatureRecord, MetaFutureOutcome, MetaLearningConfig, MetaLearningInput,
    MetaExpectancyWalkForwardReport, MetaWalkForwardReport, MetaWindow, MetaWindowRole,
};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn feature(id: &str, cutoff: i64, positive: bool) -> MetaFeatureRecord {
    let quality = if positive { 1.0 } else { -1.0 };
    MetaFeatureRecord {
        strategy_id: id.into(),
        asset: Some("EURUSD".into()),
        feature_cutoff_timestamp_ms: cutoff,
        family: "trend_pullback".into(),
        complexity: if positive { 4 } else { 18 },
        entry_conditions: 2,
        exit_conditions: 1,
        is_expectancy_r: quality,
        is_return_percent: quality * 10.0,
        is_trade_count: if positive { 40 } else { 8 },
        is_profit_factor: Some(if positive { 1.5 } else { 0.9 }),
        is_sharpe: Some(quality),
        is_drawdown_percent: if positive { 10.0 } else { 40.0 },
        is_recovery_factor: Some(quality),
        is_return_drawdown_ratio: quality,
        is_median_r: quality,
        fold_median_expectancy_r: quality,
        fold_spread_r: 0.1,
        fold_passing_fraction: Some(if positive { 0.8 } else { 0.2 }),
        fold_has_negative: !positive,
        parameter_median_ratio: Some(if positive { 1.0 } else { 0.3 }),
        neighborhood_survival_fraction: Some(if positive { 0.8 } else { 0.1 }),
        neighborhood_samples: 20,
        m1_return_retention: Some(if positive { 0.9 } else { 0.2 }),
        m1_trade_retention: Some(if positive { 0.9 } else { 0.2 }),
        m1_drawdown_expansion: Some(if positive { 1.0 } else { 2.0 }),
    }
}

fn input() -> MetaLearningInput {
    let windows = vec![
        MetaWindow {
            id: "dev-100".into(),
            role: MetaWindowRole::Development,
            feature_cutoff_timestamp_ms: 100,
            label_start_timestamp_ms: 101,
            label_end_timestamp_ms: 200,
            horizon_months: 6,
        },
        MetaWindow {
            id: "val-200".into(),
            role: MetaWindowRole::Validation,
            feature_cutoff_timestamp_ms: 200,
            label_start_timestamp_ms: 201,
            label_end_timestamp_ms: 300,
            horizon_months: 6,
        },
        MetaWindow {
            id: "sealed-300".into(),
            role: MetaWindowRole::Sealed,
            feature_cutoff_timestamp_ms: 300,
            label_start_timestamp_ms: 301,
            label_end_timestamp_ms: 400,
            horizon_months: 12,
        },
    ];
    let mut candidates = Vec::new();
    for (cutoff, window_id) in [(100_i64, "dev-100"), (200, "val-200"), (300, "sealed-300")] {
        for index in 0..6 {
            let positive = index < 3;
            let id = format!("{cutoff}-{index}");
            candidates.push(MetaCandidate {
                features: feature(&id, cutoff, positive),
                outcomes: vec![MetaFutureOutcome {
                    window_id: window_id.into(),
                    future_expectancy_r: if positive { 0.8 } else { -0.1 },
                    future_trade_count: if positive { 20 } else { 2 },
                    future_return_percent: None,
                    future_profit_factor: None,
                    future_drawdown_percent: None,
                }],
            });
        }
    }
    MetaLearningInput {
        config: MetaLearningConfig {
            minimum_future_trades: 5,
            minimum_retention: 0.7,
            ..MetaLearningConfig::default()
        },
        windows,
        candidates,
    }
}

#[test]
fn meta_learn_command_writes_walk_forward_report() {
    let directory = tempdir().unwrap();
    let input_path = directory.path().join("input.json");
    let output_path = directory.path().join("report.json");
    fs::write(&input_path, serde_json::to_vec_pretty(&input()).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_quantforge"))
        .args([
            "meta-learn",
            "--input",
            input_path.to_str().unwrap(),
            "--out",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: MetaWalkForwardReport =
        serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(report.episodes.len(), 2);
    assert!(report.final_sealed_evaluation.is_some());
}

#[test]
fn meta_expectancy_command_writes_walk_forward_report() {
    let directory = tempdir().unwrap();
    let input_path = directory.path().join("input.json");
    let output_path = directory.path().join("report.json");
    fs::write(&input_path, serde_json::to_vec_pretty(&input()).unwrap()).unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_quantforge"))
        .args([
            "meta-expectancy",
            "--input",
            input_path.to_str().unwrap(),
            "--out",
            output_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: MetaExpectancyWalkForwardReport =
        serde_json::from_slice(&fs::read(output_path).unwrap()).unwrap();
    assert_eq!(report.episodes.len(), 2);
    assert!(report.final_sealed_evaluation.is_some());
}
