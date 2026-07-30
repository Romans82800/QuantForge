mod adx;
mod atr;
mod atr_percentile;
mod extremes;
mod liquidity_sweep;
mod roc;
mod rsi;
mod session;
mod swing_zone;
mod zscore;

use serde::{Deserialize, Serialize};

pub use adx::directional_index;
pub use atr::atr_series;
pub use atr_percentile::atr_percentile_series;
pub use extremes::{highest_series, lowest_series, rolling_extreme_into};
pub use liquidity_sweep::liquidity_sweep_score_series;
pub use roc::rate_of_change_series;
pub use rsi::rsi_series;
pub use session::session_range_series;
pub use swing_zone::swing_base_zone_series;
pub use zscore::zscore_series;

/// SQX RetestWithHigherPrecision acceptance bands (80/80/130).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SqxPrecisionProfile {
    pub minimum_return_retention: f64,
    pub minimum_trade_retention: f64,
    pub maximum_drawdown_expansion: f64,
}

impl Default for SqxPrecisionProfile {
    fn default() -> Self {
        Self {
            minimum_return_retention: 0.80,
            minimum_trade_retention: 0.80,
            maximum_drawdown_expansion: 1.30,
        }
    }
}

/// SQX default end-of-day flatten hour (broker local).
pub const DEFAULT_END_OF_DAY_HOUR: u8 = 23;
