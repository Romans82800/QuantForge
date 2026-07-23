use quantforge_core::{ContentHash, HashError};
use quantforge_ir::IrError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mql5ExportConfig {
    pub expert_name: String,
    pub expert_directory: String,
    pub timeframe: String,
    pub magic: u64,
    pub deviation_points: u32,
    pub max_spread_points: Option<f64>,
    pub estimated_slippage_points_per_side: f64,
    pub commission_per_lot_round_turn: f64,
    pub allow_live_trading_default: bool,
    pub tester: TesterConfig,
}

impl Default for Mql5ExportConfig {
    fn default() -> Self {
        Self {
            expert_name: "QuantForgeStrategy".into(),
            expert_directory: "QuantForge".into(),
            timeframe: "M15".into(),
            magic: 42_424_242,
            deviation_points: 10,
            max_spread_points: None,
            estimated_slippage_points_per_side: 0.0,
            commission_per_lot_round_turn: 0.0,
            allow_live_trading_default: false,
            tester: TesterConfig::default(),
        }
    }
}

impl Mql5ExportConfig {
    pub(crate) fn validate(&self) -> Result<(), ExportError> {
        if self.expert_name.is_empty()
            || !self
                .expert_name
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || value == '_')
        {
            return Err(ExportError::InvalidConfig(
                "expert_name must contain only ASCII letters, digits and underscores".into(),
            ));
        }
        if self.expert_directory.is_empty()
            || !self
                .expert_directory
                .chars()
                .all(|value| value.is_ascii_alphanumeric() || value == '_')
        {
            return Err(ExportError::InvalidConfig(
                "expert_directory must contain only ASCII letters, digits and underscores".into(),
            ));
        }
        if self.timeframe.is_empty()
            || !self
                .timeframe
                .chars()
                .all(|value| value.is_ascii_alphanumeric())
        {
            return Err(ExportError::InvalidConfig(
                "timeframe must be an MT5 timeframe token such as M15 or H1".into(),
            ));
        }
        if self.magic == 0 {
            return Err(ExportError::InvalidConfig(
                "magic must be greater than zero".into(),
            ));
        }
        for (name, value) in [
            (
                "estimated_slippage_points_per_side",
                self.estimated_slippage_points_per_side,
            ),
            (
                "commission_per_lot_round_turn",
                self.commission_per_lot_round_turn,
            ),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(ExportError::InvalidConfig(format!(
                    "{name} must be finite and non-negative"
                )));
            }
        }
        if self
            .max_spread_points
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(ExportError::InvalidConfig(
                "max_spread_points must be finite and non-negative".into(),
            ));
        }
        if self.allow_live_trading_default {
            return Err(ExportError::InvalidConfig(
                "generated exports must default to live trading disabled".into(),
            ));
        }
        self.tester.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TesterConfig {
    pub from_date: Option<String>,
    pub to_date: Option<String>,
    pub deposit: f64,
    pub currency: String,
    pub leverage: u32,
    /// MT5 model 1 is one-minute OHLC; model 4 is every tick based on real ticks.
    pub model: u8,
}

impl Default for TesterConfig {
    fn default() -> Self {
        Self {
            from_date: None,
            to_date: None,
            deposit: 100_000.0,
            currency: "USD".into(),
            leverage: 100,
            model: 1,
        }
    }
}

impl TesterConfig {
    fn validate(&self) -> Result<(), ExportError> {
        if !self.deposit.is_finite() || self.deposit <= 0.0 {
            return Err(ExportError::InvalidConfig(
                "tester deposit must be finite and greater than zero".into(),
            ));
        }
        if self.currency.is_empty()
            || !self
                .currency
                .chars()
                .all(|value| value.is_ascii_alphabetic())
        {
            return Err(ExportError::InvalidConfig(
                "tester currency must contain only ASCII letters".into(),
            ));
        }
        if self.leverage == 0 || !matches!(self.model, 0..=4) {
            return Err(ExportError::InvalidConfig(
                "tester leverage must be positive and model must be between 0 and 4".into(),
            ));
        }
        for (name, value) in [
            ("from_date", self.from_date.as_deref()),
            ("to_date", self.to_date.as_deref()),
        ] {
            if let Some(value) = value
                && !valid_tester_date(value)
            {
                return Err(ExportError::InvalidConfig(format!(
                    "{name} must use YYYY.MM.DD"
                )));
            }
        }
        Ok(())
    }
}

fn valid_tester_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'.'
        && value.as_bytes()[7] == b'.'
        && value
            .chars()
            .enumerate()
            .all(|(index, value)| matches!(index, 4 | 7) || value.is_ascii_digit())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportEvidenceCard {
    pub schema_version: u16,
    pub target: String,
    pub strategy_fingerprint: ContentHash,
    pub broker_spec_hash: ContentHash,
    pub source_hash: ContentHash,
    pub strategy_ir_version: u16,
    pub expert_name: String,
    pub symbol: String,
    pub timeframe: String,
    pub live_trading_default: bool,
    pub mandatory_stop_loss: bool,
    pub mandatory_take_profit: bool,
    pub parity_deals_file: String,
    pub parity_equity_file: String,
    pub parity_metadata_file: String,
    pub config: Mql5ExportConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBundle {
    pub source: String,
    pub set_file: String,
    pub tester_ini: String,
    pub evidence: ExportEvidenceCard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetaEditorConfig {
    pub executable: PathBuf,
    pub wine_binary: Option<PathBuf>,
    pub wine_prefix: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileReport {
    pub success: bool,
    pub process_exit_code: Option<i32>,
    pub errors: Option<usize>,
    pub warnings: Option<usize>,
    pub source_path: PathBuf,
    pub binary_path: PathBuf,
    pub log_path: PathBuf,
    pub log_excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalConfig {
    pub executable: PathBuf,
    pub wine_binary: Option<PathBuf>,
    pub wine_prefix: Option<PathBuf>,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TesterRunReport {
    pub success: bool,
    pub timed_out: bool,
    pub process_exit_code: Option<i32>,
    pub elapsed_milliseconds: u128,
    pub tester_ini: PathBuf,
    pub deals_path: PathBuf,
    pub equity_path: PathBuf,
    pub metadata_path: PathBuf,
    pub deals_fresh: bool,
    pub equity_fresh: bool,
    pub metadata_fresh: bool,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("invalid MQL5 export configuration: {0}")]
    InvalidConfig(String),
    #[error("MetaEditor compilation failed: {0}")]
    Compilation(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ir(#[from] IrError),
    #[error(transparent)]
    Broker(#[from] quantforge_broker::BrokerSpecError),
    #[error(transparent)]
    Hash(#[from] HashError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
