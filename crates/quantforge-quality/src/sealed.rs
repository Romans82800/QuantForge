use crate::{
    CHALLENGE_PROTOCOL, ChallengeError, ChallengeReport, DataSplitPlan, EvidenceBinding,
    EvidenceError, SEALED_FINAL_PROTOCOL,
};
use quantforge_broker::{BrokerSpecError, SymbolSpecification};
use quantforge_core::{ContentHash, FloatPolicy, HashError, stable_json_hash};
use quantforge_data::{BarDataset, bar_content_hash};
use quantforge_eval::{
    BacktestMetrics, EvalError, ScoutConfig, ScoutResult, evaluate_strategy_from,
};
use quantforge_ir::{IrError, StrategyIr};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SEALED_FINAL_REPORT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SealedFinalConfig {
    pub scout: ScoutConfig,
    pub minimum_trades: usize,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    pub maximum_drawdown_percent: f64,
}

impl Default for SealedFinalConfig {
    fn default() -> Self {
        Self {
            scout: ScoutConfig::default(),
            minimum_trades: 20,
            minimum_return_percent: 1.0,
            minimum_profit_factor: 1.1,
            maximum_drawdown_percent: 20.0,
        }
    }
}

impl SealedFinalConfig {
    pub fn validate(&self, challenge: &ChallengeReport) -> Result<(), SealedFinalError> {
        self.scout.validate()?;
        if self.scout != challenge.config.scout {
            return Err(SealedFinalError::ScoutConfigMismatch);
        }
        for (name, value) in [
            ("minimum_return_percent", self.minimum_return_percent),
            ("minimum_profit_factor", self.minimum_profit_factor),
            ("maximum_drawdown_percent", self.maximum_drawdown_percent),
        ] {
            if !value.is_finite() {
                return Err(SealedFinalError::InvalidConfig(format!(
                    "{name} must be finite"
                )));
            }
        }
        if self.minimum_profit_factor < 0.0 || self.maximum_drawdown_percent < 0.0 {
            return Err(SealedFinalError::InvalidConfig(
                "profit factor and drawdown thresholds must be non-negative".into(),
            ));
        }

        let challenge_config = &challenge.config;
        if self.minimum_trades < challenge_config.minimum_baseline_trades
            || self.minimum_return_percent < challenge_config.minimum_return_percent
            || self.minimum_profit_factor < challenge_config.minimum_profit_factor
            || self.maximum_drawdown_percent > challenge_config.maximum_drawdown_percent
        {
            return Err(SealedFinalError::NotAtLeastAsStrictAsChallenge);
        }
        let strictly_tighter = self.minimum_trades > challenge_config.minimum_baseline_trades
            || self.minimum_return_percent > challenge_config.minimum_return_percent
            || self.minimum_profit_factor > challenge_config.minimum_profit_factor
            || self.maximum_drawdown_percent < challenge_config.maximum_drawdown_percent;
        if !strictly_tighter {
            return Err(SealedFinalError::NotStrictlyTighterThanChallenge);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SealedFinalBlocker {
    MinimumTrades,
    MinimumReturn,
    MinimumProfitFactor,
    MaximumDrawdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SealedFinalReport {
    pub schema_version: u16,
    pub protocol_version: String,
    pub binding: EvidenceBinding,
    pub split_plan_hash: ContentHash,
    pub challenge_artifact_hash: ContentHash,
    pub challenge_report_hash: ContentHash,
    pub sealed_data_hash: ContentHash,
    pub sealed_start_timestamp_ms: i64,
    pub sealed_end_timestamp_ms_exclusive: i64,
    pub sealed_bar_count: usize,
    pub shortlisted_before_open: bool,
    pub used_in_selection_score: bool,
    pub config: SealedFinalConfig,
    pub result: ScoutResult,
    pub blockers: Vec<SealedFinalBlocker>,
    pub passed: bool,
}

impl SealedFinalReport {
    pub fn validate_integrity(&self, challenge: &ChallengeReport) -> Result<(), SealedFinalError> {
        self.config.validate(challenge)?;
        if self.schema_version != SEALED_FINAL_REPORT_SCHEMA_VERSION
            || self.protocol_version != SEALED_FINAL_PROTOCOL
            || !self.shortlisted_before_open
            || self.used_in_selection_score
        {
            return Err(SealedFinalError::InvalidReport(
                "protocol or sealed-access flags are inconsistent".into(),
            ));
        }
        if self.challenge_report_hash != stable_json_hash(challenge)? {
            return Err(SealedFinalError::InvalidReport(
                "Challenge report hash is inconsistent".into(),
            ));
        }
        let blockers = sealed_blockers(&self.result.metrics, &self.config);
        if self.blockers != blockers || self.passed != blockers.is_empty() {
            return Err(SealedFinalError::InvalidReport(
                "sealed blockers or pass flag are inconsistent".into(),
            ));
        }
        if self
            .result
            .trades
            .iter()
            .any(|trade| trade.entry_timestamp_ms < self.sealed_start_timestamp_ms)
        {
            return Err(SealedFinalError::InvalidReport(
                "a scored trade entered before the sealed boundary".into(),
            ));
        }
        Ok(())
    }
}

pub fn run_sealed_final(
    strategy: &StrategyIr,
    dataset: &BarDataset,
    broker: &SymbolSpecification,
    split_plan: &DataSplitPlan,
    challenge: &ChallengeReport,
    challenge_artifact_hash: ContentHash,
    config: SealedFinalConfig,
) -> Result<SealedFinalReport, SealedFinalError> {
    split_plan.validate()?;
    challenge.validate_integrity()?;
    if !challenge.passed || !challenge.blockers.is_empty() {
        return Err(SealedFinalError::ChallengeNotPassed);
    }
    if challenge.protocol_version != CHALLENGE_PROTOCOL {
        return Err(SealedFinalError::ChallengeProtocolMismatch);
    }
    config.validate(challenge)?;
    if dataset.data_hash != split_plan.full_data_hash {
        return Err(SealedFinalError::FullDataMismatch);
    }
    let binding = EvidenceBinding {
        strategy_fingerprint: strategy.structural_fingerprint(FloatPolicy::default())?,
        broker_spec_hash: broker.content_hash()?,
    };
    if challenge.binding != binding {
        return Err(SealedFinalError::ChallengeBindingMismatch);
    }
    let split_plan_hash = split_plan.content_hash()?;
    if challenge.split_plan_hash != split_plan_hash
        || challenge.validation_data_hash != split_plan.validation.data_hash
    {
        return Err(SealedFinalError::ChallengeSplitMismatch);
    }

    let segment = &split_plan.sealed_final;
    let sealed_bars: Vec<_> = dataset
        .bars
        .iter()
        .filter(|bar| {
            bar.timestamp_ms >= segment.start_timestamp_ms
                && bar.timestamp_ms < segment.end_timestamp_ms_exclusive
        })
        .cloned()
        .collect();
    if sealed_bars.len() != segment.bar_count || bar_content_hash(&sealed_bars) != segment.data_hash
    {
        return Err(SealedFinalError::SealedDataMismatch);
    }

    // All earlier bars are past-only indicator context. The evaluator's entry
    // boundary guarantees that only trades opened in the sealed partition are
    // scored, and the split's final boundary caps the evaluation end.
    let context_bars: Vec<_> = dataset
        .bars
        .iter()
        .filter(|bar| bar.timestamp_ms < segment.end_timestamp_ms_exclusive)
        .cloned()
        .collect();
    let context = BarDataset {
        data_hash: bar_content_hash(&context_bars),
        source_rows: context_bars.len(),
        bars: context_bars,
        duplicate_rows_removed: 0,
        input_was_sorted: true,
        delimiter: dataset.delimiter,
        source_timezone: dataset.source_timezone.clone(),
    };
    let result = evaluate_strategy_from(
        strategy,
        &context,
        broker,
        &config.scout,
        segment.start_timestamp_ms,
    )?;
    let blockers = sealed_blockers(&result.metrics, &config);
    let passed = blockers.is_empty();
    let report = SealedFinalReport {
        schema_version: SEALED_FINAL_REPORT_SCHEMA_VERSION,
        protocol_version: SEALED_FINAL_PROTOCOL.into(),
        binding,
        split_plan_hash,
        challenge_artifact_hash,
        challenge_report_hash: stable_json_hash(challenge)?,
        sealed_data_hash: segment.data_hash.clone(),
        sealed_start_timestamp_ms: segment.start_timestamp_ms,
        sealed_end_timestamp_ms_exclusive: segment.end_timestamp_ms_exclusive,
        sealed_bar_count: segment.bar_count,
        shortlisted_before_open: true,
        used_in_selection_score: false,
        config,
        result,
        blockers,
        passed,
    };
    report.validate_integrity(challenge)?;
    Ok(report)
}

fn sealed_blockers(
    metrics: &BacktestMetrics,
    config: &SealedFinalConfig,
) -> Vec<SealedFinalBlocker> {
    let mut blockers = Vec::new();
    if metrics.trade_count < config.minimum_trades {
        blockers.push(SealedFinalBlocker::MinimumTrades);
    }
    if metrics.return_percent <= config.minimum_return_percent {
        blockers.push(SealedFinalBlocker::MinimumReturn);
    }
    let profit_factor = metrics.profit_factor.unwrap_or(
        if metrics.net_profit > 0.0 && metrics.winning_trades > 0 {
            f64::MAX
        } else {
            0.0
        },
    );
    if profit_factor < config.minimum_profit_factor {
        blockers.push(SealedFinalBlocker::MinimumProfitFactor);
    }
    if metrics.max_drawdown_percent > config.maximum_drawdown_percent {
        blockers.push(SealedFinalBlocker::MaximumDrawdown);
    }
    blockers
}

#[derive(Debug, Error)]
pub enum SealedFinalError {
    #[error("invalid sealed-final configuration: {0}")]
    InvalidConfig(String),
    #[error("sealed-final thresholds are weaker than the Challenge thresholds")]
    NotAtLeastAsStrictAsChallenge,
    #[error("sealed-final must use the exact Scout cost and balance configuration from Challenge")]
    ScoutConfigMismatch,
    #[error("at least one sealed-final threshold must be strictly tighter than Challenge")]
    NotStrictlyTighterThanChallenge,
    #[error("the shortlist Challenge did not pass")]
    ChallengeNotPassed,
    #[error("the shortlist artifact does not use the Challenge v1 protocol")]
    ChallengeProtocolMismatch,
    #[error("the shortlist Challenge belongs to another strategy or broker")]
    ChallengeBindingMismatch,
    #[error("the shortlist Challenge belongs to another split plan")]
    ChallengeSplitMismatch,
    #[error("input dataset does not match the split plan's full-data identity")]
    FullDataMismatch,
    #[error("sealed bars do not match the split plan")]
    SealedDataMismatch,
    #[error("invalid sealed-final report: {0}")]
    InvalidReport(String),
    #[error(transparent)]
    Challenge(#[from] ChallengeError),
    #[error(transparent)]
    Evidence(#[from] EvidenceError),
    #[error(transparent)]
    Evaluation(#[from] EvalError),
    #[error(transparent)]
    Ir(#[from] IrError),
    #[error(transparent)]
    Broker(#[from] BrokerSpecError),
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChallengeConfig, run_challenge};
    use quantforge_broker::{DayOfWeek, FillingMode, SwapMode, TradeMode};
    use quantforge_core::STRATEGY_IR_VERSION;
    use quantforge_data::Bar;
    use quantforge_eval::ScoutConfig;
    use quantforge_ir::{
        BoolExpr, ComparisonOp, EntrySignals, ManagePolicy, NumericExpr, PriceField,
        ProtectiveStops, RiskPolicy, Side, StopLossPolicy, StrategyMeta, TakeProfitPolicy,
    };

    fn dataset(sealed_profitable: bool) -> BarDataset {
        let bars: Vec<_> = (0..500)
            .map(|index| {
                let sealed = index >= 400;
                let open = if sealed && !sealed_profitable {
                    900.0 - (index - 400) as f64 * 2.0
                } else {
                    100.0 + index as f64 * 2.0
                };
                let (high, low, close) = if sealed && !sealed_profitable {
                    (open + 0.1, open - 2.0, open - 1.0)
                } else {
                    (open + 2.0, open - 0.1, open + 1.0)
                };
                Bar {
                    timestamp_ms: index as i64 * 60_000,
                    open,
                    high,
                    low,
                    close,
                    tick_volume: 100,
                    real_volume: 0,
                    spread_points: Some(0),
                }
            })
            .collect();
        BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            bars,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        }
    }

    fn broker() -> SymbolSpecification {
        SymbolSpecification {
            profile_name: "sealed-fixture".into(),
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
            id: "sealed-always-long".into(),
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
            risk: RiskPolicy::FixedCurrency { amount: 1.0 },
            stops: ProtectiveStops {
                stop_loss: StopLossPolicy::FixedPoints { points: 1.0 },
                take_profit: TakeProfitPolicy::RiskMultiple { multiple: 1.0 },
            },
            manage: ManagePolicy::default(),
            meta: StrategyMeta {
                thesis_hint: "sealed test".into(),
                complexity: 0,
                export_safe: true,
            },
        }
    }

    fn challenge_config() -> ChallengeConfig {
        ChallengeConfig {
            scout: ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
            folds: 4,
            purge_bars: 5,
            embargo_bars: 5,
            minimum_validation_bars: 50,
            minimum_baseline_trades: 20,
            minimum_fold_trades: 5,
            monte_carlo_trials: 100,
            neighborhood_samples: 8,
            evaluations_touched: 100,
            ..ChallengeConfig::default()
        }
    }

    fn sealed_config() -> SealedFinalConfig {
        SealedFinalConfig {
            scout: ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
            ..SealedFinalConfig::default()
        }
    }

    #[test]
    fn passing_sealed_test_scores_only_entries_after_the_boundary() {
        let dataset = dataset(true);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();
        let challenge =
            run_challenge(&strategy(), &dataset, &broker(), &plan, challenge_config()).unwrap();
        let report = run_sealed_final(
            &strategy(),
            &dataset,
            &broker(),
            &plan,
            &challenge,
            ContentHash::sha256("challenge artifact"),
            sealed_config(),
        )
        .unwrap();

        assert!(report.passed, "{:?}", report.blockers);
        assert!(report.shortlisted_before_open);
        assert!(!report.used_in_selection_score);
        assert!(
            report
                .result
                .trades
                .iter()
                .all(|trade| { trade.entry_timestamp_ms >= plan.sealed_final.start_timestamp_ms })
        );
    }

    #[test]
    fn losing_sealed_test_returns_a_demotion_grade_blocker() {
        let dataset = dataset(false);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();
        let challenge =
            run_challenge(&strategy(), &dataset, &broker(), &plan, challenge_config()).unwrap();
        assert!(challenge.passed);
        let report = run_sealed_final(
            &strategy(),
            &dataset,
            &broker(),
            &plan,
            &challenge,
            ContentHash::sha256("challenge artifact"),
            sealed_config(),
        )
        .unwrap();

        assert!(!report.passed);
        assert!(report.blockers.contains(&SealedFinalBlocker::MinimumReturn));
        assert!(
            report
                .blockers
                .contains(&SealedFinalBlocker::MinimumProfitFactor)
        );
    }

    #[test]
    fn sealed_thresholds_cannot_equal_or_weaken_challenge() {
        let dataset = dataset(true);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();
        let challenge =
            run_challenge(&strategy(), &dataset, &broker(), &plan, challenge_config()).unwrap();
        let config = SealedFinalConfig {
            scout: ScoutConfig {
                initial_balance: 100.0,
                ..ScoutConfig::default()
            },
            minimum_trades: challenge.config.minimum_baseline_trades,
            minimum_return_percent: challenge.config.minimum_return_percent,
            minimum_profit_factor: challenge.config.minimum_profit_factor,
            maximum_drawdown_percent: challenge.config.maximum_drawdown_percent,
        };

        assert!(matches!(
            config.validate(&challenge),
            Err(SealedFinalError::NotStrictlyTighterThanChallenge)
        ));
    }
}
