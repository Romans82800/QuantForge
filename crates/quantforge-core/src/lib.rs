//! Shared deterministic primitives used by every QuantForge crate.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;

pub const PRODUCT_NAME: &str = "QuantForge";
pub const MANIFEST_SCHEMA_VERSION: u16 = 1;
pub const STRATEGY_IR_VERSION: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn sha256(bytes: impl AsRef<[u8]>) -> Self {
        let digest = Sha256::digest(bytes.as_ref());
        Self(hex::encode(digest))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Error)]
pub enum HashError {
    #[error("value cannot be represented as stable JSON: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Hashes a serializable value.
///
/// Struct field order and ordered map types are stable. Callers must
/// canonicalize commutative collections and avoid unordered maps first.
pub fn stable_json_hash<T: Serialize>(value: &T) -> Result<ContentHash, HashError> {
    Ok(ContentHash::sha256(serde_json::to_vec(value)?))
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FloatPolicy {
    pub parameter_quantum: f64,
    pub score_quantum: f64,
}

impl Default for FloatPolicy {
    fn default() -> Self {
        Self {
            parameter_quantum: 1.0e-6,
            score_quantum: 1.0e-9,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum FloatPolicyError {
    #[error("value must be finite")]
    NonFiniteValue,
    #[error("quantum must be finite and greater than zero")]
    InvalidQuantum,
}

pub fn quantize(value: f64, quantum: f64) -> Result<f64, FloatPolicyError> {
    if !value.is_finite() {
        return Err(FloatPolicyError::NonFiniteValue);
    }
    if !quantum.is_finite() || quantum <= 0.0 {
        return Err(FloatPolicyError::InvalidQuantum);
    }

    let quantized = (value / quantum).round() * quantum;
    if quantized == 0.0 {
        Ok(0.0)
    } else {
        Ok(quantized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn stable_json_hash_is_independent_of_btree_insertion_order() {
        let left = BTreeMap::from([("alpha", 1), ("beta", 2)]);
        let right = BTreeMap::from([("beta", 2), ("alpha", 1)]);

        assert_eq!(
            stable_json_hash(&left).unwrap(),
            stable_json_hash(&right).unwrap()
        );
    }

    #[test]
    fn quantization_normalizes_negative_zero() {
        assert_eq!(quantize(-0.000_000_1, 0.000_001).unwrap().to_bits(), 0);
    }
}
