//! Trade-level and equity-path comparison between QuantForge and MT5 tester output.

mod indicator;

use quantforge_data::SourceTimezone;
use quantforge_eval::{PositionSide, ScoutResult};
use quantforge_export_mql5::ExportEvidenceCard;
use quantforge_tick::JudgeResult;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

pub use indicator::{
    INDICATOR_PARITY_PROTOCOL_VERSION, IndicatorFieldReport, IndicatorParityConfig,
    IndicatorParityReport, IndicatorReferenceMetadata, compare_indicator_reference,
};

/// v2 binds the tested export's execution inputs as well as its source and IR.
/// A report from a differently-costed EA is therefore not comparable by mistake.
pub const PARITY_PROTOCOL_VERSION: &str = "mt5-parity-v2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mt5TesterMetadata {
    pub properties: BTreeMap<String, String>,
}

impl Mt5TesterMetadata {
    pub fn validate_evidence(&self, evidence: &ExportEvidenceCard) -> Result<(), ParityError> {
        let timeframe = format!("PERIOD_{}", evidence.timeframe);
        for (property, expected) in [
            (
                "strategy_fingerprint",
                evidence.strategy_fingerprint.as_str(),
            ),
            ("broker_spec_hash", evidence.broker_spec_hash.as_str()),
            ("symbol", evidence.symbol.as_str()),
            ("timeframe", timeframe.as_str()),
        ] {
            if self.required(property)? != expected {
                return Err(ParityError::InvalidInput(format!(
                    "MT5 metadata {property} does not match export evidence"
                )));
            }
        }
        self.required("terminal_build")?
            .parse::<u64>()
            .map_err(|_| ParityError::InvalidInput("terminal_build is not an integer".into()))?;
        self.required("server")?;
        self.matches_u64("magic", evidence.config.magic)?;
        self.matches_u64(
            "deviation_points",
            u64::from(evidence.config.deviation_points),
        )?;
        self.matches_number(
            "max_spread_points",
            evidence.config.max_spread_points.unwrap_or(0.0),
        )?;
        self.matches_number(
            "estimated_slippage_points_per_side",
            evidence.config.estimated_slippage_points_per_side,
        )?;
        self.matches_number(
            "commission_per_lot_round_turn",
            evidence.config.commission_per_lot_round_turn,
        )?;
        self.matches_number("initial_deposit", evidence.config.tester.deposit)?;
        Ok(())
    }

    fn required(&self, property: &str) -> Result<&str, ParityError> {
        self.properties
            .get(property)
            .map(String::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ParityError::InvalidInput(format!("MT5 metadata is missing {property}")))
    }

    fn matches_u64(&self, property: &str, expected: u64) -> Result<(), ParityError> {
        let observed = self.required(property)?.parse::<u64>().map_err(|_| {
            ParityError::InvalidInput(format!("MT5 metadata {property} is not an integer"))
        })?;
        if observed == expected {
            Ok(())
        } else {
            Err(ParityError::InvalidInput(format!(
                "MT5 metadata {property} does not match export evidence"
            )))
        }
    }

