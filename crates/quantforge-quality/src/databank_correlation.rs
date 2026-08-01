//! Pairwise equity-signature correlation filter for databank elites.
//!
//! SQX `DatabankFilterByCorrelation` keeps a diverse subset by discarding
//! candidates whose equity-path correlation with an already-kept elite exceeds
//! a threshold. QuantForge uses the same discover-time signature convention
//! (non-negative Pearson on return deltas).

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const DATABANK_CORRELATION_PROTOCOL: &str = "databank-correlation-v1";

#[derive(Debug, Error, PartialEq)]
pub enum CorrelationFilterError {
    #[error("maximum_correlation must be finite and in [0, 1]")]
    InvalidThreshold,
    #[error("elite is missing fingerprint / structural_fingerprint")]
    MissingFingerprint,
    #[error("elite `{0}` has no equity_signature")]
    MissingSignature(String),
    #[error("equity signatures have incompatible lengths ({0} vs {1})")]
    LengthMismatch(usize, usize),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationCandidate {
    pub fingerprint: String,
    pub equity_signature: Vec<f64>,
    /// Optional ranking key; higher is preferred when deciding who stays.
    #[serde(default)]
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RejectedPair {
    pub kept: String,
    pub rejected: String,
    pub correlation: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationFilterReport {
    pub protocol: String,
    pub maximum_correlation: f64,
    pub input_count: usize,
    pub kept_count: usize,
    pub rejected_count: usize,
    pub kept_fingerprints: Vec<String>,
    pub rejected: Vec<RejectedPair>,
    pub maximum_observed_pairwise_correlation: f64,
}

/// Greedy diversity filter: sort by score descending, keep when all pairwise
/// correlations with the kept set stay ≤ `maximum_correlation`.
pub fn filter_by_correlation(
    candidates: &[CorrelationCandidate],
    maximum_correlation: f64,
) -> Result<CorrelationFilterReport, CorrelationFilterError> {
    if !maximum_correlation.is_finite() || !(0.0..=1.0).contains(&maximum_correlation) {
        return Err(CorrelationFilterError::InvalidThreshold);
    }
    let mut ordered: Vec<&CorrelationCandidate> = candidates.iter().collect();
    ordered.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.fingerprint.cmp(&right.fingerprint))
    });

    let mut kept: Vec<&CorrelationCandidate> = Vec::new();
    let mut rejected = Vec::new();
    let mut max_obs = 0.0_f64;

    for candidate in ordered {
        if candidate.equity_signature.is_empty() {
            return Err(CorrelationFilterError::MissingSignature(
                candidate.fingerprint.clone(),
            ));
        }
        let mut block: Option<RejectedPair> = None;
        for existing in &kept {
            if existing.equity_signature.len() != candidate.equity_signature.len() {
                return Err(CorrelationFilterError::LengthMismatch(
                    existing.equity_signature.len(),
                    candidate.equity_signature.len(),
                ));
            }
            let corr = correlation(&existing.equity_signature, &candidate.equity_signature);
            max_obs = max_obs.max(corr);
            if corr > maximum_correlation + 1.0e-12 {
                block = Some(RejectedPair {
                    kept: existing.fingerprint.clone(),
                    rejected: candidate.fingerprint.clone(),
                    correlation: corr,
                });
                break;
            }
        }
        if let Some(pair) = block {
            rejected.push(pair);
        } else {
            kept.push(candidate);
        }
    }

    // Recompute max among kept pairs only for the report headline.
    let mut kept_max = 0.0_f64;
    for left in 0..kept.len() {
        for right in (left + 1)..kept.len() {
            kept_max = kept_max.max(correlation(
                &kept[left].equity_signature,
                &kept[right].equity_signature,
            ));
        }
    }

