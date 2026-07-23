use crate::ParityError;
use quantforge_core::ContentHash;
use quantforge_data::Bar;
use quantforge_eval::calculate_indicator_series;
use quantforge_ir::{IndicatorExpr, PriceField};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const INDICATOR_PARITY_PROTOCOL_VERSION: &str = "mt5-indicator-parity-v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndicatorParityConfig {
    /// Rows discarded from the oldest edge of the probe pack. MT5 indicator
    /// handles have history before that edge, while the Rust series starts
    /// there, so recursive indicators need a deterministic convergence span.
    pub warmup_rows: usize,
    pub absolute_epsilon: f64,
    pub relative_epsilon: f64,
}

impl Default for IndicatorParityConfig {
    fn default() -> Self {
        Self {
            warmup_rows: 1_000,
            absolute_epsilon: 1.0e-10,
            relative_epsilon: 1.0e-9,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorReferenceMetadata {
    pub terminal_build: u64,
    pub broker: String,
    pub server: String,
    pub symbol: String,
    pub timeframe: String,
    pub period: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorFieldReport {
    pub passed: bool,
    pub compared_rows: usize,
    pub mismatch_count: usize,
    pub max_absolute_error: f64,
    pub max_relative_error: f64,
    pub first_mismatch_row: Option<usize>,
    pub first_mismatch_timestamp_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorParityReport {
    pub protocol_version: String,
    pub passed: bool,
    pub reference_hash: ContentHash,
    pub metadata: IndicatorReferenceMetadata,
    pub source_rows: usize,
    pub compared_rows: usize,
    pub config: IndicatorParityConfig,
    pub indicators: BTreeMap<String, IndicatorFieldReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReferenceRow {
    timestamp_ms: i64,
    #[allow(dead_code)]
    server_time: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    sma: f64,
    ema: f64,
    wma: f64,
    rsi: f64,
    atr: f64,
    donchian_high: f64,
    donchian_low: f64,
    highest_close: f64,
    lowest_close: f64,
    standard_deviation: f64,
    zscore: f64,
    percentile_in_range: f64,
    rate_of_change: f64,
    #[allow(dead_code)]
    session_hour: u8,
    #[allow(dead_code)]
    day_of_week: u8,
    terminal_build: u64,
    broker: String,
    server: String,
    symbol: String,
    timeframe: String,
    period: u16,
}

pub fn compare_indicator_reference(
    path: impl AsRef<Path>,
    config: IndicatorParityConfig,
) -> Result<IndicatorParityReport, ParityError> {
    validate_config(&config)?;
    let path = path.as_ref();
    let bytes = fs::read(path)?;
    let mut reader = csv::Reader::from_reader(bytes.as_slice());
    let rows: Vec<ReferenceRow> = reader.deserialize().collect::<Result<_, _>>()?;
    if rows.is_empty() {
        return Err(ParityError::InvalidInput(
            "indicator reference pack has no rows".into(),
        ));
    }
    if rows.len() <= config.warmup_rows {
        return Err(ParityError::InvalidInput(format!(
            "indicator reference pack has {} rows but warmup_rows is {}",
            rows.len(),
            config.warmup_rows
        )));
    }
    if rows
        .windows(2)
        .any(|pair| pair[0].timestamp_ms >= pair[1].timestamp_ms)
    {
        return Err(ParityError::InvalidInput(
            "indicator reference timestamps must be strictly increasing".into(),
        ));
    }

    let first = &rows[0];
    let metadata = IndicatorReferenceMetadata {
        terminal_build: first.terminal_build,
        broker: first.broker.clone(),
        server: first.server.clone(),
        symbol: first.symbol.clone(),
        timeframe: first.timeframe.clone(),
        period: first.period,
    };
    if metadata.terminal_build == 0
        || metadata.broker.trim().is_empty()
        || metadata.server.trim().is_empty()
        || metadata.symbol.trim().is_empty()
        || metadata.timeframe.trim().is_empty()
        || metadata.period < 2
    {
        return Err(ParityError::InvalidInput(
            "indicator reference metadata is incomplete or invalid".into(),
        ));
    }
    if rows.iter().any(|row| {
        row.terminal_build != metadata.terminal_build
            || row.broker != metadata.broker
            || row.server != metadata.server
            || row.symbol != metadata.symbol
            || row.timeframe != metadata.timeframe
            || row.period != metadata.period
    }) {
        return Err(ParityError::InvalidInput(
            "indicator reference metadata changes within the pack".into(),
        ));
    }

    let bars: Vec<Bar> = rows
        .iter()
        .map(|row| Bar {
            timestamp_ms: row.timestamp_ms,
            open: row.open,
            high: row.high,
            low: row.low,
            close: row.close,
            tick_volume: 0,
            real_volume: 0,
            spread_points: None,
        })
        .collect();
    validate_bars(&bars)?;

    let period = metadata.period;
    let source = PriceField::Close;
    let candidates = [
        (
            "sma",
            IndicatorExpr::Sma {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.sma).collect::<Vec<_>>(),
        ),
        (
            "ema",
            IndicatorExpr::Ema {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.ema).collect(),
        ),
        (
            "wma",
            IndicatorExpr::Wma {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.wma).collect(),
        ),
        (
            "rsi",
            IndicatorExpr::Rsi {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.rsi).collect(),
        ),
        (
            "atr",
            IndicatorExpr::Atr { period, shift: 0 },
            rows.iter().map(|row| row.atr).collect(),
        ),
        (
            "donchian_high",
            IndicatorExpr::DonchianHigh { period, shift: 0 },
            rows.iter().map(|row| row.donchian_high).collect(),
        ),
        (
            "donchian_low",
            IndicatorExpr::DonchianLow { period, shift: 0 },
            rows.iter().map(|row| row.donchian_low).collect(),
        ),
        (
            "highest_close",
            IndicatorExpr::Highest {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.highest_close).collect(),
        ),
        (
            "lowest_close",
            IndicatorExpr::Lowest {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.lowest_close).collect(),
        ),
        (
            "standard_deviation",
            IndicatorExpr::StandardDeviation {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.standard_deviation).collect(),
        ),
        (
            "zscore",
            IndicatorExpr::ZScore {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.zscore).collect(),
        ),
        (
            "percentile_in_range",
            IndicatorExpr::PercentileInRange {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.percentile_in_range).collect(),
        ),
        (
            "rate_of_change",
            IndicatorExpr::RateOfChange {
                source,
                period,
                shift: 0,
            },
            rows.iter().map(|row| row.rate_of_change).collect(),
        ),
    ];

    let mut indicators = BTreeMap::new();
    for (name, expression, reference) in candidates {
        let calculated = calculate_indicator_series(&bars, &expression);
        indicators.insert(
            name.into(),
            compare_field(&rows, &calculated, &reference, &config)?,
        );
    }
    let passed = indicators.values().all(|field| field.passed);

    Ok(IndicatorParityReport {
        protocol_version: INDICATOR_PARITY_PROTOCOL_VERSION.into(),
        passed,
        reference_hash: ContentHash::sha256(&bytes),
        metadata,
        source_rows: rows.len(),
        compared_rows: rows.len() - config.warmup_rows,
        config,
        indicators,
    })
}

fn compare_field(
    rows: &[ReferenceRow],
    calculated: &[f64],
    reference: &[f64],
    config: &IndicatorParityConfig,
) -> Result<IndicatorFieldReport, ParityError> {
    if calculated.len() != rows.len() || reference.len() != rows.len() {
        return Err(ParityError::InvalidInput(
            "indicator buffers do not match the reference row count".into(),
        ));
    }
    let mut mismatch_count = 0;
    let mut max_absolute_error = 0.0_f64;
    let mut max_relative_error = 0.0_f64;
    let mut first_mismatch_row = None;
    let mut first_mismatch_timestamp_ms = None;

    for index in config.warmup_rows..rows.len() {
        let expected = reference[index];
        let actual = calculated[index];
        if !expected.is_finite() || expected.abs() > 1.0e100 || !actual.is_finite() {
            return Err(ParityError::InvalidInput(format!(
                "indicator value is undefined after warmup at row {index}"
            )));
        }
        let absolute_error = (actual - expected).abs();
        let relative_error = absolute_error / expected.abs().max(1.0e-15);
        max_absolute_error = max_absolute_error.max(absolute_error);
        max_relative_error = max_relative_error.max(relative_error);
        let allowed = config.absolute_epsilon + config.relative_epsilon * expected.abs();
        if absolute_error > allowed {
            mismatch_count += 1;
            if first_mismatch_row.is_none() {
                first_mismatch_row = Some(index);
                first_mismatch_timestamp_ms = Some(rows[index].timestamp_ms);
            }
        }
    }

    Ok(IndicatorFieldReport {
        passed: mismatch_count == 0,
        compared_rows: rows.len() - config.warmup_rows,
        mismatch_count,
        max_absolute_error,
        max_relative_error,
        first_mismatch_row,
        first_mismatch_timestamp_ms,
    })
}

fn validate_config(config: &IndicatorParityConfig) -> Result<(), ParityError> {
    for (name, value) in [
        ("absolute_epsilon", config.absolute_epsilon),
        ("relative_epsilon", config.relative_epsilon),
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(ParityError::InvalidInput(format!(
                "{name} must be finite and non-negative"
            )));
        }
    }
    Ok(())
}

fn validate_bars(bars: &[Bar]) -> Result<(), ParityError> {
    for (index, bar) in bars.iter().enumerate() {
        if ![bar.open, bar.high, bar.low, bar.close]
            .into_iter()
            .all(f64::is_finite)
            || bar.high < bar.low
            || bar.high < bar.open.max(bar.close)
            || bar.low > bar.open.min(bar.close)
        {
            return Err(ParityError::InvalidInput(format!(
                "indicator reference row {index} has malformed OHLC"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn exact_reference_pack_passes_and_a_changed_buffer_fails() {
        let period = 14;
        let bars: Vec<Bar> = (0..120)
            .map(|index| {
                let close = 1.1 + index as f64 * 0.0001 + (index % 5) as f64 * 0.00003;
                Bar {
                    timestamp_ms: 1_700_000_000_000 + index * 900_000,
                    open: close - 0.00002,
                    high: close + 0.0002,
                    low: close - 0.0002,
                    close,
                    tick_volume: 0,
                    real_volume: 0,
                    spread_points: None,
                }
            })
            .collect();
        let series = |expression| calculate_indicator_series(&bars, &expression);
        let sma = series(IndicatorExpr::Sma {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let ema = series(IndicatorExpr::Ema {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let wma = series(IndicatorExpr::Wma {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let rsi = series(IndicatorExpr::Rsi {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let atr = series(IndicatorExpr::Atr { period, shift: 0 });
        let donchian_high = series(IndicatorExpr::DonchianHigh { period, shift: 0 });
        let donchian_low = series(IndicatorExpr::DonchianLow { period, shift: 0 });
        let highest_close = series(IndicatorExpr::Highest {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let lowest_close = series(IndicatorExpr::Lowest {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let standard_deviation = series(IndicatorExpr::StandardDeviation {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let zscore = series(IndicatorExpr::ZScore {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let percentile_in_range = series(IndicatorExpr::PercentileInRange {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let rate_of_change = series(IndicatorExpr::RateOfChange {
            source: PriceField::Close,
            period,
            shift: 0,
        });
        let mut rows: Vec<ReferenceRow> = bars
            .iter()
            .enumerate()
            .map(|(index, bar)| ReferenceRow {
                timestamp_ms: bar.timestamp_ms,
                server_time: "2024-01-01T00:00:00".into(),
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                sma: sma[index],
                ema: ema[index],
                wma: wma[index],
                rsi: rsi[index],
                atr: atr[index],
                donchian_high: donchian_high[index],
                donchian_low: donchian_low[index],
                highest_close: highest_close[index],
                lowest_close: lowest_close[index],
                standard_deviation: standard_deviation[index],
                zscore: zscore[index],
                percentile_in_range: percentile_in_range[index],
                rate_of_change: rate_of_change[index],
                session_hour: 0,
                day_of_week: 1,
                terminal_build: 5_834,
                broker: "Fixture Broker".into(),
                server: "Fixture-Demo".into(),
                symbol: "TEST".into(),
                timeframe: "PERIOD_M15".into(),
                period,
            })
            .collect();
        let config = IndicatorParityConfig {
            warmup_rows: 30,
            absolute_epsilon: 1.0e-12,
            relative_epsilon: 1.0e-12,
        };

        let exact = write_rows(&rows);
        assert!(
            compare_indicator_reference(exact.path(), config.clone())
                .unwrap()
                .passed
        );

        rows[40].sma += 0.1;
        let changed = write_rows(&rows);
        let report = compare_indicator_reference(changed.path(), config).unwrap();
        assert!(!report.passed);
        assert_eq!(report.indicators["sma"].mismatch_count, 1);
    }

    fn write_rows(rows: &[ReferenceRow]) -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let mut writer = csv::Writer::from_writer(file.reopen().unwrap());
        for row in rows {
            writer.serialize(row).unwrap();
        }
        writer.flush().unwrap();
        file
    }
}