    fn matches_number(&self, property: &str, expected: f64) -> Result<(), ParityError> {
        let observed = self.required(property)?.parse::<f64>().map_err(|_| {
            ParityError::InvalidInput(format!("MT5 metadata {property} is not numeric"))
        })?;
        let scale = expected.abs().max(1.0);
        if observed.is_finite() && (observed - expected).abs() <= scale * 1.0e-9 {
            Ok(())
        } else {
            Err(ParityError::InvalidInput(format!(
                "MT5 metadata {property} does not match export evidence"
            )))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParitySide {
    Long,
    Short,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityTrade {
    pub side: ParitySide,
    pub entry_timestamp_ms: i64,
    pub exit_timestamp_ms: i64,
    pub entry_price: f64,
    pub exit_price: f64,
    pub volume: f64,
    pub net_profit: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityEquityPoint {
    pub timestamp_ms: i64,
    pub balance: f64,
    pub equity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityMetrics {
    pub initial_balance: f64,
    pub ending_balance: f64,
    pub net_profit: f64,
    pub trade_count: usize,
    pub max_drawdown: f64,
    pub max_drawdown_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParityRun {
    pub engine: String,
    pub trades: Vec<ParityTrade>,
    pub equity: Vec<ParityEquityPoint>,
    pub metrics: ParityMetrics,
}

impl ParityRun {
    pub fn from_scout(result: &ScoutResult) -> Self {
        Self {
            engine: quantforge_eval::ENGINE_TIER.into(),
            trades: result
                .trades
                .iter()
                .map(|trade| ParityTrade {
                    side: match trade.side {
                        PositionSide::Long => ParitySide::Long,
                        PositionSide::Short => ParitySide::Short,
                    },
                    entry_timestamp_ms: trade.entry_timestamp_ms,
                    exit_timestamp_ms: trade.exit_timestamp_ms,
                    entry_price: trade.entry_price,
                    exit_price: trade.exit_price,
                    volume: trade.volume,
                    net_profit: trade.net_profit,
                })
                .collect(),
            equity: result
                .equity
                .iter()
                .map(|point| ParityEquityPoint {
                    timestamp_ms: point.timestamp_ms,
                    balance: point.balance,
                    equity: point.equity,
                })
                .collect(),
            metrics: ParityMetrics {
                initial_balance: result.metrics.initial_balance,
                ending_balance: result.metrics.ending_balance,
                net_profit: result.metrics.net_profit,
                trade_count: result.metrics.trade_count,
                max_drawdown: result.metrics.max_drawdown,
                max_drawdown_percent: result.metrics.max_drawdown_percent,
            },
        }
    }

    pub fn from_judge(result: &JudgeResult) -> Self {
        let scout_shape = ScoutResult {
            trades: result.trades.clone(),
            equity: result.equity.clone(),
            metrics: result.metrics.clone(),
            telemetry: Default::default(),
        };
        let mut run = Self::from_scout(&scout_shape);
        run.engine = result.engine.clone();
        run
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParityTolerances {
    pub trade_count_relative: f64,
    pub trade_count_absolute: usize,
    pub net_profit_relative: f64,
    pub max_drawdown_relative: f64,
    pub max_equity_divergence_percent: f64,
    pub trade_timestamp_tolerance_ms: i64,
    pub minimum_aligned_trade_fraction: f64,
}

impl Default for ParityTolerances {
    fn default() -> Self {
        Self {
            trade_count_relative: 0.10,
            trade_count_absolute: 3,
            net_profit_relative: 0.15,
            max_drawdown_relative: 0.15,
            max_equity_divergence_percent: 5.0,
            trade_timestamp_tolerance_ms: 0,
            minimum_aligned_trade_fraction: 0.90,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TradeDiff {
    pub index: usize,
    pub reference_present: bool,
    pub external_present: bool,
    pub side_match: bool,
    pub entry_timestamp_delta_ms: Option<i64>,
    pub exit_timestamp_delta_ms: Option<i64>,
    pub entry_price_delta: Option<f64>,
    pub exit_price_delta: Option<f64>,
    pub volume_delta: Option<f64>,
    pub net_profit_delta: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffReport {
    pub protocol_version: String,
    pub passed: bool,
    pub protective_orders_present: bool,
    pub trade_count_delta: i64,
    pub allowed_trade_count_delta: usize,
    pub trade_count_passed: bool,
    pub net_profit_delta: f64,
    pub net_profit_delta_relative: f64,
    pub net_profit_passed: bool,
    pub max_drawdown_delta: f64,
    pub max_drawdown_delta_relative: f64,
    pub max_drawdown_passed: bool,
    pub max_equity_path_divergence: f64,
    pub max_equity_path_divergence_percent: f64,
    pub equity_path_passed: bool,
    pub aligned_trade_count: usize,
    pub required_aligned_trade_count: usize,
    pub trade_alignment_passed: bool,
    pub trade_diffs: Vec<TradeDiff>,
    pub tolerances: ParityTolerances,
}

pub fn compare_runs(
    reference: &ParityRun,
    external: &ParityRun,
    evidence: &ExportEvidenceCard,
    tolerances: ParityTolerances,
) -> Result<DiffReport, ParityError> {
    validate_tolerances(&tolerances)?;
    let trade_count_delta =
        external.metrics.trade_count as i64 - reference.metrics.trade_count as i64;
    let allowed_trade_count_delta = tolerances.trade_count_absolute.max(
        (reference.metrics.trade_count as f64 * tolerances.trade_count_relative).ceil() as usize,
    );
    let trade_count_passed = trade_count_delta.unsigned_abs() as usize <= allowed_trade_count_delta;

    let net_profit_delta = external.metrics.net_profit - reference.metrics.net_profit;
    let net_profit_delta_relative = relative_delta(net_profit_delta, reference.metrics.net_profit);
    let net_profit_passed = net_profit_delta_relative <= tolerances.net_profit_relative;

    let max_drawdown_delta = external.metrics.max_drawdown - reference.metrics.max_drawdown;
    let max_drawdown_delta_relative =
        relative_delta(max_drawdown_delta, reference.metrics.max_drawdown);
    let max_drawdown_passed = max_drawdown_delta_relative <= tolerances.max_drawdown_relative;

    let max_equity_path_divergence = equity_divergence(reference, external, 256);
    let max_equity_path_divergence_percent = if reference.metrics.initial_balance > 0.0 {
        max_equity_path_divergence / reference.metrics.initial_balance * 100.0
    } else {
        f64::INFINITY
    };
    let equity_path_passed =
        max_equity_path_divergence_percent <= tolerances.max_equity_divergence_percent;
    let protective_orders_present = evidence.mandatory_stop_loss && evidence.mandatory_take_profit;
    let trade_diffs = align_trades(reference, external);
    let aligned_trade_count = trade_diffs
        .iter()
        .filter(|difference| {
            difference.reference_present
                && difference.external_present
                && difference.side_match
                && difference.entry_timestamp_delta_ms.is_some_and(|delta| {
                    delta.unsigned_abs() <= tolerances.trade_timestamp_tolerance_ms as u64
                })
                && difference.exit_timestamp_delta_ms.is_some_and(|delta| {
                    delta.unsigned_abs() <= tolerances.trade_timestamp_tolerance_ms as u64
                })
        })
        .count();
    let required_aligned_trade_count = ((reference
        .metrics
        .trade_count
        .min(external.metrics.trade_count) as f64)
        * tolerances.minimum_aligned_trade_fraction)
        .ceil() as usize;
    let trade_alignment_passed = aligned_trade_count >= required_aligned_trade_count;
    let passed = protective_orders_present
        && trade_count_passed
        && trade_alignment_passed
        && net_profit_passed
        && max_drawdown_passed
        && equity_path_passed;

    Ok(DiffReport {
        protocol_version: PARITY_PROTOCOL_VERSION.into(),
        passed,
        protective_orders_present,
        trade_count_delta,
        allowed_trade_count_delta,
        trade_count_passed,
        net_profit_delta,
        net_profit_delta_relative,
        net_profit_passed,
        max_drawdown_delta,
        max_drawdown_delta_relative,
        max_drawdown_passed,
        max_equity_path_divergence,
        max_equity_path_divergence_percent,
        equity_path_passed,
        aligned_trade_count,
        required_aligned_trade_count,
        trade_alignment_passed,
        trade_diffs,
        tolerances,
    })
}

pub fn load_mt5_tester_run(
    deals_path: impl AsRef<Path>,
    equity_path: impl AsRef<Path>,
    initial_balance: f64,
) -> Result<ParityRun, ParityError> {
    load_mt5_tester_run_in_timezone(deals_path, equity_path, initial_balance, None)
}

/// Load MT5 tester CSVs, optionally converting server-epoch timestamps to UTC
/// with the same [`SourceTimezone`] used for QuantForge bar ingestion.
pub fn load_mt5_tester_run_in_timezone(
    deals_path: impl AsRef<Path>,
    equity_path: impl AsRef<Path>,
    initial_balance: f64,
    broker_timezone: Option<&str>,
) -> Result<ParityRun, ParityError> {
    if !initial_balance.is_finite() || initial_balance <= 0.0 {
        return Err(ParityError::InvalidInput(
            "initial balance must be finite and greater than zero".into(),
        ));
    }
    let timezone = broker_timezone
        .map(SourceTimezone::from_str)
        .transpose()
        .map_err(|error| ParityError::InvalidInput(error.to_string()))?;
    let mut deals = load_deals(deals_path.as_ref())?;
    let mut equity = load_equity(equity_path.as_ref())?;
    if let Some(timezone) = timezone {
        for deal in &mut deals {
            deal.timestamp_ms = timezone.server_epoch_ms_to_utc_ms(deal.timestamp_ms).ok_or_else(
                || {
                    ParityError::InvalidInput(format!(
                        "cannot localize MT5 deal timestamp {} with {}",
                        deal.timestamp_ms,
                        timezone
                    ))
                },
            )?;
        }
        for point in &mut equity {
            point.timestamp_ms = timezone
                .server_epoch_ms_to_utc_ms(point.timestamp_ms)
                .ok_or_else(|| {
                    ParityError::InvalidInput(format!(
                        "cannot localize MT5 equity timestamp {} with {}",
                        point.timestamp_ms, timezone
                    ))
                })?;
        }
        deals.sort_by_key(|row| (row.timestamp_ms, row.deal_ticket));
        equity.sort_by_key(|row| row.timestamp_ms);
    }
    let trades = pair_deals(deals)?;
    let metrics = calculate_metrics(initial_balance, &trades, &equity);
    Ok(ParityRun {
        engine: "mt5-strategy-tester".into(),
        trades,
        equity,
        metrics,
    })
}

pub fn load_mt5_tester_metadata(path: impl AsRef<Path>) -> Result<Mt5TesterMetadata, ParityError> {
    let mut reader = csv::Reader::from_path(path)?;
    let mut properties = BTreeMap::new();
    for record in reader.records() {
        let record = record?;
        if record.len() != 2 {
            return Err(ParityError::InvalidInput(
                "MT5 metadata rows must contain property,value".into(),
            ));
        }
        let property = record[0].to_owned();
        if properties
            .insert(property.clone(), record[1].to_owned())
            .is_some()
        {
            return Err(ParityError::InvalidInput(format!(
                "MT5 metadata repeats {property}"
            )));
        }
    }
    Ok(Mt5TesterMetadata { properties })
}

#[derive(Debug, Deserialize)]
struct DealRow {
    deal_ticket: u64,
    position_id: u64,
    timestamp_ms: i64,
    deal_type: String,
    deal_entry: String,
    price: f64,
    volume: f64,
    profit: f64,
    commission: f64,
    swap: f64,
    fee: f64,
}

#[derive(Debug, Deserialize)]
struct EquityRow {
    timestamp_ms: i64,
    balance: f64,
    equity: f64,
}

fn load_deals(path: &Path) -> Result<Vec<DealRow>, ParityError> {
    let mut reader = csv::Reader::from_path(path)?;
    let mut rows: Vec<DealRow> = reader.deserialize().collect::<Result<_, _>>()?;
    rows.sort_by_key(|row| (row.timestamp_ms, row.deal_ticket));
    Ok(rows)
}

fn load_equity(path: &Path) -> Result<Vec<ParityEquityPoint>, ParityError> {
    let mut reader = csv::Reader::from_path(path)?;
    let mut rows: Vec<ParityEquityPoint> = reader
        .deserialize::<EquityRow>()
        .map(|row| {
            row.map(|row| ParityEquityPoint {
                timestamp_ms: row.timestamp_ms,
                balance: row.balance,
                equity: row.equity,
            })
        })
        .collect::<Result<_, _>>()?;
    rows.sort_by_key(|row| row.timestamp_ms);
    Ok(rows)
}

#[derive(Debug)]
struct PositionDeals {
    side: Option<ParitySide>,
    entry_timestamp_ms: i64,
    exit_timestamp_ms: i64,
    entry_price_volume: f64,
    entry_volume: f64,
    exit_price_volume: f64,
    exit_volume: f64,
    net_profit: f64,
}

impl Default for PositionDeals {
    fn default() -> Self {
        Self {
            side: None,
            entry_timestamp_ms: i64::MAX,
            exit_timestamp_ms: i64::MIN,
            entry_price_volume: 0.0,
            entry_volume: 0.0,
            exit_price_volume: 0.0,
            exit_volume: 0.0,
            net_profit: 0.0,
        }
    }
}

fn pair_deals(rows: Vec<DealRow>) -> Result<Vec<ParityTrade>, ParityError> {
    let mut positions = BTreeMap::<u64, PositionDeals>::new();
    for row in rows {
        if row.position_id == 0 {
            continue;
        }
        let position = positions.entry(row.position_id).or_default();
        position.net_profit += row.profit + row.commission + row.swap + row.fee;
        match row.deal_entry.as_str() {
            "DEAL_ENTRY_IN" | "DEAL_ENTRY_INOUT" => {
                let side = match row.deal_type.as_str() {
                    "DEAL_TYPE_BUY" => ParitySide::Long,
                    "DEAL_TYPE_SELL" => ParitySide::Short,
                    _ => continue,
                };
                if position.side.is_some_and(|current| current != side) {
                    return Err(ParityError::InvalidInput(format!(
                        "position {} contains conflicting entry sides",
                        row.position_id
                    )));
                }
                position.side = Some(side);
                position.entry_timestamp_ms = position.entry_timestamp_ms.min(row.timestamp_ms);
                position.entry_price_volume += row.price * row.volume;
                position.entry_volume += row.volume;
            }
            "DEAL_ENTRY_OUT" | "DEAL_ENTRY_OUT_BY" => {
                position.exit_timestamp_ms = position.exit_timestamp_ms.max(row.timestamp_ms);
                position.exit_price_volume += row.price * row.volume;
                position.exit_volume += row.volume;
            }
            _ => {}
        }
    }

    let mut trades = Vec::new();
    for (position_id, position) in positions {
        let Some(side) = position.side else {
            continue;
        };
        if position.entry_volume <= 0.0 || position.exit_volume <= 0.0 {
            return Err(ParityError::InvalidInput(format!(
                "position {position_id} does not have both entry and exit deals"
            )));
        }
        trades.push(ParityTrade {
            side,
            entry_timestamp_ms: position.entry_timestamp_ms,
            exit_timestamp_ms: position.exit_timestamp_ms,
            entry_price: position.entry_price_volume / position.entry_volume,
            exit_price: position.exit_price_volume / position.exit_volume,
            volume: position.entry_volume,
            net_profit: position.net_profit,
        });
    }
    trades.sort_by_key(|trade| (trade.entry_timestamp_ms, trade.exit_timestamp_ms));
    Ok(trades)
}

fn calculate_metrics(
    initial_balance: f64,
    trades: &[ParityTrade],
    equity: &[ParityEquityPoint],
) -> ParityMetrics {
    let net_profit = trades.iter().map(|trade| trade.net_profit).sum::<f64>();
    let ending_balance = equity
        .last()
        .map(|point| point.balance)
        .unwrap_or(initial_balance + net_profit);
    let mut peak = initial_balance;
    let mut max_drawdown = 0.0_f64;
    let mut max_drawdown_percent = 0.0_f64;
    for point in equity {
        peak = peak.max(point.equity);
        let drawdown = peak - point.equity;
        max_drawdown = max_drawdown.max(drawdown);
        if peak > 0.0 {
            max_drawdown_percent = max_drawdown_percent.max(drawdown / peak * 100.0);
        }
    }
    ParityMetrics {
        initial_balance,
        ending_balance,
        net_profit,
        trade_count: trades.len(),
        max_drawdown,
        max_drawdown_percent,
    }
}

/// Pair reference↔external trades by side and closest entry time (not list index).
///
/// Index pairing fails as soon as one side inserts/skips a fill; after broker-clock
/// localization the true matches are usually within a minute but sit at different
/// offsets in the two lists.
fn align_trades(reference: &ParityRun, external: &ParityRun) -> Vec<TradeDiff> {
    const MAX_SEARCH_MS: i64 = 24 * 60 * 60 * 1000;
    let mut used = vec![false; external.trades.len()];
    let mut diffs = Vec::with_capacity(reference.trades.len() + external.trades.len());

    for (index, left) in reference.trades.iter().enumerate() {
        let mut best: Option<(i64, f64, usize)> = None;
        for (external_index, right) in external.trades.iter().enumerate() {
            if used[external_index] || left.side != right.side {
                continue;
            }
            let time_delta = (right.entry_timestamp_ms - left.entry_timestamp_ms).abs();
            if time_delta > MAX_SEARCH_MS {
                continue;
            }
            let price_delta = (right.entry_price - left.entry_price).abs();
            let better = match best {
                None => true,
                Some((best_time, best_price, _)) => {
                    time_delta < best_time
                        || (time_delta == best_time && price_delta < best_price)
                }
            };
            if better {
                best = Some((time_delta, price_delta, external_index));
            }
        }

        if let Some((_, _, external_index)) = best {
            used[external_index] = true;
            let right = &external.trades[external_index];
            diffs.push(TradeDiff {
                index,
                reference_present: true,
                external_present: true,
                side_match: true,
                entry_timestamp_delta_ms: Some(right.entry_timestamp_ms - left.entry_timestamp_ms),
                exit_timestamp_delta_ms: Some(right.exit_timestamp_ms - left.exit_timestamp_ms),
                entry_price_delta: Some(right.entry_price - left.entry_price),
                exit_price_delta: Some(right.exit_price - left.exit_price),
                volume_delta: Some(right.volume - left.volume),
                net_profit_delta: Some(right.net_profit - left.net_profit),
            });
        } else {
            diffs.push(TradeDiff {
                index,
                reference_present: true,
                external_present: false,
                side_match: false,
                entry_timestamp_delta_ms: None,
                exit_timestamp_delta_ms: None,
                entry_price_delta: None,
                exit_price_delta: None,
                volume_delta: None,
                net_profit_delta: None,
            });
        }
    }

    for (external_index, _) in external.trades.iter().enumerate() {
        if used[external_index] {
            continue;
        }
        diffs.push(TradeDiff {
            index: diffs.len(),
            reference_present: false,
            external_present: true,
            side_match: false,
            entry_timestamp_delta_ms: None,
            exit_timestamp_delta_ms: None,
            entry_price_delta: None,
            exit_price_delta: None,
            volume_delta: None,
            net_profit_delta: None,
        });
    }
    diffs
}

/// Compare the realised balance path, not independently sampled floating P&L.
///
/// The M1 judge marks an open position once per minute while the generated EA
/// records MT5 equity on decision-bar ticks. Those snapshots can differ during
/// an otherwise identical open trade, especially around a stop. Closed deals
/// are the common, broker-verifiable equity path and are already checked for
/// timing and economics by `align_trades`.
fn equity_divergence(reference: &ParityRun, external: &ParityRun, points: usize) -> f64 {
    let left = resample_balance(&realized_balance_path(reference), points);
    let right = resample_balance(&realized_balance_path(external), points);
    if left.is_empty() || right.is_empty() {
        return if left.is_empty() && right.is_empty() {
            0.0
        } else {
            f64::INFINITY
        };
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f64::max)
}

fn realized_balance_path(run: &ParityRun) -> Vec<f64> {
    let mut balance = run.metrics.initial_balance;
    let mut values = Vec::with_capacity(run.trades.len() + 1);
    values.push(balance);
    for trade in &run.trades {
        balance += trade.net_profit;
        values.push(balance);
    }
    values
}

fn resample_balance(values: &[f64], points: usize) -> Vec<f64> {
    if values.is_empty() {
        return Vec::new();
    }
    let count = values.len().min(points).max(1);
    if count == 1 {
        return vec![values[0]];
    }
    (0..count)
        .map(|index| {
            let source = index * (values.len() - 1) / (count - 1);
            values[source]
        })
        .collect()
}

fn relative_delta(delta: f64, reference: f64) -> f64 {
    delta.abs() / reference.abs().max(1.0)
}

fn validate_tolerances(value: &ParityTolerances) -> Result<(), ParityError> {
    for (name, tolerance) in [
        ("trade_count_relative", value.trade_count_relative),
        ("net_profit_relative", value.net_profit_relative),
        ("max_drawdown_relative", value.max_drawdown_relative),
        (
            "max_equity_divergence_percent",
            value.max_equity_divergence_percent,
        ),
        (
            "minimum_aligned_trade_fraction",
            value.minimum_aligned_trade_fraction,
        ),
    ] {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return Err(ParityError::InvalidInput(format!(
                "{name} must be finite and non-negative"
            )));
        }
    }
    if value.trade_timestamp_tolerance_ms < 0 {
        return Err(ParityError::InvalidInput(
            "trade_timestamp_tolerance_ms must be non-negative".into(),
        ));
    }
    if value.minimum_aligned_trade_fraction > 1.0 {
        return Err(ParityError::InvalidInput(
            "minimum_aligned_trade_fraction must not exceed 1".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum ParityError {
    #[error("invalid parity input: {0}")]
    InvalidInput(String),
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_core::ContentHash;
    use quantforge_export_mql5::{Mql5ExportConfig, TesterConfig};
    use std::fs;

    fn evidence() -> ExportEvidenceCard {
        ExportEvidenceCard {
            schema_version: 1,
            target: "MetaTrader 5".into(),
            strategy_fingerprint: ContentHash::sha256("strategy"),
            broker_spec_hash: ContentHash::sha256("broker"),
            source_hash: ContentHash::sha256("source"),
            strategy_ir_version: 1,
            expert_name: "Fixture".into(),
            symbol: "TEST".into(),
            timeframe: "M15".into(),
            live_trading_default: false,
            mandatory_stop_loss: true,
            mandatory_take_profit: true,
            parity_deals_file: "deals.csv".into(),
            parity_equity_file: "equity.csv".into(),
            parity_metadata_file: "metadata.csv".into(),
            config: Mql5ExportConfig {
                tester: TesterConfig::default(),
                ..Mql5ExportConfig::default()
            },
        }
    }

    fn run(net_profit: f64) -> ParityRun {
        ParityRun {
            engine: "fixture".into(),
            trades: vec![ParityTrade {
                side: ParitySide::Long,
                entry_timestamp_ms: 1,
                exit_timestamp_ms: 2,
                entry_price: 100.0,
                exit_price: 101.0,
                volume: 1.0,
                net_profit,
            }],
            equity: vec![
                ParityEquityPoint {
                    timestamp_ms: 1,
                    balance: 100_000.0,
                    equity: 100_000.0,
                },
                ParityEquityPoint {
                    timestamp_ms: 2,
                    balance: 100_000.0 + net_profit,
                    equity: 100_000.0 + net_profit,
                },
            ],
            metrics: ParityMetrics {
                initial_balance: 100_000.0,
                ending_balance: 100_000.0 + net_profit,
                net_profit,
                trade_count: 1,
                max_drawdown: 0.0,
                max_drawdown_percent: 0.0,
            },
        }
    }

    #[test]
    fn identical_runs_pass_and_large_profit_divergence_fails() {
        let pass = compare_runs(&run(100.0), &run(100.0), &evidence(), Default::default()).unwrap();
        assert!(pass.passed);
        assert_eq!(pass.aligned_trade_count, 1);

        let fail = compare_runs(&run(100.0), &run(50.0), &evidence(), Default::default()).unwrap();
        assert!(!fail.passed);
        assert!(!fail.net_profit_passed);
    }

    #[test]
    fn tester_deals_are_paired_by_position_id() {
        let directory = tempfile::tempdir().unwrap();
        let deals = directory.path().join("deals.csv");
        let equity = directory.path().join("equity.csv");
        fs::write(
            &deals,
            "deal_ticket,position_id,timestamp_ms,deal_type,deal_entry,price,volume,profit,commission,swap,fee\n\
             1,99,1000,DEAL_TYPE_BUY,DEAL_ENTRY_IN,1.1000,0.1,0,-0.35,0,0\n\
             2,99,2000,DEAL_TYPE_SELL,DEAL_ENTRY_OUT,1.1010,0.1,10,-0.35,0,0\n",
        )
        .unwrap();
        fs::write(
            &equity,
            "timestamp_ms,balance,equity\n1000,100000,100000\n2000,100009.3,100009.3\n",
        )
        .unwrap();

        let run = load_mt5_tester_run(deals, equity, 100_000.0).unwrap();
        assert_eq!(run.trades.len(), 1);
        assert_eq!(run.trades[0].side, ParitySide::Long);
        assert!((run.trades[0].net_profit - 9.3).abs() < 1e-12);
    }

    #[test]
    fn tester_metadata_is_bound_to_export_evidence() {
        let evidence = evidence();
        let metadata = Mt5TesterMetadata {
            properties: BTreeMap::from([
                (
                    "strategy_fingerprint".into(),
                    evidence.strategy_fingerprint.to_string(),
                ),
                (
                    "broker_spec_hash".into(),
                    evidence.broker_spec_hash.to_string(),
                ),
                ("symbol".into(), evidence.symbol.clone()),
                ("timeframe".into(), "PERIOD_M15".into()),
                ("terminal_build".into(), "5834".into()),
                ("server".into(), "Fixture-Demo".into()),
                ("magic".into(), evidence.config.magic.to_string()),
                (
                    "deviation_points".into(),
                    evidence.config.deviation_points.to_string(),
                ),
                ("max_spread_points".into(), "0".into()),
                ("estimated_slippage_points_per_side".into(), "0".into()),
                ("commission_per_lot_round_turn".into(), "0".into()),
                ("initial_deposit".into(), "100000".into()),
            ]),
        };
        metadata.validate_evidence(&evidence).unwrap();

        let mut wrong = metadata.clone();
        wrong.properties.insert("symbol".into(), "OTHER".into());
        assert!(wrong.validate_evidence(&evidence).is_err());

        let mut wrong_cost = metadata;
        wrong_cost
            .properties
            .insert("commission_per_lot_round_turn".into(), "7".into());
        assert!(wrong_cost.validate_evidence(&evidence).is_err());
    }
}
