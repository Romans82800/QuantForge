//! Statistical evidence, sealed data partitions and non-bypassable promotion gates.

mod challenge;
mod databank_correlation;
mod databank_filter;
mod incubation;
mod multi_symbol_matrix;
mod negate;
mod results_html;
mod results_pack;
mod sealed;
mod task_executor;
mod task_graph;
mod walk_forward_matrix;
mod what_if;

pub use challenge::{
    CHALLENGE_REPORT_SCHEMA_VERSION, ChallengeBlocker, ChallengeConfig, ChallengeError,
    ChallengeReport, CostShockPoint, CostShockReport, MonteCarloReport, MultipleTestingReport,
    ParameterNeighbor, ParameterNeighborhoodReport, PurgedFoldReport, SelectionBiasLevel,
    deflated_trade_sharpe, expected_max_lucky_sharpe, monte_carlo_from_trade_profits,
    monte_carlo_trade_resampling_with_skip,
    perturb_strategy_parameters, perturb_strategy_parameters_with_probability, run_challenge,
    trade_sharpe_proxy,
};
pub use databank_correlation::{
    DATABANK_CORRELATION_PROTOCOL, CorrelationCandidate, CorrelationFilterError,
    CorrelationFilterReport, RejectedPair, candidates_from_values, filter_by_correlation,
};
pub use databank_filter::{
    DATABANK_FILTER_PROTOCOL, CompareOp, FilterError, FilterExpr, FilterReport, FilterValue,
    eval_filter, filter_rows, known_columns, parse_filter, row_from_value,
};
pub use multi_symbol_matrix::{
    MULTI_SYMBOL_MATRIX_PROTOCOL, MatrixSymbolInput, MultiSymbolMatrixError,
    MultiSymbolMatrixReport, MultiSymbolMatrixRow, PairwiseSymbolCorrelation,
    run_multi_symbol_matrix,
};
pub use incubation::{
    INCUBATION_SCHEMA_VERSION, IncubationBlocker, IncubationError, IncubationKillRules,
    IncubationObservation, IncubationReport, IncubationStart, run_incubation,
};
pub use negate::{
    NEGATE_PROTOCOL_VERSION, NegateError, NegateMode, NegateReport, negate_strategy,
};
pub use results_html::{
    HTML_REPORT_PROTOCOL, render_results_html, render_results_html_from_json,
    render_results_html_from_scout,
};
pub use results_pack::{
    RESULTS_PACK_PROTOCOL, ResultsPackPaths, render_results_pdf, render_trades_csv,
    write_results_pack, write_results_pack_from_json, write_results_pack_from_scout,
};
pub use sealed::{
    SEALED_FINAL_REPORT_SCHEMA_VERSION, SealedFinalBlocker, SealedFinalConfig, SealedFinalError,
    SealedFinalReport, run_sealed_final,
};
pub use task_executor::{TaskArtifactStore, TaskRunOptions, run_task_graph};
pub use task_graph::{
    TASK_GRAPH_PROTOCOL, TASK_GRAPH_SCHEMA_VERSION, TaskGraph, TaskGraphError, TaskRunReport,
    TaskStep, TaskStepKind, TaskStepResult, TaskStepStatus, example_retester_graph,
};
pub use walk_forward_matrix::{
    WALK_FORWARD_MATRIX_PROTOCOL, WalkForwardMatrixCell, WalkForwardMatrixConfig,
    WalkForwardMatrixError, WalkForwardMatrixFold, WalkForwardMatrixReport,
    run_walk_forward_matrix,
};
pub use what_if::{
    WHAT_IF_PROTOCOL_VERSION, WhatIfError, WhatIfFilter, WhatIfReport, apply_what_if,
};

