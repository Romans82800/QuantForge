use crate::{EvidenceBinding, INCUBATION_PROTOCOL};
use chrono::NaiveDate;
use quantforge_core::ContentHash;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const INCUBATION_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncubationKillRules {
    pub maximum_daily_loss_percent: f64,
    pub maximum_total_drawdown_percent: f64,
    pub minimum_observation_days: usize,
    pub minimum_total_trades: usize,
    pub maximum_consecutive_zero_trade_days: usize,
}

impl Default for IncubationKillRules {
    fn default() -> Self {
        Self {
            maximum_daily_loss_percent: 2.0,
            maximum_total_drawdown_percent: 10.0,
            minimum_observation_days: 30,
            minimum_total_trades: 20,
            maximum_consecutive_zero_trade_days: 5,
        }
    }
}

impl IncubationKillRules {
    pub fn validate(&self) -> Result<(), IncubationError> {
        for (name, value) in [
            (
                "maximum_daily_loss_percent",
                self.maximum_daily_loss_percent,
            ),
            (
                "maximum_total_drawdown_percent",
                self.maximum_total_drawdown_percent,
            ),
        ] {
            if !value.is_finite() || value <= 0.0 || value > 100.0 {
                return Err(IncubationError::InvalidRules(format!(
                    "{name} must be finite, greater than zero and at most 100"
                )));
            }
        }
        if self.minimum_observation_days == 0 || self.minimum_total_trades == 0 {
            return Err(IncubationError::InvalidRules(
                "minimum observation days and total trades must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncubationStart {
    pub schema_version: u16,
    pub protocol_version: String,
    pub binding: EvidenceBinding,
    pub split_plan_hash: ContentHash,
    pub started_on: NaiveDate,
    pub initial_balance: f64,
    pub kill_rules: IncubationKillRules,
}

impl IncubationStart {
    pub fn validate(&self) -> Result<(), IncubationError> {
        self.kill_rules.validate()?;
        if self.schema_version != INCUBATION_SCHEMA_VERSION
            || self.protocol_version != INCUBATION_PROTOCOL
        {
            return Err(IncubationError::InvalidStart(
                "schema or protocol does not match incubation v1".into(),
            ));
        }
        for (field, hash) in [
            ("strategy_fingerprint", &self.binding.strategy_fingerprint),
            ("broker_spec_hash", &self.binding.broker_spec_hash),
            ("split_plan_hash", &self.split_plan_hash),
        ] {
            if hash.as_str().len() != 64
                || !hash.as_str().bytes().all(|value| value.is_ascii_hexdigit())
            {
                return Err(IncubationError::InvalidStart(format!(
                    "{field} is not a SHA-256 identity"
                )));
            }
        }
        if !self.initial_balance.is_finite() || self.initial_balance <= 0.0 {
            return Err(IncubationError::InvalidStart(
                "initial balance must be finite and positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncubationObservation {
    pub date: NaiveDate,
    pub starting_balance: f64,
    pub ending_balance: f64,
    pub maximum_drawdown_percent: f64,
    pub trade_count: usize,
    pub note: Option<String>,
}

impl IncubationObservation {
    pub fn net_profit(&self) -> f64 {
        self.ending_balance - self.starting_balance
    }

    pub fn return_percent(&self) -> f64 {
        self.net_profit() / self.starting_balance * 100.0
    }

    pub fn validate(&self) -> Result<(), IncubationError> {
        if !self.starting_balance.is_finite()
            || self.starting_balance <= 0.0
            || !self.ending_balance.is_finite()
            || self.ending_balance <= 0.0
        {
            return Err(IncubationError::InvalidObservation(
                "balances must be finite and positive".into(),
            ));
        }
        if !self.maximum_drawdown_percent.is_finite()
            || !(0.0..=100.0).contains(&self.maximum_drawdown_percent)
        {
            return Err(IncubationError::InvalidObservation(
                "maximum drawdown must be finite and between zero and 100".into(),
            ));
        }
        if self
            .note
            .as_ref()
            .is_some_and(|note| note.trim().is_empty())
        {
            return Err(IncubationError::InvalidObservation(
                "an optional note cannot be blank".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncubationBlocker {
    InsufficientObservationDays,
    DailyLossLimitBreached,
    TotalDrawdownLimitBreached,
    InsufficientTotalTrades,
    TradeFrequencyCollapsed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IncubationReport {
    pub schema_version: u16,
    pub protocol_version: String,
    pub method: String,
    pub binding: EvidenceBinding,
    pub split_plan_hash: ContentHash,
    pub start_artifact_hash: ContentHash,
    pub observation_artifact_hashes: Vec<ContentHash>,
    pub started_on: NaiveDate,
    pub ended_on: NaiveDate,
    pub kill_rules: IncubationKillRules,
    pub initial_balance: f64,
    pub ending_balance: f64,
    pub net_profit: f64,
    pub return_percent: f64,
    pub observation_days: usize,
    pub total_trades: usize,
    pub maximum_observed_drawdown_percent: f64,
    pub maximum_consecutive_zero_trade_days: usize,
    pub blockers: Vec<IncubationBlocker>,
    pub passed: bool,
}

impl IncubationReport {
    pub fn validate_integrity(
        &self,
        start: &IncubationStart,
        observations: &[IncubationObservation],
    ) -> Result<(), IncubationError> {
        validate_hash("start_artifact_hash", &self.start_artifact_hash)?;
        validate_sequence(start, observations, &self.observation_artifact_hashes)?;
        let expected = build_report(
            start,
            observations,
            self.start_artifact_hash.clone(),
            self.observation_artifact_hashes.clone(),
        );
        if self != &expected {
            return Err(IncubationError::InvalidReport(
                "stored aggregates, blockers or bindings are inconsistent".into(),
            ));
        }
        Ok(())
    }
}

pub fn run_incubation(
    start: &IncubationStart,
    observations: &[IncubationObservation],
    start_artifact_hash: ContentHash,
    observation_artifact_hashes: Vec<ContentHash>,
) -> Result<IncubationReport, IncubationError> {
    validate_hash("start_artifact_hash", &start_artifact_hash)?;
    validate_sequence(start, observations, &observation_artifact_hashes)?;
    let report = build_report(
        start,
        observations,
        start_artifact_hash,
        observation_artifact_hashes,
    );
    report.validate_integrity(start, observations)?;
    Ok(report)
}

fn validate_sequence(
    start: &IncubationStart,
    observations: &[IncubationObservation],
    hashes: &[ContentHash],
) -> Result<(), IncubationError> {
    start.validate()?;
    if observations.is_empty() || observations.len() != hashes.len() {
        return Err(IncubationError::InvalidSequence(
            "at least one observation and one matching artifact hash are required".into(),
        ));
    }
    for (index, (observation, hash)) in observations.iter().zip(hashes).enumerate() {
        observation.validate()?;
        validate_hash("observation_artifact_hash", hash)?;
        if observation.date < start.started_on {
            return Err(IncubationError::InvalidSequence(
                "an observation predates incubation".into(),
            ));
        }
        if index > 0 && observation.date <= observations[index - 1].date {
            return Err(IncubationError::InvalidSequence(
                "observation dates must be strictly increasing".into(),
            ));
        }
        let expected_start = if index == 0 {
            start.initial_balance
        } else {
            observations[index - 1].ending_balance
        };
        if !same_float(observation.starting_balance, expected_start) {
            return Err(IncubationError::InvalidSequence(
                "observation balances are not continuous".into(),
            ));
        }
    }
    Ok(())
}

fn build_report(
    start: &IncubationStart,
    observations: &[IncubationObservation],
    start_artifact_hash: ContentHash,
    observation_artifact_hashes: Vec<ContentHash>,
) -> IncubationReport {
    let ending_balance = observations
        .last()
        .expect("sequence validation requires observations")
        .ending_balance;
    let net_profit = ending_balance - start.initial_balance;
    let return_percent = net_profit / start.initial_balance * 100.0;
    let total_trades = observations
        .iter()
        .map(|observation| observation.trade_count)
        .sum();
    let maximum_observed_drawdown_percent = observations
        .iter()
        .map(|observation| observation.maximum_drawdown_percent)
        .fold(0.0_f64, f64::max);
    let maximum_consecutive_zero_trade_days = maximum_zero_trade_streak(observations);
    let mut blockers = Vec::new();
    if observations.len() < start.kill_rules.minimum_observation_days {
        blockers.push(IncubationBlocker::InsufficientObservationDays);
    }
    if observations.iter().any(|observation| {
        observation.return_percent() < -start.kill_rules.maximum_daily_loss_percent
    }) {
        blockers.push(IncubationBlocker::DailyLossLimitBreached);
    }
    if maximum_observed_drawdown_percent > start.kill_rules.maximum_total_drawdown_percent {
        blockers.push(IncubationBlocker::TotalDrawdownLimitBreached);
    }
    if total_trades < start.kill_rules.minimum_total_trades {
        blockers.push(IncubationBlocker::InsufficientTotalTrades);
    }
    if maximum_consecutive_zero_trade_days > start.kill_rules.maximum_consecutive_zero_trade_days {
        blockers.push(IncubationBlocker::TradeFrequencyCollapsed);
    }
    IncubationReport {
        schema_version: INCUBATION_SCHEMA_VERSION,
        protocol_version: INCUBATION_PROTOCOL.into(),
        method: "append_only_daily_paper_ledger_v1".into(),
        binding: start.binding.clone(),
        split_plan_hash: start.split_plan_hash.clone(),
        start_artifact_hash,
        observation_artifact_hashes,
        started_on: start.started_on,
        ended_on: observations
            .last()
            .expect("sequence validation requires observations")
            .date,
        kill_rules: start.kill_rules.clone(),
        initial_balance: start.initial_balance,
        ending_balance,
        net_profit,
        return_percent,
        observation_days: observations.len(),
        total_trades,
        maximum_observed_drawdown_percent,
        maximum_consecutive_zero_trade_days,
        passed: blockers.is_empty(),
        blockers,
    }
}

fn maximum_zero_trade_streak(observations: &[IncubationObservation]) -> usize {
    let mut current = 0;
    let mut maximum = 0;
    for observation in observations {
        if observation.trade_count == 0 {
            current += 1;
            maximum = maximum.max(current);
        } else {
            current = 0;
        }
    }
    maximum
}

fn validate_hash(field: &str, hash: &ContentHash) -> Result<(), IncubationError> {
    if hash.as_str().len() == 64 && hash.as_str().bytes().all(|value| value.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(IncubationError::InvalidSequence(format!(
            "{field} is not a SHA-256 identity"
        )))
    }
}

fn same_float(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1.0e-9 * left.abs().max(right.abs()).max(1.0)
}

#[derive(Debug, Error)]
pub enum IncubationError {
    #[error("invalid incubation kill rules: {0}")]
    InvalidRules(String),
    #[error("invalid incubation start: {0}")]
    InvalidStart(String),
    #[error("invalid incubation observation: {0}")]
    InvalidObservation(String),
    #[error("invalid incubation sequence: {0}")]
    InvalidSequence(String),
    #[error("invalid incubation report: {0}")]
    InvalidReport(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start() -> IncubationStart {
        IncubationStart {
            schema_version: INCUBATION_SCHEMA_VERSION,
            protocol_version: INCUBATION_PROTOCOL.into(),
            binding: EvidenceBinding {
                strategy_fingerprint: ContentHash::sha256("strategy"),
                broker_spec_hash: ContentHash::sha256("broker"),
            },
            split_plan_hash: ContentHash::sha256("split"),
            started_on: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            initial_balance: 100_000.0,
            kill_rules: IncubationKillRules {
                minimum_observation_days: 3,
                minimum_total_trades: 2,
                maximum_consecutive_zero_trade_days: 1,
                ..IncubationKillRules::default()
            },
        }
    }

    #[test]
    fn passing_ledger_is_deterministic_and_auditable() {
        let observations = vec![
            IncubationObservation {
                date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                starting_balance: 100_000.0,
                ending_balance: 100_100.0,
                maximum_drawdown_percent: 1.0,
                trade_count: 1,
                note: None,
            },
            IncubationObservation {
                date: NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
                starting_balance: 100_100.0,
                ending_balance: 100_050.0,
                maximum_drawdown_percent: 1.5,
                trade_count: 0,
                note: None,
            },
            IncubationObservation {
                date: NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(),
                starting_balance: 100_050.0,
                ending_balance: 100_250.0,
                maximum_drawdown_percent: 1.0,
                trade_count: 1,
                note: Some("paper fill review complete".into()),
            },
        ];
        let hashes = (0..observations.len())
            .map(|index| ContentHash::sha256(index.to_string()))
            .collect();
        let report = run_incubation(
            &start(),
            &observations,
            ContentHash::sha256("start"),
            hashes,
        )
        .unwrap();
        assert!(report.passed);
        assert!(report.blockers.is_empty());
        assert_eq!(report.total_trades, 2);
        report.validate_integrity(&start(), &observations).unwrap();
    }

    #[test]
    fn breached_kill_rules_are_preserved_as_blockers() {
        let observations = vec![
            IncubationObservation {
                date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
                starting_balance: 100_000.0,
                ending_balance: 97_000.0,
                maximum_drawdown_percent: 12.0,
                trade_count: 0,
                note: None,
            },
            IncubationObservation {
                date: NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
                starting_balance: 97_000.0,
                ending_balance: 97_000.0,
                maximum_drawdown_percent: 12.0,
                trade_count: 0,
                note: None,
            },
        ];
        let report = run_incubation(
            &start(),
            &observations,
            ContentHash::sha256("start"),
            vec![ContentHash::sha256("one"), ContentHash::sha256("two")],
        )
        .unwrap();
        assert!(!report.passed);
        assert!(
            report
                .blockers
                .contains(&IncubationBlocker::DailyLossLimitBreached)
        );
        assert!(
            report
                .blockers
                .contains(&IncubationBlocker::TotalDrawdownLimitBreached)
        );
        assert!(
            report
                .blockers
                .contains(&IncubationBlocker::TradeFrequencyCollapsed)
        );
    }

    #[test]
    fn balance_discontinuity_is_rejected() {
        let observation = IncubationObservation {
            date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            starting_balance: 99_000.0,
            ending_balance: 100_000.0,
            maximum_drawdown_percent: 1.0,
            trade_count: 1,
            note: None,
        };
        let result = run_incubation(
            &start(),
            &[observation],
            ContentHash::sha256("start"),
            vec![ContentHash::sha256("observation")],
        );
        assert!(matches!(result, Err(IncubationError::InvalidSequence(_))));
    }
}
