use crate::archive::niche_key;
use crate::{DATABANK_SCHEMA_VERSION, GRAMMAR_VERSION};
use quantforge_core::{ContentHash, FloatPolicy};
use quantforge_eval::{BacktestMetrics, EvalError, ScoutConfig};
use quantforge_ir::{IrError, StrategyIr};
use quantforge_quality::MonteCarloReport;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GateConfig {
    pub minimum_trades: usize,
    pub maximum_drawdown_percent: f64,
    pub minimum_return_percent: f64,
    pub minimum_profit_factor: f64,
    /// Floor on MT5-style recovery factor (`net_profit / absolute equity DD`).
    /// Serialized as `minimum_return_drawdown` for databank JSON compatibility.
    #[serde(
        default,
        rename = "minimum_return_drawdown",
        alias = "minimum_recovery_factor",
        skip_serializing_if = "is_zero"
    )]
    pub minimum_recovery_factor: f64,
}

impl Default for GateConfig {
    fn default() -> Self {
        // Scout defaults: loose enough for random search to fill the pot.
        Self {
            minimum_trades: 10,
            maximum_drawdown_percent: 40.0,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            minimum_recovery_factor: 0.0,
        }
    }
}

impl GateConfig {
    /// Stricter thresholds applied only when depositing into the databank.
    pub fn deposit_defaults() -> Self {
        Self {
            minimum_trades: 20,
            maximum_drawdown_percent: 30.0,
            minimum_return_percent: 0.0,
            minimum_profit_factor: 1.0,
            minimum_recovery_factor: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrecisionGateConfig {
    /// A positive M1 return must retain at least this fraction of the H1
    /// screening return. Values above one are allowed and mean M1 improved.
    pub minimum_return_retention: f64,
}

/// A discrete numeric gene ladder. Values are sampled and mutated only on this
/// grid, which makes the researcher-selected search space explicit and auditable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRange {
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
}

impl SearchRange {
    pub const fn new(minimum: f64, maximum: f64, step: f64) -> Self {
        Self {
            minimum,
            maximum,
            step,
        }
    }

    pub fn validate(&self, name: &str) -> Result<(), DiscoverError> {
        if !self.minimum.is_finite()
            || !self.maximum.is_finite()
            || !self.step.is_finite()
            || self.step <= 0.0
            || self.maximum < self.minimum
        {
            return Err(DiscoverError::InvalidConfig(format!(
                "search range `{name}` requires finite min/max and a positive step"
            )));
        }
        let steps = (self.maximum - self.minimum) / self.step;
        if steps > 1_000.0 {
            return Err(DiscoverError::InvalidConfig(format!(
                "search range `{name}` has more than 1,000 discrete values"
            )));
        }
        Ok(())
    }
}

/// User-controlled numeric gene space. This profile is stored inside every
/// databank and is immutable on continuation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchRangeProfile {
    #[serde(rename = "indicatorPeriod", alias = "indicator_period")]
    pub indicator_period: SearchRange,
    #[serde(rename = "atrPeriod", alias = "atr_period")]
    pub atr_period: SearchRange,
    #[serde(rename = "atrStopMultiple", alias = "atr_stop_multiple")]
    pub atr_stop_multiple: SearchRange,
    #[serde(rename = "atrTargetMultiple", alias = "atr_target_multiple")]
    pub atr_target_multiple: SearchRange,
    #[serde(rename = "riskTargetMultiple", alias = "risk_target_multiple")]
    pub risk_target_multiple: SearchRange,
    #[serde(rename = "pendingDistanceAtr", alias = "pending_distance_atr")]
    pub pending_distance_atr: SearchRange,
    #[serde(rename = "pendingExpiryBars", alias = "pending_expiry_bars")]
    pub pending_expiry_bars: SearchRange,
    #[serde(rename = "timeStopBars", alias = "time_stop_bars")]
    pub time_stop_bars: SearchRange,
    #[serde(rename = "rsiUpper", alias = "rsi_upper")]
    pub rsi_upper: SearchRange,
    #[serde(rename = "rsiLower", alias = "rsi_lower")]
    pub rsi_lower: SearchRange,
    #[serde(rename = "adxThreshold", alias = "adx_threshold")]
    pub adx_threshold: SearchRange,
    #[serde(rename = "rocThreshold", alias = "roc_threshold")]
    pub roc_threshold: SearchRange,
    #[serde(rename = "percentileLow", alias = "percentile_low")]
    pub percentile_low: SearchRange,
    #[serde(rename = "zscoreThreshold", alias = "zscore_threshold")]
    pub zscore_threshold: SearchRange,
    #[serde(rename = "impulseBodyRatio", alias = "impulse_body_ratio")]
    pub impulse_body_ratio: SearchRange,
    #[serde(rename = "impulseCloseLocation", alias = "impulse_close_location")]
    pub impulse_close_location: SearchRange,
    #[serde(rename = "atrPercentileMax", alias = "atr_percentile_max")]
    pub atr_percentile_max: SearchRange,
    #[serde(rename = "atrPercentileLookback", alias = "atr_percentile_lookback")]
    pub atr_percentile_lookback: SearchRange,
    #[serde(rename = "sessionStartHour", alias = "session_start_hour")]
    pub session_start_hour: SearchRange,
    #[serde(rename = "sessionRangeBars", alias = "session_range_bars")]
    pub session_range_bars: SearchRange,
    #[serde(rename = "swingBars", alias = "swing_bars")]
    pub swing_bars: SearchRange,
    #[serde(rename = "baseBars", alias = "base_bars")]
    pub base_bars: SearchRange,
    #[serde(
        rename = "liquiditySweepThreshold",
        alias = "liquidity_sweep_threshold"
    )]
    pub liquidity_sweep_threshold: SearchRange,
}

impl Default for SearchRangeProfile {
    /// Compact H1/M15 plateau: bounded ranges, not fixed constants.  The
    /// researcher can widen these in the settings wall, while the default
    /// still keeps the first pass on a local volatility/indicator plateau.
    fn default() -> Self {
        Self::h1_compact()
    }
}

impl SearchRangeProfile {
    /// Current QuantForge H1/M15 default.  ATR lookback and target geometry
    /// are genes; there is no implicit ``14 ATR / 1.5R`` rule.
    pub fn h1_compact() -> Self {
        Self {
            indicator_period: SearchRange::new(10.0, 20.0, 1.0),
            atr_period: SearchRange::new(10.0, 20.0, 1.0),
            atr_stop_multiple: SearchRange::new(1.0, 4.0, 0.25),
            atr_target_multiple: SearchRange::new(1.0, 6.0, 0.5),
            risk_target_multiple: SearchRange::new(0.75, 4.5, 0.25),
            pending_distance_atr: SearchRange::new(0.25, 2.0, 0.25),
            pending_expiry_bars: SearchRange::new(2.0, 8.0, 1.0),
            time_stop_bars: SearchRange::new(4.0, 16.0, 1.0),
            rsi_upper: SearchRange::new(52.0, 65.0, 1.0),
            rsi_lower: SearchRange::new(20.0, 40.0, 1.0),
            adx_threshold: SearchRange::new(20.0, 35.0, 1.0),
            roc_threshold: SearchRange::new(0.1, 2.5, 0.1),
            percentile_low: SearchRange::new(5.0, 25.0, 1.0),
            zscore_threshold: SearchRange::new(1.0, 2.5, 0.1),
            impulse_body_ratio: SearchRange::new(0.55, 0.75, 0.05),
            impulse_close_location: SearchRange::new(0.70, 0.90, 0.05),
            atr_percentile_max: SearchRange::new(15.0, 35.0, 1.0),
            atr_percentile_lookback: SearchRange::new(20.0, 60.0, 20.0),
            session_start_hour: SearchRange::new(7.0, 14.0, 1.0),
            session_range_bars: SearchRange::new(2.0, 4.0, 1.0),
            swing_bars: SearchRange::new(2.0, 4.0, 1.0),
            base_bars: SearchRange::new(2.0, 4.0, 1.0),
            liquidity_sweep_threshold: SearchRange::new(0.0, 0.5, 0.5),
        }
    }