    Ok(CorrelationFilterReport {
        protocol: DATABANK_CORRELATION_PROTOCOL.into(),
        maximum_correlation,
        input_count: candidates.len(),
        kept_count: kept.len(),
        rejected_count: rejected.len(),
        kept_fingerprints: kept.iter().map(|c| c.fingerprint.clone()).collect(),
        rejected,
        maximum_observed_pairwise_correlation: if kept.len() >= 2 { kept_max } else { max_obs },
    })
}

/// Parse elite JSON objects into correlation candidates.
pub fn candidates_from_values(items: &[Value]) -> Result<Vec<CorrelationCandidate>, CorrelationFilterError> {
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let fingerprint = item
            .get("fingerprint")
            .or_else(|| item.get("structural_fingerprint"))
            .and_then(|v| v.as_str())
            .ok_or(CorrelationFilterError::MissingFingerprint)?
            .to_string();
        let signature = item
            .get("equity_signature")
            .or_else(|| item.get("equitySignature"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| CorrelationFilterError::MissingSignature(fingerprint.clone()))?
            .iter()
            .filter_map(|v| v.as_f64())
            .collect::<Vec<_>>();
        if signature.is_empty() {
            return Err(CorrelationFilterError::MissingSignature(fingerprint));
        }
        let score = item
            .get("evidence")
            .or_else(|| item.get("score"))
            .and_then(|v| {
                v.as_f64().or_else(|| {
                    v.as_object()
                        .and_then(|m| m.get("total"))
                        .and_then(|t| t.as_f64())
                })
            })
            .unwrap_or(0.0);
        out.push(CorrelationCandidate {
            fingerprint,
            equity_signature: signature,
            score,
        });
    }
    Ok(out)
}

fn correlation(left: &[f64], right: &[f64]) -> f64 {
    let length = left.len().min(right.len());
    if length < 2 {
        return 0.0;
    }
    let left = &left[..length];
    let right = &right[..length];
    let left_mean = left.iter().sum::<f64>() / length as f64;
    let right_mean = right.iter().sum::<f64>() / length as f64;
    let mut covariance = 0.0;
    let mut left_variance = 0.0;
    let mut right_variance = 0.0;
    for (left, right) in left.iter().zip(right) {
        let left_delta = left - left_mean;
        let right_delta = right - right_mean;
        covariance += left_delta * right_delta;
        left_variance += left_delta * left_delta;
        right_variance += right_delta * right_delta;
    }
    let denominator = (left_variance * right_variance).sqrt();
    if denominator <= f64::EPSILON {
        0.0
    } else {
        // Discover archive uses max(0) — keep the same convention for databank actions.
        (covariance / denominator).clamp(-1.0, 1.0).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_diverse_signatures() {
        let candidates = vec![
            CorrelationCandidate {
                fingerprint: "a".into(),
                equity_signature: vec![1.0, 2.0, 3.0, 4.0],
                score: 10.0,
            },
            CorrelationCandidate {
                fingerprint: "b".into(),
                equity_signature: vec![1.0, 2.0, 3.0, 4.1],
                score: 9.0,
            },
            CorrelationCandidate {
                fingerprint: "c".into(),
                equity_signature: vec![4.0, 3.0, 2.0, 1.0],
                score: 8.0,
            },
        ];
        let report = filter_by_correlation(&candidates, 0.95).unwrap();
        assert_eq!(report.kept_fingerprints, vec!["a".to_string(), "c".to_string()]);
        assert_eq!(report.rejected_count, 1);
        assert_eq!(report.rejected[0].rejected, "b");
    }

    #[test]
    fn parses_elite_json() {
        let items = vec![
            json!({
                "fingerprint": "fp1",
                "equity_signature": [0.1, -0.2, 0.3],
                "evidence": { "total": 12.5 }
            }),
            json!({
                "structural_fingerprint": "fp2",
                "equitySignature": [0.1, -0.2, 0.25],
                "score": 3.0
            }),
        ];
        let candidates = candidates_from_values(&items).unwrap();
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].score, 12.5);
        assert_eq!(candidates[1].fingerprint, "fp2");
    }
}