use quantforge_core::{ContentHash, HashError, stable_json_hash};
use quantforge_data::{BarDataset, bar_content_hash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const EVIDENCE_PROTOCOL_VERSION: &str = "certification-evidence-v1";
pub const SPLIT_PLAN_SCHEMA_VERSION: u16 = 1;
pub const CERTIFICATION_SCHEMA_VERSION: u16 = 1;
pub const VALIDATION_ATTESTATION_SCHEMA_VERSION: u16 = 1;

pub const VALIDATION_PROTOCOL: &str = "validation-v1";
pub const ILLUMINATION_PROTOCOL: &str = "map-elites-illumination-v1";
pub const CHALLENGE_PROTOCOL: &str = "challenge-battery-v1";
pub const JUDGE_PROTOCOL: &str = "m1-judge-v1";
pub const EXTERNAL_PARITY_PROTOCOL: &str = "mt5-parity-v1";
pub const INDICATOR_PARITY_PROTOCOL: &str = "mt5-indicator-parity-v1";
pub const SEALED_FINAL_PROTOCOL: &str = "sealed-final-v1";
pub const INCUBATION_PROTOCOL: &str = "incubation-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSegment {
    pub start_timestamp_ms: i64,
    pub end_timestamp_ms_exclusive: i64,
    pub bar_count: usize,
    pub data_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSplitPlan {
    pub schema_version: u16,
    pub full_data_hash: ContentHash,
    pub bar_count: usize,
    pub development: DataSegment,
    pub validation: DataSegment,
    pub sealed_final: DataSegment,
}

impl DataSplitPlan {
    /// Creates three non-overlapping chronological partitions. Only hashes and
    /// boundaries are retained in the plan; the sealed bars are not copied out.
    pub fn chronological(
        dataset: &BarDataset,
        validation_fraction: f64,
        sealed_fraction: f64,
    ) -> Result<Self, EvidenceError> {
        validate_fractions(validation_fraction, sealed_fraction)?;
        let bars = &dataset.bars;
        if bars.len() < 3 {
            return Err(EvidenceError::InsufficientBars(bars.len()));
        }
        if !bars
            .windows(2)
            .all(|pair| pair[0].timestamp_ms < pair[1].timestamp_ms)
        {
            return Err(EvidenceError::TimestampsNotStrictlyIncreasing);
        }
        if bar_content_hash(bars) != dataset.data_hash {
            return Err(EvidenceError::DatasetHashMismatch);
        }

        let validation_count = ((bars.len() as f64 * validation_fraction).floor() as usize).max(1);
        let sealed_count = ((bars.len() as f64 * sealed_fraction).floor() as usize).max(1);
        if validation_count + sealed_count >= bars.len() {
            return Err(EvidenceError::EmptyDevelopmentPartition);
        }
        let validation_start = bars.len() - validation_count - sealed_count;
        let sealed_start = bars.len() - sealed_count;
        let final_end = bars
            .last()
            .expect("the length check proves a final bar exists")
            .timestamp_ms
            .checked_add(1)
            .ok_or(EvidenceError::TimestampOverflow)?;

        let plan = Self {
            schema_version: SPLIT_PLAN_SCHEMA_VERSION,
            full_data_hash: dataset.data_hash.clone(),
            bar_count: bars.len(),
            development: segment(
                &bars[..validation_start],
                bars[validation_start].timestamp_ms,
            ),
            validation: segment(
                &bars[validation_start..sealed_start],
                bars[sealed_start].timestamp_ms,
            ),
            sealed_final: segment(&bars[sealed_start..], final_end),
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<(), EvidenceError> {
        if self.schema_version != SPLIT_PLAN_SCHEMA_VERSION {
            return Err(EvidenceError::UnsupportedSplitSchema(self.schema_version));
        }
        for (name, partition) in [
            ("development", &self.development),
            ("validation", &self.validation),
            ("sealed_final", &self.sealed_final),
        ] {
            if partition.bar_count == 0
                || partition.start_timestamp_ms >= partition.end_timestamp_ms_exclusive
            {
                return Err(EvidenceError::InvalidPartition(name.into()));
            }
            validate_sha256(&partition.data_hash)
                .map_err(|_| EvidenceError::InvalidPartitionHash(name.into()))?;
        }
        if self.development.end_timestamp_ms_exclusive != self.validation.start_timestamp_ms
            || self.validation.end_timestamp_ms_exclusive != self.sealed_final.start_timestamp_ms
        {
            return Err(EvidenceError::PartitionGapOrOverlap);
        }
        let represented = self
            .development
            .bar_count
            .checked_add(self.validation.bar_count)
            .and_then(|count| count.checked_add(self.sealed_final.bar_count))
            .ok_or_else(|| EvidenceError::InvalidPartition("bar count overflow".into()))?;
        if represented != self.bar_count {
            return Err(EvidenceError::PartitionCountMismatch {
                expected: self.bar_count,
                actual: represented,
            });
        }
        validate_sha256(&self.full_data_hash)
            .map_err(|_| EvidenceError::InvalidPartitionHash("full_data".into()))?;
        Ok(())
    }

    pub fn content_hash(&self) -> Result<ContentHash, HashError> {
        stable_json_hash(self)
    }
}

fn validate_fractions(validation: f64, sealed: f64) -> Result<(), EvidenceError> {
    if !validation.is_finite()
        || !sealed.is_finite()
        || validation <= 0.0
        || sealed <= 0.0
        || validation + sealed >= 1.0
    {
        return Err(EvidenceError::InvalidSplitFractions { validation, sealed });
    }
    Ok(())
}

fn segment(bars: &[quantforge_data::Bar], end_timestamp_ms_exclusive: i64) -> DataSegment {
    DataSegment {
        start_timestamp_ms: bars[0].timestamp_ms,
        end_timestamp_ms_exclusive,
        bar_count: bars.len(),
        data_hash: bar_content_hash(bars),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyGrade {
    Scouted,
    Accepted,
    Illuminated,
    Challenged,
    ParityPassed,
    Certified,
    Deployed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBinding {
    pub strategy_fingerprint: ContentHash,
    pub broker_spec_hash: ContentHash,
}

/// A separately hashed validation-stage record derived from the immutable
/// baseline inside a Challenge report. Keeping this as its own artifact makes
/// validation and the full robustness battery independently auditable without
/// evaluating or selecting the strategy a second time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationAttestation {
    pub schema_version: u16,
    pub protocol_version: String,
    pub binding: EvidenceBinding,
    pub split_plan_hash: ContentHash,
    pub validation_data_hash: ContentHash,
    pub validation_bar_count: usize,
    pub source_challenge_artifact_hash: ContentHash,
    pub source_challenge_report_hash: ContentHash,
    pub result: quantforge_eval::ScoutResult,
    pub passed: bool,
}

impl ValidationAttestation {
    pub fn from_challenge(
        challenge: &ChallengeReport,
        challenge_artifact_hash: ContentHash,
    ) -> Result<Self, ValidationAttestationError> {
        challenge.validate_integrity()?;
        let attestation = Self {
            schema_version: VALIDATION_ATTESTATION_SCHEMA_VERSION,
            protocol_version: VALIDATION_PROTOCOL.into(),
            binding: challenge.binding.clone(),
            split_plan_hash: challenge.split_plan_hash.clone(),
            validation_data_hash: challenge.validation_data_hash.clone(),
            validation_bar_count: challenge.validation_bar_count,
            source_challenge_artifact_hash: challenge_artifact_hash.clone(),
            source_challenge_report_hash: stable_json_hash(challenge)?,
            result: challenge.baseline.clone(),
            passed: challenge.baseline_passed(),
        };
        attestation.validate_integrity(challenge, &challenge_artifact_hash)?;
        Ok(attestation)
    }

    pub fn validate_integrity(
        &self,
        challenge: &ChallengeReport,
        challenge_artifact_hash: &ContentHash,
    ) -> Result<(), ValidationAttestationError> {
        challenge.validate_integrity()?;
        if self.schema_version != VALIDATION_ATTESTATION_SCHEMA_VERSION
            || self.protocol_version != VALIDATION_PROTOCOL
        {
            return Err(ValidationAttestationError::Invalid(
                "schema or protocol does not match validation v1".into(),
            ));
        }
        if self.binding != challenge.binding
            || self.split_plan_hash != challenge.split_plan_hash
            || self.validation_data_hash != challenge.validation_data_hash
            || self.validation_bar_count != challenge.validation_bar_count
            || self.source_challenge_artifact_hash != *challenge_artifact_hash
            || self.source_challenge_report_hash != stable_json_hash(challenge)?
            || self.result != challenge.baseline
            || self.passed != challenge.baseline_passed()
        {
            return Err(ValidationAttestationError::Invalid(
                "attestation does not match its source Challenge baseline".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ValidationAttestationError {
    #[error("invalid validation attestation: {0}")]
    Invalid(String),
    #[error(transparent)]
    Challenge(#[from] ChallengeError),
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundGateEvidence {
    pub binding: EvidenceBinding,
    pub artifact_hash: ContentHash,
    pub protocol_version: String,
    pub passed: bool,
    #[serde(default)]
    pub override_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataGateEvidence {
    #[serde(flatten)]
    pub gate: BoundGateEvidence,
    pub data_hash: ContentHash,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalEngine {
    Mt5StrategyTester,
    InternalJudge,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalParityEvidence {
    #[serde(flatten)]
    pub gate: BoundGateEvidence,
    pub engine: ExternalEngine,
    pub protective_orders_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedFinalEvidence {
    #[serde(flatten)]
    pub gate: BoundGateEvidence,
    pub split_plan_hash: ContentHash,
    pub sealed_data_hash: ContentHash,
    pub shortlisted_before_open: bool,
    pub used_in_selection_score: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificationEvidence {
    pub schema_version: u16,
    pub protocol_version: String,
    pub candidate: EvidenceBinding,
    pub split_plan_hash: ContentHash,
    pub validation: DataGateEvidence,
    pub illumination: BoundGateEvidence,
    pub challenge: BoundGateEvidence,
    pub judge: BoundGateEvidence,
    pub external_parity: ExternalParityEvidence,
    pub indicator_parity: BoundGateEvidence,
    pub sealed_final: SealedFinalEvidence,
    pub incubation: Option<BoundGateEvidence>,
    pub evaluations_touched: u64,
    #[serde(default)]
    pub research_override_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificationPolicy {
    pub require_incubation: bool,
    pub selection_bias_warning_threshold: u64,
}

impl Default for CertificationPolicy {
    fn default() -> Self {
        Self {
            require_incubation: false,
            selection_bias_warning_threshold: 1_500,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CertificationBlocker {
    UnsupportedEvidenceSchema {
        actual: u16,
    },
    WrongEvidenceProtocol {
        actual: String,
    },
    MalformedHash {
        field: String,
    },
    SplitPlanHashMismatch,
    BindingMismatch {
        gate: String,
    },
    ArtifactReused {
        gates: Vec<String>,
    },
    WrongGateProtocol {
        gate: String,
        expected: String,
        actual: String,
    },
    ResearchOverrideUsed {
        gate: String,
        flags: Vec<String>,
    },
    ValidationDataMismatch,
    ValidationFailed,
    IlluminationFailed,
    ChallengeFailed,
    JudgeFailed,
    ExternalEngineIsNotMt5,
    ExternalParityFailed,
    ProtectiveOrdersMissing,
    IndicatorParityFailed,
    SealedPlanMismatch,
    SealedDataMismatch,
    SealedOpenedBeforeShortlist,
    SealedUsedInSelection,
    SealedFinalFailed,
    IncubationMissing,
    IncubationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum CertificationWarning {
    SelectionBiasRisk { evaluations_touched: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromotionDecision {
    pub schema_version: u16,
    pub protocol_version: String,
    pub requested_grade: StrategyGrade,
    pub resulting_grade: StrategyGrade,
    pub passed: bool,
    pub candidate: EvidenceBinding,
    pub evidence_hash: ContentHash,
    pub blockers: Vec<CertificationBlocker>,
    pub warnings: Vec<CertificationWarning>,
}

pub fn evaluate_certification(
    evidence: &CertificationEvidence,
    split_plan: &DataSplitPlan,
    policy: &CertificationPolicy,
) -> Result<PromotionDecision, EvidenceError> {
    split_plan.validate()?;
    let evidence_hash = stable_json_hash(evidence)?;
    let expected_split_hash = split_plan.content_hash()?;
    let mut blockers = Vec::new();

    if evidence.schema_version != CERTIFICATION_SCHEMA_VERSION {
        blockers.push(CertificationBlocker::UnsupportedEvidenceSchema {
            actual: evidence.schema_version,
        });
    }
    if evidence.protocol_version != EVIDENCE_PROTOCOL_VERSION {
        blockers.push(CertificationBlocker::WrongEvidenceProtocol {
            actual: evidence.protocol_version.clone(),
        });
    }
    inspect_hash(
        "candidate.strategy_fingerprint",
        &evidence.candidate.strategy_fingerprint,
        &mut blockers,
    );
    inspect_hash(
        "candidate.broker_spec_hash",
        &evidence.candidate.broker_spec_hash,
        &mut blockers,
    );
    inspect_hash("split_plan_hash", &evidence.split_plan_hash, &mut blockers);
    if evidence.split_plan_hash != expected_split_hash {
        blockers.push(CertificationBlocker::SplitPlanHashMismatch);
    }
    if !evidence.research_override_flags.is_empty() {
        blockers.push(CertificationBlocker::ResearchOverrideUsed {
            gate: "certification".into(),
            flags: evidence.research_override_flags.clone(),
        });
    }

    let mut artifact_uses = BTreeMap::<&ContentHash, Vec<String>>::new();
    for (gate, artifact_hash) in [
        ("validation", &evidence.validation.gate.artifact_hash),
        ("illumination", &evidence.illumination.artifact_hash),
        ("challenge", &evidence.challenge.artifact_hash),
        ("judge", &evidence.judge.artifact_hash),
        (
            "external_parity",
            &evidence.external_parity.gate.artifact_hash,
        ),
        ("indicator_parity", &evidence.indicator_parity.artifact_hash),
        ("sealed_final", &evidence.sealed_final.gate.artifact_hash),
    ] {
        artifact_uses
            .entry(artifact_hash)
            .or_default()
            .push(gate.into());
    }
    if let Some(incubation) = &evidence.incubation {
        artifact_uses
            .entry(&incubation.artifact_hash)
            .or_default()
            .push("incubation".into());
    }
    for gates in artifact_uses.into_values().filter(|gates| gates.len() > 1) {
        blockers.push(CertificationBlocker::ArtifactReused { gates });
    }

    inspect_gate(
        "validation",
        VALIDATION_PROTOCOL,
        &evidence.validation.gate,
        &evidence.candidate,
        &mut blockers,
    );
    inspect_gate(
        "illumination",
        ILLUMINATION_PROTOCOL,
        &evidence.illumination,
        &evidence.candidate,
        &mut blockers,
    );
    inspect_gate(
        "challenge",
        CHALLENGE_PROTOCOL,
        &evidence.challenge,
        &evidence.candidate,
        &mut blockers,
    );
    inspect_gate(
        "judge",
        JUDGE_PROTOCOL,
        &evidence.judge,
        &evidence.candidate,
        &mut blockers,
    );
    inspect_gate(
        "external_parity",
        EXTERNAL_PARITY_PROTOCOL,
        &evidence.external_parity.gate,
        &evidence.candidate,
        &mut blockers,
    );
    inspect_gate(
        "indicator_parity",
        INDICATOR_PARITY_PROTOCOL,
        &evidence.indicator_parity,
        &evidence.candidate,
        &mut blockers,
    );
    inspect_gate(
        "sealed_final",
        SEALED_FINAL_PROTOCOL,
        &evidence.sealed_final.gate,
        &evidence.candidate,
        &mut blockers,
    );
    if let Some(incubation) = &evidence.incubation {
        inspect_gate(
            "incubation",
            INCUBATION_PROTOCOL,
            incubation,
            &evidence.candidate,
            &mut blockers,
        );
    }

    inspect_hash(
        "validation.data_hash",
        &evidence.validation.data_hash,
        &mut blockers,
    );
    if evidence.validation.data_hash != split_plan.validation.data_hash {
        blockers.push(CertificationBlocker::ValidationDataMismatch);
    }
    if !evidence.validation.gate.passed {
        blockers.push(CertificationBlocker::ValidationFailed);
    }
    if !evidence.illumination.passed {
        blockers.push(CertificationBlocker::IlluminationFailed);
    }
    if !evidence.challenge.passed {
        blockers.push(CertificationBlocker::ChallengeFailed);
    }
    if !evidence.judge.passed {
        blockers.push(CertificationBlocker::JudgeFailed);
    }
    if evidence.external_parity.engine != ExternalEngine::Mt5StrategyTester {
        blockers.push(CertificationBlocker::ExternalEngineIsNotMt5);
    }
    if !evidence.external_parity.gate.passed {
        blockers.push(CertificationBlocker::ExternalParityFailed);
    }
    if !evidence.external_parity.protective_orders_present {
        blockers.push(CertificationBlocker::ProtectiveOrdersMissing);
    }
    if !evidence.indicator_parity.passed {
        blockers.push(CertificationBlocker::IndicatorParityFailed);
    }

    inspect_hash(
        "sealed_final.split_plan_hash",
        &evidence.sealed_final.split_plan_hash,
        &mut blockers,
    );
    inspect_hash(
        "sealed_final.sealed_data_hash",
        &evidence.sealed_final.sealed_data_hash,
        &mut blockers,
    );
    if evidence.sealed_final.split_plan_hash != expected_split_hash {
        blockers.push(CertificationBlocker::SealedPlanMismatch);
    }
    if evidence.sealed_final.sealed_data_hash != split_plan.sealed_final.data_hash {
        blockers.push(CertificationBlocker::SealedDataMismatch);
    }
    if !evidence.sealed_final.shortlisted_before_open {
        blockers.push(CertificationBlocker::SealedOpenedBeforeShortlist);
    }
    if evidence.sealed_final.used_in_selection_score {
        blockers.push(CertificationBlocker::SealedUsedInSelection);
    }
    if !evidence.sealed_final.gate.passed {
        blockers.push(CertificationBlocker::SealedFinalFailed);
    }

    if policy.require_incubation {
        match &evidence.incubation {
            None => blockers.push(CertificationBlocker::IncubationMissing),
            Some(incubation) if !incubation.passed => {
                blockers.push(CertificationBlocker::IncubationFailed);
            }
            Some(_) => {}
        }
    }

    let warnings = (evidence.evaluations_touched >= policy.selection_bias_warning_threshold)
        .then_some(CertificationWarning::SelectionBiasRisk {
            evaluations_touched: evidence.evaluations_touched,
        })
        .into_iter()
        .collect();
    let passed = blockers.is_empty();
    Ok(PromotionDecision {
        schema_version: CERTIFICATION_SCHEMA_VERSION,
        protocol_version: EVIDENCE_PROTOCOL_VERSION.into(),
        requested_grade: StrategyGrade::Certified,
        resulting_grade: resulting_grade(&blockers),
        passed,
        candidate: evidence.candidate.clone(),
        evidence_hash,
        blockers,
        warnings,
    })
}

fn inspect_gate(
    name: &str,
    expected_protocol: &str,
    gate: &BoundGateEvidence,
    candidate: &EvidenceBinding,
    blockers: &mut Vec<CertificationBlocker>,
) {
    if &gate.binding != candidate {
        blockers.push(CertificationBlocker::BindingMismatch { gate: name.into() });
    }
    inspect_hash(
        &format!("{name}.artifact_hash"),
        &gate.artifact_hash,
        blockers,
    );
    if gate.protocol_version != expected_protocol {
        blockers.push(CertificationBlocker::WrongGateProtocol {
            gate: name.into(),
            expected: expected_protocol.into(),
            actual: gate.protocol_version.clone(),
        });
    }
    if !gate.override_flags.is_empty() {
        blockers.push(CertificationBlocker::ResearchOverrideUsed {
            gate: name.into(),
            flags: gate.override_flags.clone(),
        });
    }
}

fn inspect_hash(field: &str, hash: &ContentHash, blockers: &mut Vec<CertificationBlocker>) {
    if validate_sha256(hash).is_err() {
        blockers.push(CertificationBlocker::MalformedHash {
            field: field.into(),
        });
    }
}

fn validate_sha256(hash: &ContentHash) -> Result<(), ()> {
    let value = hash.as_str();
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(())
    }
}

fn resulting_grade(blockers: &[CertificationBlocker]) -> StrategyGrade {
    if blockers.is_empty() {
        return StrategyGrade::Certified;
    }
    if blockers.iter().any(|blocker| {
        matches!(
            blocker,
            CertificationBlocker::UnsupportedEvidenceSchema { .. }
                | CertificationBlocker::WrongEvidenceProtocol { .. }
                | CertificationBlocker::MalformedHash { .. }
                | CertificationBlocker::SplitPlanHashMismatch
                | CertificationBlocker::BindingMismatch { .. }
                | CertificationBlocker::ArtifactReused { .. }
                | CertificationBlocker::WrongGateProtocol { .. }
                | CertificationBlocker::ResearchOverrideUsed { .. }
                | CertificationBlocker::ValidationDataMismatch
                | CertificationBlocker::ValidationFailed
        )
    }) {
        return StrategyGrade::Scouted;
    }
    if blockers
        .iter()
        .any(|blocker| matches!(blocker, CertificationBlocker::IlluminationFailed))
    {
        return StrategyGrade::Accepted;
    }
    if blockers.iter().any(|blocker| {
        matches!(
            blocker,
            CertificationBlocker::ChallengeFailed
                | CertificationBlocker::SealedPlanMismatch
                | CertificationBlocker::SealedDataMismatch
                | CertificationBlocker::SealedOpenedBeforeShortlist
                | CertificationBlocker::SealedUsedInSelection
                | CertificationBlocker::SealedFinalFailed
        )
    }) {
        return StrategyGrade::Illuminated;
    }
    if blockers.iter().any(|blocker| {
        matches!(
            blocker,
            CertificationBlocker::JudgeFailed
                | CertificationBlocker::ExternalEngineIsNotMt5
                | CertificationBlocker::ExternalParityFailed
                | CertificationBlocker::ProtectiveOrdersMissing
                | CertificationBlocker::IndicatorParityFailed
        )
    }) {
        return StrategyGrade::Challenged;
    }
    StrategyGrade::ParityPassed
}

#[derive(Debug, Error)]
pub enum EvidenceError {
    #[error(
        "validation and sealed fractions must be finite, positive, and sum to less than one (validation={validation}, sealed={sealed})"
    )]
    InvalidSplitFractions { validation: f64, sealed: f64 },
    #[error(
        "at least three bars are required to create development, validation and sealed partitions; got {0}"
    )]
    InsufficientBars(usize),
    #[error("bar timestamps must be strictly increasing")]
    TimestampsNotStrictlyIncreasing,
    #[error("dataset identity does not match its ordered bar content")]
    DatasetHashMismatch,
    #[error("the requested fractions leave no development partition")]
    EmptyDevelopmentPartition,
    #[error("the final timestamp cannot be represented as an exclusive boundary")]
    TimestampOverflow,
    #[error("unsupported split-plan schema version {0}")]
    UnsupportedSplitSchema(u16),
    #[error("invalid {0} partition")]
    InvalidPartition(String),
    #[error("invalid SHA-256 identity for {0}")]
    InvalidPartitionHash(String),
    #[error("data partitions contain a gap or overlap")]
    PartitionGapOrOverlap,
    #[error("split plan declares {expected} bars but its partitions contain {actual}")]
    PartitionCountMismatch { expected: usize, actual: usize },
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_data::Bar;

    fn dataset(count: usize) -> BarDataset {
        let bars: Vec<_> = (0..count)
            .map(|index| Bar {
                timestamp_ms: index as i64 * 60_000,
                open: 1.0,
                high: 1.1,
                low: 0.9,
                close: 1.0 + index as f64 / 10_000.0,
                tick_volume: 100,
                real_volume: 0,
                spread_points: Some(10),
            })
            .collect();
        BarDataset {
            data_hash: bar_content_hash(&bars),
            bars,
            source_rows: count,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "UTC".into(),
        }
    }

    fn gate(binding: &EvidenceBinding, label: &str, protocol: &str) -> BoundGateEvidence {
        BoundGateEvidence {
            binding: binding.clone(),
            artifact_hash: ContentHash::sha256(label),
            protocol_version: protocol.into(),
            passed: true,
            override_flags: Vec::new(),
        }
    }

    fn passing_evidence(plan: &DataSplitPlan) -> CertificationEvidence {
        let candidate = EvidenceBinding {
            strategy_fingerprint: ContentHash::sha256("strategy"),
            broker_spec_hash: ContentHash::sha256("broker"),
        };
        let split_plan_hash = plan.content_hash().unwrap();
        CertificationEvidence {
            schema_version: CERTIFICATION_SCHEMA_VERSION,
            protocol_version: EVIDENCE_PROTOCOL_VERSION.into(),
            candidate: candidate.clone(),
            split_plan_hash: split_plan_hash.clone(),
            validation: DataGateEvidence {
                gate: gate(&candidate, "validation", VALIDATION_PROTOCOL),
                data_hash: plan.validation.data_hash.clone(),
            },
            illumination: gate(&candidate, "illumination", ILLUMINATION_PROTOCOL),
            challenge: gate(&candidate, "challenge", CHALLENGE_PROTOCOL),
            judge: gate(&candidate, "judge", JUDGE_PROTOCOL),
            external_parity: ExternalParityEvidence {
                gate: gate(&candidate, "external parity", EXTERNAL_PARITY_PROTOCOL),
                engine: ExternalEngine::Mt5StrategyTester,
                protective_orders_present: true,
            },
            indicator_parity: gate(&candidate, "indicator parity", INDICATOR_PARITY_PROTOCOL),
            sealed_final: SealedFinalEvidence {
                gate: gate(&candidate, "sealed", SEALED_FINAL_PROTOCOL),
                split_plan_hash,
                sealed_data_hash: plan.sealed_final.data_hash.clone(),
                shortlisted_before_open: true,
                used_in_selection_score: false,
            },
            incubation: None,
            evaluations_touched: 42,
            research_override_flags: Vec::new(),
        }
    }

    #[test]
    fn chronological_split_has_exact_non_overlapping_boundaries() {
        let dataset = dataset(100);
        let plan = DataSplitPlan::chronological(&dataset, 0.2, 0.2).unwrap();

        assert_eq!(plan.development.bar_count, 60);
        assert_eq!(plan.validation.bar_count, 20);
        assert_eq!(plan.sealed_final.bar_count, 20);
        assert_eq!(
            plan.development.end_timestamp_ms_exclusive,
            plan.validation.start_timestamp_ms
        );
        assert_eq!(
            plan.validation.end_timestamp_ms_exclusive,
            plan.sealed_final.start_timestamp_ms
        );
        assert_ne!(plan.validation.data_hash, plan.sealed_final.data_hash);
    }

    #[test]
    fn all_required_evidence_awards_certified() {
        let plan = DataSplitPlan::chronological(&dataset(100), 0.2, 0.2).unwrap();
        let decision = evaluate_certification(
            &passing_evidence(&plan),
            &plan,
            &CertificationPolicy::default(),
        )
        .unwrap();

        assert!(decision.passed);
        assert_eq!(decision.resulting_grade, StrategyGrade::Certified);
        assert!(decision.blockers.is_empty());
    }

    #[test]
    fn internal_judge_cannot_impersonate_external_mt5_parity() {
        let plan = DataSplitPlan::chronological(&dataset(100), 0.2, 0.2).unwrap();
        let mut evidence = passing_evidence(&plan);
        evidence.external_parity.engine = ExternalEngine::InternalJudge;
        let decision =
            evaluate_certification(&evidence, &plan, &CertificationPolicy::default()).unwrap();

        assert!(!decision.passed);
        assert_eq!(decision.resulting_grade, StrategyGrade::Challenged);
        assert!(
            decision
                .blockers
                .contains(&CertificationBlocker::ExternalEngineIsNotMt5)
        );
    }

    #[test]
    fn sealed_data_used_for_selection_demotes_to_illuminated() {
        let plan = DataSplitPlan::chronological(&dataset(100), 0.2, 0.2).unwrap();
        let mut evidence = passing_evidence(&plan);
        evidence.sealed_final.used_in_selection_score = true;
        let decision =
            evaluate_certification(&evidence, &plan, &CertificationPolicy::default()).unwrap();

        assert!(!decision.passed);
        assert_eq!(decision.resulting_grade, StrategyGrade::Illuminated);
        assert!(
            decision
                .blockers
                .contains(&CertificationBlocker::SealedUsedInSelection)
        );
    }

    #[test]
    fn evidence_for_another_strategy_is_rejected() {
        let plan = DataSplitPlan::chronological(&dataset(100), 0.2, 0.2).unwrap();
        let mut evidence = passing_evidence(&plan);
        evidence.judge.binding.strategy_fingerprint = ContentHash::sha256("other strategy");
        let decision =
            evaluate_certification(&evidence, &plan, &CertificationPolicy::default()).unwrap();

        assert!(!decision.passed);
        assert_eq!(decision.resulting_grade, StrategyGrade::Scouted);
        assert!(
            decision
                .blockers
                .contains(&CertificationBlocker::BindingMismatch {
                    gate: "judge".into()
                })
        );
    }

    #[test]
    fn any_research_override_blocks_certification() {
        let plan = DataSplitPlan::chronological(&dataset(100), 0.2, 0.2).unwrap();
        let mut evidence = passing_evidence(&plan);
        evidence
            .judge
            .override_flags
            .push("allow_execution_gaps".into());
        let decision =
            evaluate_certification(&evidence, &plan, &CertificationPolicy::default()).unwrap();

        assert!(!decision.passed);
        assert!(decision.blockers.iter().any(|blocker| matches!(
            blocker,
            CertificationBlocker::ResearchOverrideUsed { gate, .. } if gate == "judge"
        )));
    }

    #[test]
    fn one_artifact_cannot_satisfy_two_independent_gates() {
        let plan = DataSplitPlan::chronological(&dataset(100), 0.2, 0.2).unwrap();
        let mut evidence = passing_evidence(&plan);
        evidence.challenge.artifact_hash = evidence.judge.artifact_hash.clone();
        let decision =
            evaluate_certification(&evidence, &plan, &CertificationPolicy::default()).unwrap();

        assert!(!decision.passed);
        assert!(decision.blockers.iter().any(|blocker| matches!(
            blocker,
            CertificationBlocker::ArtifactReused { gates }
                if gates == &vec!["challenge".to_owned(), "judge".to_owned()]
        )));
    }
}