    /// SQX-style "random periods and parameters within reason": wider gene
    /// space (indicator periods typically 10–50, free ATR period, broader
    /// stops/targets). Does not replace H1 compact — callers pick which preset
    /// to seal into a new databank.
    pub fn sqx_random() -> Self {
        Self {
            indicator_period: SearchRange::new(10.0, 50.0, 1.0),
            atr_period: SearchRange::new(7.0, 28.0, 1.0),
            atr_stop_multiple: SearchRange::new(1.0, 5.0, 0.25),
            atr_target_multiple: SearchRange::new(1.5, 8.0, 0.5),
            risk_target_multiple: SearchRange::new(1.0, 5.0, 0.25),
            pending_distance_atr: SearchRange::new(0.25, 3.0, 0.25),
            pending_expiry_bars: SearchRange::new(1.0, 12.0, 1.0),
            time_stop_bars: SearchRange::new(2.0, 48.0, 1.0),
            rsi_upper: SearchRange::new(55.0, 80.0, 1.0),
            rsi_lower: SearchRange::new(20.0, 45.0, 1.0),
            adx_threshold: SearchRange::new(15.0, 40.0, 1.0),
            roc_threshold: SearchRange::new(0.05, 5.0, 0.05),
            percentile_low: SearchRange::new(5.0, 30.0, 1.0),
            zscore_threshold: SearchRange::new(0.5, 3.0, 0.1),
            impulse_body_ratio: SearchRange::new(0.50, 0.85, 0.05),
            impulse_close_location: SearchRange::new(0.60, 0.95, 0.05),
            atr_percentile_max: SearchRange::new(10.0, 50.0, 1.0),
            atr_percentile_lookback: SearchRange::new(20.0, 100.0, 10.0),
            session_start_hour: SearchRange::new(0.0, 20.0, 1.0),
            session_range_bars: SearchRange::new(1.0, 6.0, 1.0),
            swing_bars: SearchRange::new(2.0, 8.0, 1.0),
            base_bars: SearchRange::new(2.0, 8.0, 1.0),
            liquidity_sweep_threshold: SearchRange::new(0.0, 1.0, 0.25),
        }
    }

    pub fn validate(&self) -> Result<(), DiscoverError> {
        for (name, range) in [
            ("indicator_period", &self.indicator_period),
            ("atr_period", &self.atr_period),
            ("atr_stop_multiple", &self.atr_stop_multiple),
            ("atr_target_multiple", &self.atr_target_multiple),
            ("risk_target_multiple", &self.risk_target_multiple),
            ("pending_distance_atr", &self.pending_distance_atr),
            ("pending_expiry_bars", &self.pending_expiry_bars),
            ("time_stop_bars", &self.time_stop_bars),
            ("rsi_upper", &self.rsi_upper),
            ("rsi_lower", &self.rsi_lower),
            ("adx_threshold", &self.adx_threshold),
            ("roc_threshold", &self.roc_threshold),
            ("percentile_low", &self.percentile_low),
            ("zscore_threshold", &self.zscore_threshold),
            ("impulse_body_ratio", &self.impulse_body_ratio),
            ("impulse_close_location", &self.impulse_close_location),
            ("atr_percentile_max", &self.atr_percentile_max),
            ("atr_percentile_lookback", &self.atr_percentile_lookback),
            ("session_start_hour", &self.session_start_hour),
            ("session_range_bars", &self.session_range_bars),
            ("swing_bars", &self.swing_bars),
            ("base_bars", &self.base_bars),
            ("liquidity_sweep_threshold", &self.liquidity_sweep_threshold),
        ] {
            range.validate(name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod search_range_profile_tests {
    use super::*;

    #[test]
    fn accepts_browser_camel_case_and_existing_snake_case_profiles() {
        let camel: SearchRangeProfile = serde_json::from_value(serde_json::json!({
            "indicatorPeriod": { "minimum": 12.0, "maximum": 12.0, "step": 1.0 },
            "adxThreshold": { "minimum": 24.0, "maximum": 24.0, "step": 1.0 }
        }))
        .expect("browser profile must deserialize");
        assert_eq!(camel.indicator_period, SearchRange::new(12.0, 12.0, 1.0));
        assert_eq!(camel.adx_threshold, SearchRange::new(24.0, 24.0, 1.0));

        let snake: SearchRangeProfile = serde_json::from_value(serde_json::json!({
            "indicator_period": { "minimum": 16.0, "maximum": 16.0, "step": 1.0 },
            "adx_threshold": { "minimum": 28.0, "maximum": 28.0, "step": 1.0 }
        }))
        .expect("existing local profile must deserialize");
        assert_eq!(snake.indicator_period, SearchRange::new(16.0, 16.0, 1.0));
        assert_eq!(snake.adx_threshold, SearchRange::new(28.0, 28.0, 1.0));

        let encoded = serde_json::to_value(camel).expect("profile must serialize");
        assert!(encoded.get("indicatorPeriod").is_some());
        assert!(encoded.get("indicator_period").is_none());
    }

    #[test]
    fn built_in_presets_validate_and_stay_distinct() {
        let h1 = SearchRangeProfile::h1_compact();
        let sqx = SearchRangeProfile::sqx_random();
        h1.validate().expect("H1 compact must validate");
        sqx.validate().expect("SQX random must validate");
        assert_eq!(h1.indicator_period.maximum, 20.0);
        assert_eq!(sqx.indicator_period.maximum, 50.0);
        assert!(sqx.atr_period.maximum > h1.atr_period.maximum);
        assert_eq!(SearchRangeProfile::default(), h1);
    }
}

impl Default for PrecisionGateConfig {
    fn default() -> Self {
        Self {
            minimum_return_retention: 0.80,
        }
    }
}

/// Named institutional search recipe (locked grammar + mirror + ATR14).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SearchFamily {
    #[default]
    TrendPullback,
    MomentumBurst,
    DonchianBreakout,
    MeanReversionBand,
    ZScoreReversion,
    SessionOrb,
    ImpulseCandle,
    VolSqueezeBreak,
    SupplyDemandReclaim,
    SweepReclaim,
    /// Family-free typed block search across the full condition library.
    Universal,
}

impl SearchFamily {
    pub const ALL: [Self; 11] = [
        Self::TrendPullback,
        Self::MomentumBurst,
        Self::DonchianBreakout,
        Self::MeanReversionBand,
        Self::ZScoreReversion,
        Self::SessionOrb,
        Self::ImpulseCandle,
        Self::VolSqueezeBreak,
        Self::SupplyDemandReclaim,
        Self::SweepReclaim,
        Self::Universal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TrendPullback => "TrendPullback",
            Self::MomentumBurst => "MomentumBurst",
            Self::DonchianBreakout => "DonchianBreakout",
            Self::MeanReversionBand => "MeanReversionBand",
            Self::ZScoreReversion => "ZScoreReversion",
            Self::SessionOrb => "SessionOrb",
            Self::ImpulseCandle => "ImpulseCandle",
            Self::VolSqueezeBreak => "VolSqueezeBreak",
            Self::SupplyDemandReclaim => "SupplyDemandReclaim",
            Self::SweepReclaim => "SweepReclaim",
            Self::Universal => "UniversalGrammar",
        }
    }

    pub fn recipe_summary(self) -> &'static str {
        match self {
            Self::TrendPullback => {
                "EMA/SMA structure + optional ROC · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::MomentumBurst => {
                "RSI/ROC thrust atoms · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::DonchianBreakout => {
                "Donchian/HH-LL + optional SMA · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::MeanReversionBand => {
                "RSI + percentile fades · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::ZScoreReversion => {
                "Close z-score extremes · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::SessionOrb => {
                "Broker-local opening-range breakout · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::ImpulseCandle => {
                "Body/range + close-in-bar thrust · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::VolSqueezeBreak => {
                "ATR-percentile squeeze then break · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::SupplyDemandReclaim => {
                "Swing-base zone reclaim · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::SweepReclaim => {
                "Liquidity sweep then reclaim · max 3 atoms · ATR14 · next-open market · mirror"
            }
            Self::Universal => {
                "Any typed factors · 2–3 mirrored entry conditions · 1–3 side-specific exits · closed-bar shifts"
            }
        }
    }

    pub fn style(self) -> FamilyStyle {
        match self {
            Self::TrendPullback => FamilyStyle::TrendPullback,
            Self::MomentumBurst => FamilyStyle::MomentumBurst,
            Self::DonchianBreakout => FamilyStyle::DonchianBreakout,
            Self::MeanReversionBand => FamilyStyle::MeanReversionBand,
            Self::ZScoreReversion => FamilyStyle::ZScoreReversion,
            Self::SessionOrb => FamilyStyle::SessionOrb,
            Self::ImpulseCandle => FamilyStyle::ImpulseCandle,
            Self::VolSqueezeBreak => FamilyStyle::VolSqueezeBreak,
            Self::SupplyDemandReclaim => FamilyStyle::SupplyDemandReclaim,
            Self::SweepReclaim => FamilyStyle::SweepReclaim,
            Self::Universal => FamilyStyle::Universal,
        }
    }

    pub fn from_style(style: FamilyStyle) -> Self {
        match style {
            FamilyStyle::TrendPullback => Self::TrendPullback,
            FamilyStyle::MomentumBurst => Self::MomentumBurst,
            FamilyStyle::DonchianBreakout => Self::DonchianBreakout,
            FamilyStyle::MeanReversionBand => Self::MeanReversionBand,
            FamilyStyle::ZScoreReversion => Self::ZScoreReversion,
            FamilyStyle::SessionOrb => Self::SessionOrb,
            FamilyStyle::ImpulseCandle => Self::ImpulseCandle,
            FamilyStyle::VolSqueezeBreak => Self::VolSqueezeBreak,
            FamilyStyle::SupplyDemandReclaim => Self::SupplyDemandReclaim,
            FamilyStyle::SweepReclaim => Self::SweepReclaim,
            FamilyStyle::Universal => Self::Universal,
        }
    }

    pub fn spec(self) -> SearchFamilySpec {
        SearchFamilySpec {
            family: self,
            max_atoms: 3,
            atr_period: crate::FROZEN_ATR_PERIOD,
            indicator_periods: vec![10, 14, 20],
            market_only: true,
            pending_only: false,
            mirror_sides: true,
            complexity_penalty_weight: 0.01,
        }
    }
}

/// Immutable family-free grammar bounds sealed into a databank.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UniversalGrammarConfig {
    pub minimum_entry_conditions: usize,
    pub maximum_entry_conditions: usize,
    pub minimum_exit_conditions: usize,
    pub maximum_exit_conditions: usize,
    /// Oldest/youngest completed decision-bar shifts. Shift zero is forbidden.
    pub minimum_shift: u16,
    pub maximum_shift: u16,
}

impl Default for UniversalGrammarConfig {
    fn default() -> Self {
        Self {
            minimum_entry_conditions: 2,
            maximum_entry_conditions: 4,
            minimum_exit_conditions: 1,
            maximum_exit_conditions: 3,
            minimum_shift: 1,
            maximum_shift: 3,
        }
    }
}

impl UniversalGrammarConfig {
    /// Widest entry-condition count the grammar will build. Four is the ceiling
    /// because each extra mirrored block is another degree of freedom to overfit.
    pub const MAX_ENTRY_CONDITIONS: usize = 4;

    fn validate(&self) -> Result<(), DiscoverError> {
        if self.minimum_entry_conditions < 2
            || self.maximum_entry_conditions > Self::MAX_ENTRY_CONDITIONS
            || self.minimum_entry_conditions > self.maximum_entry_conditions
        {
            return Err(DiscoverError::InvalidConfig(
                "universal entry conditions must be an ordered range within 2..=4".into(),
            ));
        }
        if self.minimum_exit_conditions == 0
            || self.maximum_exit_conditions > 3
            || self.minimum_exit_conditions > self.maximum_exit_conditions
        {
            return Err(DiscoverError::InvalidConfig(
                "universal exit conditions must be an ordered range within 1..=3".into(),
            ));
        }
        if self.minimum_shift == 0
            || self.minimum_shift > self.maximum_shift
            || self.maximum_shift > 50
        {
            return Err(DiscoverError::InvalidConfig(
                "universal shifts must be an ordered closed-bar range within 1..=50".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchFamilySpec {
    pub family: SearchFamily,
    pub max_atoms: usize,
    pub atr_period: u16,
    pub indicator_periods: Vec<u16>,
    pub market_only: bool,
    /// When true, seeds/mutations use stop or limit entries only (no market).
    #[serde(default)]
    pub pending_only: bool,
    pub mirror_sides: bool,
    pub complexity_penalty_weight: f64,
}

/// Fast Scout = cheap H1 IS/OOS1; Full Harvest = multi-elite stacking + Selected-TF;
/// Quota Harvest = ~20 databank elites per family without chasing pot 300.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverRunMode {
    FastScout,
    #[default]
    FullHarvest,
    /// Seed-heavy, softer param neighborhood, stop when databank hits quota.
    QuotaHarvest,
}

/// Named gate outcome persisted on elites for evidence-first promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Warn in UI when planned evaluations exceed this (Veritas-style honesty).
pub const TRIAL_BUDGET_WARNING: u64 = 1_500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoverConfig {
    pub initial_candidates: usize,
    pub batch_size: usize,
    pub correlation_threshold: f64,
    pub novelty_weight: f64,
    pub tournament_size: usize,
    pub structural_mutation_probability: f64,
    pub seed: u64,
    /// Immutable grammar bounds sealed into the databank: how many mirrored
    /// entry conditions, how many exit conditions, and the closed-bar shifts.
    #[serde(default)]
    pub universal_grammar: UniversalGrammarConfig,
    /// Fast Scout vs Full Harvest knobs applied at start.
    #[serde(default)]
    pub run_mode: DiscoverRunMode,
    /// Stop evolving once the accepted pot reaches this size (`None` = no early stop).
    #[serde(default)]
    pub early_stop_pot_elites: Option<usize>,
    /// Stop evolving once the databank reaches this many elites (`None` = no early stop).
    #[serde(default)]
    pub target_databank_elites: Option<usize>,
    /// Soft warning threshold for planned trial budget (UI).
    #[serde(default = "default_trial_budget_warning")]
    pub trial_budget_warning: u64,
    /// Early H1/IS screen used during random search (cheap reject).
    pub gates: GateConfig,
    /// Final metrics required to enter or replace an elite in the pot.
    #[serde(default = "GateConfig::deposit_defaults")]
    pub deposit_gates: GateConfig,
    pub precision: PrecisionGateConfig,
    /// Immutable numeric search space for indicators, stops and management genes.
    #[serde(default)]
    pub search_ranges: SearchRangeProfile,
    /// Required OOS1 expectancy retention after the complete Development
    /// promotion battery. OOS1 never contributes to breeding or ranking.
    #[serde(default = "default_oos1_expectancy_retention")]
    pub oos1_expectancy_retention: f64,
    /// Minimum M1 Development expectancy, expressed in R at the immutable
    /// fixed-risk amount. This is a post-breed databank gate and therefore
    /// never narrows the random-search reservoir.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub minimum_development_expectancy_r: f64,
    /// Preference / seal flag: databank elites are M1-promoted after breeding.
    /// Discover always runs the M1 fidelity + robustness battery on the
    /// post-breed databank path (SQX structure); this does not gate the pot.
    #[serde(default = "default_require_m1_precision")]
    pub require_m1_precision: bool,
    /// Prefer stop/limit pendings, ATR/R SL-TP, no trailing/BE/partials, and a
    /// hard time stop of at most 16 bars — higher H1↔M1 agreement.
    #[serde(default = "default_simple_exits")]
    pub simple_exits: bool,
    /// Individually opt-in execution genes. Off in the high-parity Selected-TF
    /// baseline; enabling them widens search and makes M1/MT5 final gates more
    /// important — they do not block Discover breeding.
    #[serde(default)]
    pub allow_break_even: bool,
    #[serde(default)]
    pub allow_trailing_stops: bool,
    #[serde(default)]
    pub allow_partial_exits: bool,
    /// Entry order kinds the search may sample. At least one must stay enabled;
    /// disabling market makes the run pending-only (stop-only or limit-only).
    #[serde(default = "default_allow_market_entries")]
    pub allow_market_entries: bool,
    #[serde(default)]
    pub allow_stop_entries: bool,
    #[serde(default)]
    pub allow_limit_entries: bool,
    /// Mandatory portfolio protection applied to every generated strategy.
    /// When enabled, exposure is flattened at `end_of_day_hour` broker time.
    pub flatten_at_22: bool,
    /// Broker-local hour for end-of-day flatten (SQX default 23:00).
    #[serde(default = "default_end_of_day_hour")]
    pub end_of_day_hour: u8,
    /// Cap each strategy to one filled entry per broker-local calendar day.
    /// Improves H1↔M1 agreement and keeps trade counts in a swing-friendly band.
    #[serde(default = "default_max_one_entry_per_day")]
    pub max_one_entry_per_day: bool,
    /// Keep random-filling the initial accepted pot until it holds this many
    /// strategies, then unlock crossover/mutation from that pot. Databank
    /// Development robustness → M1 promotion starts only after this unlock.
    #[serde(
        default = "default_mutate_after_elites",
        alias = "mutate_after_generation"
    )]
    pub mutate_after_elites: usize,
    /// After breeding starts, this fraction of each batch remains fresh random seeds.
    #[serde(default = "default_random_fill_fraction")]
    pub random_fill_fraction: f64,
    /// Rayon worker threads for H1 scout / pot admission. `0` = auto
    /// (`available_parallelism − 1 − promotion workers`).
    #[serde(default = "default_worker_threads")]
    pub worker_threads: usize,
    /// Dedicated Rayon workers for post-breed databank promotion
    /// (Development CPCV/robustness → M1 → OOS1 validation). `0` = auto (2–4 threads). Scout keeps
    /// generating on `worker_threads` while this pool drains the queue.
    #[serde(default = "default_promotion_worker_threads")]
    pub promotion_worker_threads: usize,
    /// Max pot admissions waiting or running on the promotion pool. When full,
    /// Discover applies backpressure (pauses further enqueue / pot growth) rather
    /// than dropping elites or growing RAM without bound.
    #[serde(default = "default_promotion_queue_capacity")]
    pub promotion_queue_capacity: usize,
    /// After breeding unlocks, run the M1 walk-forward / Monte Carlo / ±param
    /// neighborhood battery before databank admission. Pot fill never waits on this.
    #[serde(default = "default_require_m1_robustness")]
    pub require_m1_robustness: bool,
    #[serde(default = "default_robustness_folds")]
    pub robustness_folds: usize,
    #[serde(default = "default_robustness_monte_carlo_trials")]
    pub robustness_monte_carlo_trials: usize,
    /// Moving-block length for Discover Monte Carlo trade resampling.
    #[serde(default = "default_robustness_monte_carlo_block_length")]
    pub robustness_monte_carlo_block_length: usize,
    /// Fraction of trades skipped on each MC path (SQX-style trade manipulation).
    #[serde(default = "default_robustness_monte_carlo_skip_trade_probability")]
    pub robustness_monte_carlo_skip_trade_probability: f64,
    /// P80 net-profit retention floor vs baseline (default 0.60).
    #[serde(default = "default_robustness_monte_carlo_p80_profit_retention")]
    pub robustness_monte_carlo_p80_profit_retention: f64,
    /// Cap on P95 MC drawdown as a multiple of baseline max DD (default 1.75).
    #[serde(default = "default_robustness_monte_carlo_max_drawdown_ratio")]
    pub robustness_monte_carlo_max_drawdown_ratio: f64,
    #[serde(default = "default_robustness_neighborhood_samples")]
    pub robustness_neighborhood_samples: usize,
    /// Size of the ±% jitter applied to every numeric gene when probing the
    /// local plateau. `0.20` matches the SQX parameter-sensitivity default.
    #[serde(default = "default_robustness_perturbation_fraction")]
    pub robustness_perturbation_fraction: f64,
    /// Fraction of ±param neighbors that must survive for databank promotion.
    #[serde(default = "default_minimum_neighborhood_survival_fraction")]
    pub minimum_neighborhood_survival_fraction: f64,
    /// Use broker-local calendar-year folds (every year must pass) instead of
    /// contiguous index slices.
    #[serde(default = "default_calendar_year_folds")]
    pub calendar_year_folds: bool,
    /// When set, databank admission requires deflated trade Sharpe ≥ this floor.
    /// `Some(0.0)` is the production default; `None` reports without rejecting.
    #[serde(default = "default_minimum_deflated_trade_sharpe")]
    pub minimum_deflated_trade_sharpe: Option<f64>,
    /// Require identical-parameter H1 profitability on at least this many pack
    /// symbols before M1 work. `0` disables the multi-symbol screen.
    #[serde(default = "default_multi_symbol_minimum_pass")]
    pub multi_symbol_minimum_pass: usize,
    pub scout: ScoutConfig,
}

fn default_trial_budget_warning() -> u64 {
    TRIAL_BUDGET_WARNING
}

fn default_oos1_expectancy_retention() -> f64 {
    0.7
}

fn default_require_m1_precision() -> bool {
    // Post-breed databank path always promotes on M1 evidence.
    true
}

fn default_simple_exits() -> bool {
    true
}

fn default_max_one_entry_per_day() -> bool {
    true
}

fn default_end_of_day_hour() -> u8 {
    23
}

fn default_mutate_after_elites() -> usize {
    300
}

fn default_random_fill_fraction() -> f64 {
    0.4
}

fn default_worker_threads() -> usize {
    // 0 = auto at pool build time (reserves room for promotion workers).
    0
}

fn default_promotion_worker_threads() -> usize {
    // 0 = auto: clamp 2..=4 from available CPUs.
    0
}

fn default_promotion_queue_capacity() -> usize {
    64
}

fn default_require_m1_robustness() -> bool {
    // Post-breed databank path always runs the M1 robustness battery.
    true
}

fn default_robustness_folds() -> usize {
    3
}

fn default_robustness_monte_carlo_trials() -> usize {
    250
}

fn default_robustness_monte_carlo_block_length() -> usize {
    5
}

fn default_robustness_monte_carlo_skip_trade_probability() -> f64 {
    crate::robustness::MONTE_CARLO_SKIP_TRADE_PROBABILITY
}

fn default_robustness_monte_carlo_p80_profit_retention() -> f64 {
    crate::robustness::MONTE_CARLO_P80_PROFIT_RETENTION
}

fn default_robustness_monte_carlo_max_drawdown_ratio() -> f64 {
    crate::robustness::MONTE_CARLO_MAX_DRAWDOWN_RATIO
}

fn default_robustness_neighborhood_samples() -> usize {
    8
}

fn default_allow_market_entries() -> bool {
    true
}

fn default_robustness_perturbation_fraction() -> f64 {
    crate::robustness::PARAMETER_NEIGHBORHOOD_PERTURBATION_FRACTION
}

fn default_minimum_neighborhood_survival_fraction() -> f64 {
    0.7
}

fn default_calendar_year_folds() -> bool {
    false
}

fn default_minimum_deflated_trade_sharpe() -> Option<f64> {
    // Report-only until multi-symbol pooling is wired; flip to Some(0.0) for hard gate.
    None
}

fn default_multi_symbol_minimum_pass() -> usize {
    0
}

impl Default for DiscoverConfig {
    fn default() -> Self {
        Self {
            initial_candidates: 500,
            batch_size: 200,
            correlation_threshold: 0.85,
            novelty_weight: 10.0,
            tournament_size: 4,
            structural_mutation_probability: 0.18,
            seed: 42,
            universal_grammar: UniversalGrammarConfig::default(),
            run_mode: DiscoverRunMode::FullHarvest,
            early_stop_pot_elites: None,
            target_databank_elites: None,
            trial_budget_warning: default_trial_budget_warning(),
            gates: GateConfig::default(),
            deposit_gates: GateConfig::deposit_defaults(),
            precision: PrecisionGateConfig::default(),
            search_ranges: SearchRangeProfile::default(),
            oos1_expectancy_retention: default_oos1_expectancy_retention(),
            minimum_development_expectancy_r: 0.0,
            require_m1_precision: default_require_m1_precision(),
            simple_exits: default_simple_exits(),
            allow_break_even: false,
            allow_trailing_stops: false,
            allow_partial_exits: false,
            allow_market_entries: default_allow_market_entries(),
            allow_stop_entries: false,
            allow_limit_entries: false,
            flatten_at_22: false,
            end_of_day_hour: default_end_of_day_hour(),
            max_one_entry_per_day: default_max_one_entry_per_day(),
            mutate_after_elites: default_mutate_after_elites(),
            random_fill_fraction: default_random_fill_fraction(),
            worker_threads: default_worker_threads(),
            promotion_worker_threads: default_promotion_worker_threads(),
            promotion_queue_capacity: default_promotion_queue_capacity(),
            require_m1_robustness: default_require_m1_robustness(),
            robustness_folds: default_robustness_folds(),
            robustness_monte_carlo_trials: default_robustness_monte_carlo_trials(),
            robustness_monte_carlo_block_length: default_robustness_monte_carlo_block_length(),
            robustness_monte_carlo_skip_trade_probability:
                default_robustness_monte_carlo_skip_trade_probability(),
            robustness_monte_carlo_p80_profit_retention:
                default_robustness_monte_carlo_p80_profit_retention(),
            robustness_monte_carlo_max_drawdown_ratio:
                default_robustness_monte_carlo_max_drawdown_ratio(),
            robustness_neighborhood_samples: default_robustness_neighborhood_samples(),
            robustness_perturbation_fraction: default_robustness_perturbation_fraction(),
            minimum_neighborhood_survival_fraction: default_minimum_neighborhood_survival_fraction(
            ),
            calendar_year_folds: default_calendar_year_folds(),
            minimum_deflated_trade_sharpe: default_minimum_deflated_trade_sharpe(),
            multi_symbol_minimum_pass: default_multi_symbol_minimum_pass(),
            scout: ScoutConfig::default(),
        }
    }
}

impl DiscoverConfig {
    /// Apply Fast Scout, Full Harvest, or Quota Harvest presets.
    pub fn apply_run_mode(&mut self) {
        match self.run_mode {
            DiscoverRunMode::FastScout => {
                self.initial_candidates = self.initial_candidates.clamp(40, 80);
                self.batch_size = self.batch_size.clamp(20, 40);
                // Pot-only scout: stop before breeding so M1/databank never starts.
                self.simple_exits = !self.has_complex_execution();
                if self.early_stop_pot_elites.is_none() {
                    self.early_stop_pot_elites = Some(8);
                }
                self.mutate_after_elites = self.mutate_after_elites.min(20);
            }
            DiscoverRunMode::FullHarvest => {
                // Do not silently erase an explicitly selected execution module.
                // A run with no modules retains the selected-TF high-parity shape.
                if !self.has_complex_execution() {
                    self.simple_exits = true;
                } else {
                    self.simple_exits = false;
                }
            }
            DiscoverRunMode::QuotaHarvest => {
                // ~20 databank elites: seed-heavy, softer param neighborhood,
                // stop on databank quota — not pot 300. Databank still requires
                // post-breed Development robustness → M1 (never H1-only elites).
                if !self.has_complex_execution() {
                    self.simple_exits = true;
                } else {
                    self.simple_exits = false;
                }
                self.require_m1_precision = true;
                self.require_m1_robustness = true;
                self.initial_candidates = self.initial_candidates.max(1000);
                self.batch_size = self.batch_size.max(300);
                self.random_fill_fraction = self.random_fill_fraction.max(0.75);
                self.mutate_after_elites = self.mutate_after_elites.min(25);
                // Do not early-stop on pot size — that freezes before databank fills.
                // Only the databank quota stops the run.
                self.early_stop_pot_elites = None;
                if self.target_databank_elites.is_none() {
                    self.target_databank_elites = Some(20);
                }
                self.robustness_monte_carlo_trials = self.robustness_monte_carlo_trials.min(80);
                self.robustness_neighborhood_samples = self.robustness_neighborhood_samples.min(5);
                self.minimum_neighborhood_survival_fraction =
                    self.minimum_neighborhood_survival_fraction.min(0.5);
            }
        }
    }

    pub const fn has_complex_execution(&self) -> bool {
        self.allow_break_even
            || self.allow_trailing_stops
            || self.allow_partial_exits
            || self.allow_stop_entries
            || self.allow_limit_entries
            || !self.allow_market_entries
    }

    /// True when the search may only ever place market orders, so seeding never
    /// samples pending distances that `enforce_execution_feature_flags` discards.
    pub const fn market_entries_only(&self) -> bool {
        self.simple_exits || !(self.allow_stop_entries || self.allow_limit_entries)
    }

    /// Planned evaluations for honesty UI: initial + batch × generations.
    pub fn planned_evaluations(&self, generations: u64) -> u64 {
        (self.initial_candidates as u64)
            .saturating_add((self.batch_size as u64).saturating_mul(generations))
    }

    pub fn exceeds_trial_budget_warning(&self, generations: u64) -> bool {
        self.planned_evaluations(generations) > self.trial_budget_warning
    }

    pub(crate) fn validate(&self) -> Result<(), DiscoverError> {
        if !(self.allow_market_entries || self.allow_stop_entries || self.allow_limit_entries) {
            return Err(DiscoverError::InvalidConfig(
                "enable at least one entry order kind: market, stop or limit".into(),
            ));
        }
        // Stop/limit/BE/trail/partials are free to search on Selected-TF for the pot.
        // After breeding unlocks, databank admission always runs Development
        // CPCV/robustness → M1, then the separate OOS1 validation gate.
        if self.initial_candidates == 0 {
            return Err(DiscoverError::InvalidConfig(
                "initial_candidates must be greater than zero".into(),
            ));
        }
        if self.batch_size == 0 {
            return Err(DiscoverError::InvalidConfig(
                "batch_size must be greater than zero".into(),
            ));
        }
        if self.tournament_size == 0 {
            return Err(DiscoverError::InvalidConfig(
                "tournament_size must be greater than zero".into(),
            ));
        }
        for (name, value, inclusive_max) in [
            ("correlation_threshold", self.correlation_threshold, 1.0),
            (
                "structural_mutation_probability",
                self.structural_mutation_probability,
                1.0,
            ),
        ] {
            if !value.is_finite() || !(0.0..=inclusive_max).contains(&value) {
                return Err(DiscoverError::InvalidConfig(format!(
                    "{name} must be finite and between 0 and {inclusive_max}"
                )));
            }
        }
        if !self.novelty_weight.is_finite() || self.novelty_weight < 0.0 {
            return Err(DiscoverError::InvalidConfig(
                "novelty_weight must be finite and non-negative".into(),
            ));
        }
        if !self.precision.minimum_return_retention.is_finite()
            || !(0.0..=1.0).contains(&self.precision.minimum_return_retention)
        {
            return Err(DiscoverError::InvalidConfig(
                "minimum_return_retention must be finite and between 0 and 1".into(),
            ));
        }
        if !self.oos1_expectancy_retention.is_finite()
            || !(0.0..=2.0).contains(&self.oos1_expectancy_retention)
        {
            return Err(DiscoverError::InvalidConfig(
                "oos1_expectancy_retention must be finite and between 0 and 2".into(),
            ));
        }
        if !self.minimum_development_expectancy_r.is_finite()
            || self.minimum_development_expectancy_r < 0.0
        {
            return Err(DiscoverError::InvalidConfig(
                "minimum_development_expectancy_r must be finite and non-negative".into(),
            ));
        }
        if !self.gates.maximum_drawdown_percent.is_finite()
            || self.gates.maximum_drawdown_percent < 0.0
            || !self.gates.minimum_return_percent.is_finite()
            || !self.gates.minimum_profit_factor.is_finite()
            || self.gates.minimum_profit_factor < 0.0
            || !self.gates.minimum_recovery_factor.is_finite()
            || self.gates.minimum_recovery_factor < 0.0
            || !self.deposit_gates.maximum_drawdown_percent.is_finite()
            || self.deposit_gates.maximum_drawdown_percent < 0.0
            || !self.deposit_gates.minimum_return_percent.is_finite()
            || !self.deposit_gates.minimum_profit_factor.is_finite()
            || self.deposit_gates.minimum_profit_factor < 0.0
            || !self.deposit_gates.minimum_recovery_factor.is_finite()
            || self.deposit_gates.minimum_recovery_factor < 0.0
        {
            return Err(DiscoverError::InvalidConfig(
                "gate thresholds must be finite and non-negative where applicable".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.random_fill_fraction)
            || !self.random_fill_fraction.is_finite()
        {
            return Err(DiscoverError::InvalidConfig(
                "random_fill_fraction must be finite and between 0 and 1".into(),
            ));
        }
        if self.promotion_queue_capacity == 0 {
            return Err(DiscoverError::InvalidConfig(
                "promotion_queue_capacity must be greater than zero".into(),
            ));
        }
        if self.require_m1_robustness {
            if self.robustness_folds < 2 {
                return Err(DiscoverError::InvalidConfig(
                    "robustness_folds must be at least 2".into(),
                ));
            }
            if self.robustness_monte_carlo_trials == 0
                || self.robustness_monte_carlo_block_length == 0
                || self.robustness_neighborhood_samples == 0
            {
                return Err(DiscoverError::InvalidConfig(
                    "robustness Monte Carlo trials, block length and neighborhood samples must be positive"
                        .into(),
                ));
            }
            if !self
                .robustness_monte_carlo_skip_trade_probability
                .is_finite()
                || !(0.0..1.0).contains(&self.robustness_monte_carlo_skip_trade_probability)
            {
                return Err(DiscoverError::InvalidConfig(
                    "robustness_monte_carlo_skip_trade_probability must be finite and in [0, 1)"
                        .into(),
                ));
            }
            if !self.robustness_monte_carlo_p80_profit_retention.is_finite()
                || !(0.0..=1.0).contains(&self.robustness_monte_carlo_p80_profit_retention)
            {
                return Err(DiscoverError::InvalidConfig(
                    "robustness_monte_carlo_p80_profit_retention must be finite and between 0 and 1"
                        .into(),
                ));
            }
            if !self.robustness_monte_carlo_max_drawdown_ratio.is_finite()
                || self.robustness_monte_carlo_max_drawdown_ratio < 1.0
            {
                return Err(DiscoverError::InvalidConfig(
                    "robustness_monte_carlo_max_drawdown_ratio must be finite and >= 1".into(),
                ));
            }
            if !self.minimum_neighborhood_survival_fraction.is_finite()
                || !(0.0..=1.0).contains(&self.minimum_neighborhood_survival_fraction)
            {
                return Err(DiscoverError::InvalidConfig(
                    "minimum_neighborhood_survival_fraction must be finite and between 0 and 1"
                        .into(),
                ));
            }
            if !self.robustness_perturbation_fraction.is_finite()
                || !(0.01..=1.0).contains(&self.robustness_perturbation_fraction)
            {
                return Err(DiscoverError::InvalidConfig(
                    "robustness_perturbation_fraction must be finite and between 0.01 and 1".into(),
                ));
            }
        }
        self.search_ranges.validate()?;
        self.universal_grammar.validate()?;
        self.scout.validate()?;
        Ok(())
    }

    /// Dedicated promotion-pool size. `0` → auto clamp into 2..=4.
    pub fn resolved_promotion_worker_threads(&self) -> usize {
        if self.promotion_worker_threads > 0 {
            return self.promotion_worker_threads;
        }
        let cpus = std::thread::available_parallelism()
            .map(|cores| cores.get())
            .unwrap_or(4);
        (cpus / 4).clamp(2, 4)
    }

    /// H1 scout-pool size. `0` → auto (`cpus − 1 − promotion`), leaving room
    /// for the side promotion workers.
    pub fn resolved_scout_worker_threads(&self) -> usize {
        if self.worker_threads > 0 {
            return self.worker_threads;
        }
        let cpus = std::thread::available_parallelism()
            .map(|cores| cores.get())
            .unwrap_or(4);
        let promotion = self.resolved_promotion_worker_threads();
        cpus.saturating_sub(1).saturating_sub(promotion).max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyStyle {
    TrendPullback,
    MomentumBurst,
    DonchianBreakout,
    MeanReversionBand,
    ZScoreReversion,
    SessionOrb,
    ImpulseCandle,
    VolSqueezeBreak,
    SupplyDemandReclaim,
    SweepReclaim,
    Universal,
}

impl FamilyStyle {
    pub const ALL: [Self; 11] = [
        Self::TrendPullback,
        Self::MomentumBurst,
        Self::DonchianBreakout,
        Self::MeanReversionBand,
        Self::ZScoreReversion,
        Self::SessionOrb,
        Self::ImpulseCandle,
        Self::VolSqueezeBreak,
        Self::SupplyDemandReclaim,
        Self::SweepReclaim,
        Self::Universal,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreeLevelBucket {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LongShortSkewBucket {
    ShortHeavy,
    Balanced,
    LongHeavy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorDescriptor {
    /// Count of mirrored entry condition blocks (2, 3 or 4).
    pub entry_conditions: usize,
    /// Count of exit condition blocks OR'd together.
    pub exit_conditions: usize,
    pub trades_per_1000_bars: f64,
    pub average_bars_held: f64,
    pub drawdown_percent: f64,
    pub win_rate_percent: f64,
    /// -1 is entirely short, 0 balanced and +1 entirely long.
    pub long_short_skew: f64,
}

/// MAP-Elites cell. The first axis is the entry-condition count so the archive
/// keeps the best 2-, 3- and 4-condition strategy for every behaviour cell
/// instead of letting one complexity level crowd the others out.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NicheKey {
    pub entry_conditions: usize,
    pub trade_frequency: ThreeLevelBucket,
    pub hold_time: ThreeLevelBucket,
    pub drawdown: ThreeLevelBucket,
    pub win_rate: ThreeLevelBucket,
    pub long_short_skew: LongShortSkewBucket,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceComponents {
    pub return_component: f64,
    pub profit_factor_component: f64,
    pub trade_count_bonus: f64,
    pub drawdown_penalty: f64,
    pub complexity_penalty: f64,
    pub total: f64,
}

/// SQX RetestWithHigherPrecision retention observed when the M1 replay was
/// compared against the Selected-TF scout that proposed the candidate.
///
/// The M1 metrics themselves are `Elite::metrics`; only the scout side and the
/// derived retention ratios are recorded here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct M1RetentionEvidence {
    pub selected_timeframe_metrics: BacktestMetrics,
    pub minimum_return_retention: f64,
    pub return_retention: Option<f64>,
    pub trade_retention: Option<f64>,
    pub drawdown_expansion: Option<f64>,
}

/// One walk-forward fold of the M1 robustness battery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkForwardFold {
    pub fold: usize,
    /// Development group indexes held out by this combination. Empty on
    /// legacy contiguous/calendar-year evidence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_groups: Vec<usize>,
    /// Open of the first decision bar inside the fold.
    pub start_timestamp_ms: i64,
    /// Open of the last decision bar inside the fold.
    pub end_timestamp_ms: i64,
    pub decision_bars: usize,
    /// Trades whose entry fell inside the fold window.
    pub trades_in_fold: usize,
    /// Metrics of the fold replay, which is warmed up on a lookback prefix and
    /// can therefore contain trades that opened before `start_timestamp_ms`.
    pub metrics: BacktestMetrics,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WalkForwardEvidence {
    /// Development diagnostics only. CPCV combinations are adaptive search
    /// evidence and must never be represented as frozen OOS certification.
    pub fold_scheme: String,
    pub total_folds: usize,
    pub passing_folds: usize,
    pub passing_fraction: f64,
    pub required_passing_fraction: f64,
    /// Purge applied before each held-out Development group.
    #[serde(default)]
    pub purge_bars: usize,
    /// Embargo applied after each held-out Development group.
    #[serde(default)]
    pub embargo_bars: usize,
    /// Evaluated folds only. Degenerate ranges still count in `total_folds`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folds: Vec<WalkForwardFold>,
}

/// One evaluated ±parameter neighbour retained for SQX-style distribution UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterNeighborhoodSample {
    pub sample_index: usize,
    pub net_profit: f64,
    pub return_percent: f64,
    pub max_drawdown_percent: f64,
    pub trade_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profit_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharpe_ratio: Option<f64>,
    pub survived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterNeighborhoodEvidence {
    /// `systematic_axis` means each execution parameter was tested at both
    /// sides of the configured perturbation band; older databanks omit this.
    #[serde(default)]
    pub method: String,
    pub perturbation_fraction: f64,
    pub samples_requested: usize,
    /// Samples that produced a canonical, evaluable neighbour.
    pub samples_evaluated: usize,
    pub surviving_samples: usize,
    pub survival_fraction: f64,
    pub required_survival_fraction: f64,
    /// Dedicated ADX plateau neighbours; zero when the strategy has no ADX.
    pub plateau_neighbors: usize,
    pub plateau_surviving: usize,
    pub plateau_survival_fraction: Option<f64>,
    /// Original (unperturbed) M1 baseline metrics for Orig. reference markers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_metrics: Option<BacktestMetrics>,
    /// Per-neighbour metrics for distribution charts. Older elites omit this.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<ParameterNeighborhoodSample>,
}

/// Structured record of everything the M1 pre-deposit robustness battery
/// measured, retained so a promoted elite carries its own audit trail instead of
/// only a gate pass/fail flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RobustnessEvidence {
    pub m1_retention: M1RetentionEvidence,
    pub walk_forward: WalkForwardEvidence,
    pub monte_carlo: MonteCarloReport,
    pub parameter_neighborhood: ParameterNeighborhoodEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Elite {
    pub strategy: StrategyIr,
    pub structural_fingerprint: ContentHash,
    pub descriptor: BehaviorDescriptor,
    pub niche: NicheKey,
    pub evidence: EvidenceComponents,
    pub novelty: f64,
    pub complexity: usize,
    pub metrics: BacktestMetrics,
    /// IS (development) expectancy used for ranking and the OOS1 pick gate.
    #[serde(default)]
    pub is_expectancy: f64,
    /// OOS1 (first holdout) expectancy when the promotion split was active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oos1_expectancy: Option<f64>,
    /// `oos1_expectancy / is_expectancy` when IS expectancy is positive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oos1_expectancy_ratio: Option<f64>,
    /// Observed trade Sharpe proxy at deposit time (primary or pooled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_trade_sharpe: Option<f64>,
    /// Expected max lucky Sharpe given `evaluations_touched` at deposit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_max_lucky_sharpe: Option<f64>,
    /// `observed - expected_max_lucky` at deposit time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deflated_trade_sharpe: Option<f64>,
    /// Per-symbol H1 screen metrics when multi-symbol gate ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub multi_symbol_results: Vec<SymbolScreenResult>,
    /// Named gate outcomes at deposit (evidence strip).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gate_results: Vec<GateResult>,
    /// Full M1 robustness battery record when the battery ran at deposit.
    /// Absent on databanks written before the field existed and on
    /// research-only runs that skipped the battery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub robustness: Option<RobustnessEvidence>,
    /// Downsampled equity deltas, normalized only when correlation is computed.
    pub equity_signature: Vec<f64>,
    pub discovered_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolScreenResult {
    pub symbol: String,
    pub passed: bool,
    pub trade_count: usize,
    pub return_percent: f64,
    pub profit_factor: Option<f64>,
    pub net_profit: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositDecision {
    AcceptedToPot,
    ReplacedInPot,
    AcceptedToDatabank,
    ReplacedInDatabank,
    RejectedGate,
    RejectedDepositGate,
    RejectedClone,
    RejectedCorrelated,
    RejectedNicheNotImproved,
    RejectedPrecision,
    RejectedAmbiguous,
    RejectedOos1,
    RejectedDevelopmentExpectancy,
    RejectedM1Fidelity,
    RejectedWalkForward,
    RejectedMonteCarlo,
    RejectedParamNeighborhood,
    RejectedMultiSymbol,
    RejectedDeflatedSharpe,
    RejectedEvaluation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverTelemetry {
    #[serde(default)]
    pub pot_accepted: u64,
    #[serde(default)]
    pub pot_replaced: u64,
    #[serde(default)]
    pub databank_accepted: u64,
    #[serde(default)]
    pub databank_replaced: u64,
    /// Legacy alias counters kept for older UI/readers.
    #[serde(default)]
    pub accepted_empty: u64,
    #[serde(default)]
    pub replaced_elite: u64,
    pub rejected_gate: u64,
    #[serde(default)]
    pub rejected_deposit_gate: u64,
    pub rejected_clone: u64,
    pub rejected_correlated: u64,
    pub rejected_niche_not_improved: u64,
    pub rejected_precision: u64,
    #[serde(default)]
    pub rejected_ambiguous: u64,
    #[serde(default)]
    pub rejected_oos1: u64,
    #[serde(default)]
    pub rejected_development_expectancy: u64,
    #[serde(default)]
    pub rejected_m1_fidelity: u64,
    #[serde(default)]
    pub rejected_walk_forward: u64,
    #[serde(default)]
    pub rejected_monte_carlo: u64,
    #[serde(default)]
    pub rejected_param_neighborhood: u64,
    #[serde(default)]
    pub rejected_multi_symbol: u64,
    #[serde(default)]
    pub rejected_deflated_sharpe: u64,
    pub rejected_evaluation: u64,
    pub evaluation_errors: BTreeMap<String, u64>,
    /// Pot admissions enqueued for the post-breed databank pipeline.
    #[serde(default)]
    pub promotions_enqueued: u64,
    /// Promotion jobs that finished (pass or reject) on the side pool.
    #[serde(default)]
    pub promotions_completed: u64,
    /// Times enqueue waited because the promotion queue was at capacity.
    #[serde(default)]
    pub promotion_backpressure_events: u64,
    /// Last observed depth: waiting + in-flight promotion jobs.
    #[serde(default)]
    pub promotion_queue_depth: u64,
    /// Last observed in-flight (actively running) promotion jobs.
    #[serde(default)]
    pub promotion_inflight: u64,
}

impl DiscoverTelemetry {
    pub(crate) fn record(&mut self, decision: DepositDecision) {
        match decision {
            DepositDecision::AcceptedToPot => {
                self.pot_accepted += 1;
                self.accepted_empty += 1;
            }
            DepositDecision::ReplacedInPot => {
                self.pot_replaced += 1;
                self.replaced_elite += 1;
            }
            DepositDecision::AcceptedToDatabank => {
                self.databank_accepted += 1;
            }
            DepositDecision::ReplacedInDatabank => {
                self.databank_replaced += 1;
            }
            DepositDecision::RejectedGate => self.rejected_gate += 1,
            DepositDecision::RejectedDepositGate => self.rejected_deposit_gate += 1,
            DepositDecision::RejectedClone => self.rejected_clone += 1,
            DepositDecision::RejectedCorrelated => self.rejected_correlated += 1,
            DepositDecision::RejectedNicheNotImproved => {
                self.rejected_niche_not_improved += 1;
            }
            DepositDecision::RejectedPrecision => self.rejected_precision += 1,
            DepositDecision::RejectedAmbiguous => self.rejected_ambiguous += 1,
            DepositDecision::RejectedOos1 => self.rejected_oos1 += 1,
            DepositDecision::RejectedDevelopmentExpectancy => {
                self.rejected_development_expectancy += 1;
            }
            DepositDecision::RejectedM1Fidelity => self.rejected_m1_fidelity += 1,
            DepositDecision::RejectedWalkForward => self.rejected_walk_forward += 1,
            DepositDecision::RejectedMonteCarlo => self.rejected_monte_carlo += 1,
            DepositDecision::RejectedParamNeighborhood => self.rejected_param_neighborhood += 1,
            DepositDecision::RejectedMultiSymbol => self.rejected_multi_symbol += 1,
            DepositDecision::RejectedDeflatedSharpe => self.rejected_deflated_sharpe += 1,
            DepositDecision::RejectedEvaluation => self.rejected_evaluation += 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Databank {
    pub schema_version: u16,
    pub grammar_version: String,
    pub data_hash: ContentHash,
    /// M1 chronology used to decide which candidates were allowed into the
    /// archive. A databank without this binding is not promotion grade.
    pub execution_data_hash: ContentHash,
    pub broker_spec_hash: ContentHash,
    pub config: DiscoverConfig,
    pub completed_generations: u64,
    pub evaluation_count: u64,
    /// Initial accepted pot used for breeding (H1 / Selected-TF gates only).
    #[serde(default)]
    pub accepted_pool: Vec<Elite>,
    #[serde(default)]
    pub accepted_coverage_map: BTreeMap<String, ContentHash>,
    /// Promotion databank: elites that passed Development M1/CPCV/MC/parameter
    /// robustness and the subsequent OOS1 validation gate. OOS2 is absent.
    pub elites: Vec<Elite>,
    /// Stable niche string to elite fingerprint, convenient for UI coverage maps.
    pub coverage_map: BTreeMap<String, ContentHash>,
    pub telemetry: DiscoverTelemetry,
}

impl Databank {
    pub fn coverage(&self) -> usize {
        self.elites.len()
    }

    pub fn pot_size(&self) -> usize {
        self.accepted_pool.len()
    }

    pub fn qd_score(&self) -> f64 {
        self.elites
            .iter()
            .map(|elite| elite.evidence.total.max(0.0))
            .sum()
    }

    /// Validates the persisted archive independently of any UI or CLI adapter.
    pub fn validate_integrity(&self) -> Result<(), DiscoverError> {
        self.config.validate()?;
        if self.schema_version != DATABANK_SCHEMA_VERSION {
            return Err(DiscoverError::IncompatibleDatabank(format!(
                "schema version {} is not supported",
                self.schema_version
            )));
        }
        if self.grammar_version != GRAMMAR_VERSION {
            return Err(DiscoverError::IncompatibleDatabank(format!(
                "grammar {} does not match {}",
                self.grammar_version, GRAMMAR_VERSION
            )));
        }
        if self.evaluation_count == 0 || (self.elites.is_empty() && self.accepted_pool.is_empty()) {
            return Err(DiscoverError::IncompatibleDatabank(
                "a databank requires evaluations and either an accepted pot or databank elites"
                    .into(),
            ));
        }
        // Elites used to be MAP-Elites (niche-keyed coverage). Validate against a
        // fingerprint stack view so older databanks keep loading after the change.
        let elite_coverage: BTreeMap<_, _> = self
            .elites
            .iter()
            .map(|elite| {
                (
                    elite.structural_fingerprint.to_string(),
                    elite.structural_fingerprint.clone(),
                )
            })
            .collect();
        let pot_coverage: BTreeMap<_, _> = self
            .accepted_pool
            .iter()
            .map(|elite| {
                (
                    elite.structural_fingerprint.to_string(),
                    elite.structural_fingerprint.clone(),
                )
            })
            .collect();
        validate_archive_entries(
            &self.elites,
            &elite_coverage,
            &self.config,
            self.completed_generations,
            ArchiveKind::BreedingBag,
        )?;
        validate_archive_entries(
            &self.accepted_pool,
            &pot_coverage,
            &self.config,
            self.completed_generations,
            ArchiveKind::BreedingBag,
        )?;
        Ok(())
    }
}

enum ArchiveKind {
    BreedingBag,
}

fn validate_archive_entries(
    entries: &[Elite],
    coverage_map: &BTreeMap<String, ContentHash>,
    config: &DiscoverConfig,
    completed_generations: u64,
    kind: ArchiveKind,
) -> Result<(), DiscoverError> {
    let fingerprints: BTreeSet<_> = entries
        .iter()
        .map(|elite| elite.structural_fingerprint.clone())
        .collect();
    let covered: BTreeSet<_> = coverage_map.values().cloned().collect();
    if fingerprints.len() != entries.len()
        || coverage_map.len() != entries.len()
        || covered != fingerprints
    {
        return Err(DiscoverError::IncompatibleDatabank(
            "coverage, niche or fingerprint identities are inconsistent".into(),
        ));
    }
    for elite in entries {
        let fingerprint = elite
            .strategy
            .structural_fingerprint(FloatPolicy::default())?;
        let effective_profit_factor = elite.metrics.profit_factor.unwrap_or(
            if elite.metrics.net_profit > 0.0 && elite.metrics.winning_trades > 0 {
                f64::MAX
            } else {
                0.0
            },
        );
        let fixed_risk = matches!(
            elite.strategy.risk,
            quantforge_ir::RiskPolicy::FixedCurrency { amount }
                if (amount - crate::FIXED_RISK_PER_TRADE).abs() <= 1.0e-9
        );
        let coverage_ok = match kind {
            ArchiveKind::BreedingBag => {
                coverage_map.get(&elite.structural_fingerprint.to_string())
                    == Some(&elite.structural_fingerprint)
            }
        };
        if elite.strategy.manage.flatten_end_of_day != config.flatten_at_22
            || elite.strategy.manage.max_one_entry_per_day != config.max_one_entry_per_day
            || !fixed_risk
            || fingerprint != elite.structural_fingerprint
            || niche_key(&elite.descriptor) != elite.niche
            || !coverage_ok
            || elite.metrics.trade_count < config.deposit_gates.minimum_trades
            || elite.metrics.return_percent <= config.deposit_gates.minimum_return_percent
            || effective_profit_factor < config.deposit_gates.minimum_profit_factor
            || recovery_factor(&elite.metrics) < config.deposit_gates.minimum_recovery_factor
            || elite.metrics.max_drawdown_percent > config.deposit_gates.maximum_drawdown_percent
            || elite.discovered_generation > completed_generations
            || !elite.evidence.total.is_finite()
            || !elite.novelty.is_finite()
        {
            return Err(DiscoverError::IncompatibleDatabank(
                "an elite is structurally invalid or no longer passes its stored gates".into(),
            ));
        }
    }
    Ok(())
}

/// MT5-style recovery factor: net profit ÷ absolute equity max drawdown.
pub(crate) fn recovery_factor(metrics: &BacktestMetrics) -> f64 {
    metrics.recovery_factor()
}

fn is_zero(value: &f64) -> bool {
    value.abs() <= f64::EPSILON
}

#[derive(Debug, Error)]
pub enum DiscoverError {
    #[error("invalid discovery configuration: {0}")]
    InvalidConfig(String),
    #[error("cannot continue databank: {0}")]
    IncompatibleDatabank(String),
    #[error("the initial population produced no eligible elites; loosen gates or use more data")]
    EmptyArchive,
    #[error(transparent)]
    Eval(#[from] EvalError),
    #[error(transparent)]
    Ir(#[from] IrError),
    #[error(transparent)]
    Broker(#[from] quantforge_broker::BrokerSpecError),
}
