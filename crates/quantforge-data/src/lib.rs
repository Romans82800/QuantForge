//! Deterministic MT5 bar ingestion and data-quality diagnostics.

use chrono::{Datelike, Duration, LocalResult, NaiveDateTime, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use csv::{ReaderBuilder, StringRecord, Trim};
use quantforge_core::{ContentHash, HashError, stable_json_hash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub timestamp_ms: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub tick_volume: u64,
    pub real_volume: u64,
    pub spread_points: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BarDataset {
    pub bars: Vec<Bar>,
    pub source_rows: usize,
    pub duplicate_rows_removed: usize,
    pub input_was_sorted: bool,
    pub delimiter: char,
    pub source_timezone: String,
    pub data_hash: ContentHash,
}

impl BarDataset {
    pub fn load_mt5(
        path: impl AsRef<Path>,
        source_timezone: SourceTimezone,
    ) -> Result<Self, DataError> {
        let bytes = fs::read(path)?;
        let delimiter = detect_delimiter(&bytes)?;
        let mut reader = ReaderBuilder::new()
            .delimiter(delimiter)
            .trim(Trim::All)
            .flexible(false)
            .from_reader(bytes.as_slice());

        let headers = reader.headers()?.clone();
        let columns = Columns::from_headers(&headers)?;
        let mut parsed = Vec::new();

        for (index, record) in reader.records().enumerate() {
            let record = record?;
            parsed.push(columns.parse(&record, index + 2, source_timezone)?);
        }

        if parsed.is_empty() {
            return Err(DataError::NoRows);
        }

        let input_was_sorted = parsed
            .windows(2)
            .all(|pair| pair[0].timestamp_ms <= pair[1].timestamp_ms);
        let source_rows = parsed.len();
        let mut by_timestamp = BTreeMap::new();
        for bar in parsed {
            // MT5 exports occasionally repeat rows. The last source row wins.
            by_timestamp.insert(bar.timestamp_ms, bar);
        }
        let duplicate_rows_removed = source_rows - by_timestamp.len();
        let bars: Vec<_> = by_timestamp.into_values().collect();
        let data_hash = bar_content_hash(&bars);

        Ok(Self {
            bars,
            source_rows,
            duplicate_rows_removed,
            input_was_sorted,
            delimiter: delimiter as char,
            source_timezone: source_timezone.to_string(),
            data_hash,
        })
    }
}

const IC_MARKETS_EST_PLUS_7: &str = "ICMarkets/EST+7";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceTimezone {
    timezone: Tz,
    /// Broker wall time is this many hours ahead of the named timezone.
    wall_clock_shift_hours: i64,
    custom_name: Option<&'static str>,
}

impl SourceTimezone {
    pub fn name(self) -> &'static str {
        self.custom_name.unwrap_or_else(|| self.timezone.name())
    }

    fn localize(self, timestamp: NaiveDateTime) -> LocalResult<chrono::DateTime<Tz>> {
        let Some(timestamp) =
            timestamp.checked_sub_signed(Duration::hours(self.wall_clock_shift_hours))
        else {
            return LocalResult::None;
        };
        self.timezone.from_local_datetime(&timestamp)
    }

    fn source_wall_time(self, timestamp_ms: i64) -> Option<NaiveDateTime> {
        let timestamp = Utc.timestamp_millis_opt(timestamp_ms).single()?;
        timestamp
            .with_timezone(&self.timezone)
            .naive_local()
            .checked_add_signed(Duration::hours(self.wall_clock_shift_hours))
    }
}

impl std::fmt::Display for SourceTimezone {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

impl FromStr for SourceTimezone {
    type Err = DataError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.eq_ignore_ascii_case(IC_MARKETS_EST_PLUS_7) {
            return Ok(Self {
                timezone: chrono_tz::America::New_York,
                wall_clock_shift_hours: 7,
                custom_name: Some(IC_MARKETS_EST_PLUS_7),
            });
        }
        value
            .parse::<Tz>()
            .map(|timezone| Self {
                timezone,
                wall_clock_shift_hours: 0,
                custom_name: None,
            })
            .map_err(|_| DataError::UnknownTimezone(value.into()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mt5ExportMetadata {
    pub properties: BTreeMap<String, String>,
    pub metadata_hash: ContentHash,
}

impl Mt5ExportMetadata {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DataError> {
        let mut reader = ReaderBuilder::new().trim(Trim::All).from_path(path)?;
        let headers = reader.headers()?.clone();
        let property_index = headers
            .iter()
            .position(|value| normalize_header(value) == "PROPERTY")
            .ok_or(DataError::MissingColumn("property"))?;
        let value_index = headers
            .iter()
            .position(|value| normalize_header(value) == "VALUE")
            .ok_or(DataError::MissingColumn("value"))?;
        let mut properties = BTreeMap::new();
        for (index, record) in reader.records().enumerate() {
            let record = record?;
            let property = value(&record, property_index, index + 2)?.to_owned();
            let property_value = value(&record, value_index, index + 2)?.to_owned();
            if properties
                .insert(property.clone(), property_value)
                .is_some()
            {
                return Err(DataError::DuplicateMetadataProperty(property));
            }
        }
        let metadata_hash = stable_json_hash(&properties)?;
        Ok(Self {
            properties,
            metadata_hash,
        })
    }

    pub fn required(&self, property: &'static str) -> Result<&str, DataError> {
        self.properties
            .get(property)
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or(DataError::MissingMetadataProperty(property))
    }

    pub fn source_timezone(&self) -> Result<SourceTimezone, DataError> {
        self.required("broker_timezone")?.parse()
    }

    pub fn validate_dataset(&self, dataset: &BarDataset) -> Result<(), DataError> {
        let declared_count = self.required("bar_count")?.parse::<usize>().map_err(|_| {
            DataError::InvalidMetadataProperty {
                property: "bar_count",
                reason: "must be an unsigned integer",
            }
        })?;
        if declared_count != dataset.bars.len() {
            return Err(DataError::MetadataDatasetMismatch(format!(
                "metadata declares {declared_count} bars but data contains {}",
                dataset.bars.len()
            )));
        }
        if self.source_timezone()?.name() != dataset.source_timezone {
            return Err(DataError::MetadataDatasetMismatch(format!(
                "metadata timezone does not match normalized source timezone {}",
                dataset.source_timezone
            )));
        }

        if let Some(expected) = timeframe_seconds(self.required("timeframe")?) {
            let report = DataQualityReport::analyze(dataset);
            if report.expected_interval_seconds != Some(expected) {
                return Err(DataError::MetadataDatasetMismatch(format!(
                    "metadata timeframe expects {expected}s bars but observed median is {:?}",
                    report.expected_interval_seconds
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityGrade {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataQualityReport {
    pub grade: QualityGrade,
    pub score: u8,
    pub bar_count: usize,
    pub expected_interval_seconds: Option<u64>,
    pub missing_bar_estimate: u64,
    pub gap_events: usize,
    pub duplicate_rows_removed: usize,
    pub zero_range_bars: usize,
    pub ohlc_violations: usize,
    pub spike_bars: usize,
    pub weekend_bars: usize,
    pub input_was_sorted: bool,
}

impl DataQualityReport {
    pub fn analyze(dataset: &BarDataset) -> Self {
        let bars = &dataset.bars;
        let source_timezone = dataset.source_timezone.parse::<SourceTimezone>().ok();
        let intervals: Vec<u64> = bars
            .windows(2)
            .filter_map(|pair| {
                let delta = pair[1].timestamp_ms - pair[0].timestamp_ms;
                (delta > 0).then_some(delta as u64 / 1_000)
            })
            .collect();
        let expected_interval_seconds = median_u64(&intervals);

        let mut gap_events = 0;
        let mut missing_bar_estimate = 0u64;
        if let Some(expected) = expected_interval_seconds
            && expected > 0
        {
            for pair in bars.windows(2) {
                let delta = (pair[1].timestamp_ms - pair[0].timestamp_ms) as u64 / 1_000;
                if delta > expected + expected / 2
                    && !is_weekend_closure(&pair[0], &pair[1], source_timezone)
                {
                    gap_events += 1;
                    missing_bar_estimate += delta.saturating_div(expected).saturating_sub(1);
                }
            }
        }

        let zero_range_bars = bars.iter().filter(|bar| bar.high == bar.low).count();
        let ohlc_violations = bars
            .iter()
            .filter(|bar| {
                !all_prices_finite(bar)
                    || bar.high < bar.low
                    || bar.high < bar.open.max(bar.close)
                    || bar.low > bar.open.min(bar.close)
            })
            .count();
        let weekend_bars = bars
            .iter()
            .filter(|bar| is_source_weekend(bar, source_timezone))
            .count();

        let ranges: Vec<f64> = bars
            .iter()
            .map(|bar| bar.high - bar.low)
            .filter(|range| range.is_finite() && *range > 0.0)
            .collect();
        let median_range = median_f64(&ranges).unwrap_or(0.0);
        let spike_bars = if median_range > 0.0 {
            bars.iter()
                .filter(|bar| bar.high - bar.low > median_range * 20.0)
                .count()
        } else {
            0
        };

        let count = bars.len().max(1) as f64;
        let missing_denominator = bars.len() as u64 + missing_bar_estimate;
        let missing_ratio = if missing_denominator == 0 {
            0.0
        } else {
            missing_bar_estimate as f64 / missing_denominator as f64
        };
        let penalty = (ohlc_violations as f64 / count * 100.0).min(50.0)
            + (dataset.duplicate_rows_removed as f64 / dataset.source_rows.max(1) as f64 * 100.0)
                .min(10.0)
            + (missing_ratio * 100.0).min(25.0)
            + (zero_range_bars as f64 / count * 100.0).min(10.0)
            + (weekend_bars as f64 / count * 100.0).min(10.0)
            + (spike_bars as f64 / count * 100.0).min(10.0)
            + if dataset.input_was_sorted { 0.0 } else { 2.0 };
        let score = (100.0 - penalty).clamp(0.0, 100.0).round() as u8;
        let grade = match score {
            90..=100 => QualityGrade::Pass,
            70..=89 => QualityGrade::Warn,
            _ => QualityGrade::Fail,
        };

        Self {
            grade,
            score,
            bar_count: bars.len(),
            expected_interval_seconds,
            missing_bar_estimate,
            gap_events,
            duplicate_rows_removed: dataset.duplicate_rows_removed,
            zero_range_bars,
            ohlc_violations,
            spike_bars,
            weekend_bars,
            input_was_sorted: dataset.input_was_sorted,
        }
    }
}

#[derive(Debug)]
struct Columns {
    date: Option<usize>,
    time: Option<usize>,
    timestamp: Option<usize>,
    open: usize,
    high: usize,
    low: usize,
    close: usize,
    tick_volume: Option<usize>,
    real_volume: Option<usize>,
    spread: Option<usize>,
}

impl Columns {
    fn from_headers(headers: &StringRecord) -> Result<Self, DataError> {
        let headers: Vec<String> = headers.iter().map(normalize_header).collect();
        let find = |names: &[&str]| {
            headers
                .iter()
                .position(|header| names.contains(&header.as_str()))
        };
        let required = |names: &[&str], display: &'static str| {
            find(names).ok_or(DataError::MissingColumn(display))
        };

        let date = find(&["DATE"]);
        let time = find(&["TIME"]);
        let timestamp = find(&["DATETIME", "TIMESTAMP", "DATE_TIME"]);
        if timestamp.is_none() && (date.is_none() || time.is_none()) {
            return Err(DataError::MissingTimestampColumns);
        }

        Ok(Self {
            date,
            time,
            timestamp,
            open: required(&["OPEN"], "OPEN")?,
            high: required(&["HIGH"], "HIGH")?,
            low: required(&["LOW"], "LOW")?,
            close: required(&["CLOSE"], "CLOSE")?,
            tick_volume: find(&["TICKVOL", "TICK_VOLUME", "TICKVOLUME"]),
            real_volume: find(&["VOL", "VOLUME", "REAL_VOLUME", "REALVOL"]),
            spread: find(&["SPREAD", "SPREAD_POINTS"]),
        })
    }

    fn parse(
        &self,
        record: &StringRecord,
        row: usize,
        source_timezone: SourceTimezone,
    ) -> Result<Bar, DataError> {
        let timestamp_text = if let Some(index) = self.timestamp {
            value(record, index, row)?.to_owned()
        } else {
            format!(
                "{} {}",
                value(record, self.date.unwrap(), row)?,
                value(record, self.time.unwrap(), row)?
            )
        };

        Ok(Bar {
            timestamp_ms: parse_timestamp(&timestamp_text, source_timezone).map_err(|reason| {
                DataError::InvalidTimestamp {
                    row,
                    value: timestamp_text,
                    reason,
                }
            })?,
            open: parse_f64(record, self.open, row, "OPEN")?,
            high: parse_f64(record, self.high, row, "HIGH")?,
            low: parse_f64(record, self.low, row, "LOW")?,
            close: parse_f64(record, self.close, row, "CLOSE")?,
            tick_volume: parse_optional(record, self.tick_volume, row, "TICKVOL")?.unwrap_or(0),
            real_volume: parse_optional(record, self.real_volume, row, "VOL")?.unwrap_or(0),
            spread_points: parse_optional(record, self.spread, row, "SPREAD")?,
        })
    }
}

fn detect_delimiter(bytes: &[u8]) -> Result<u8, DataError> {
    let first_line = bytes
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .find(|line| !line.is_empty())
        .ok_or(DataError::NoRows)?;
    let choices = *b"\t,;";
    choices
        .into_iter()
        .map(|delimiter| {
            (
                delimiter,
                first_line.iter().filter(|byte| **byte == delimiter).count(),
            )
        })
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count > 0)
        .map(|(delimiter, _)| delimiter)
        .ok_or(DataError::UnknownDelimiter)
}

fn normalize_header(header: &str) -> String {
    header
        .trim()
        .trim_matches(|character| character == '<' || character == '>')
        .replace([' ', '-'], "_")
        .to_ascii_uppercase()
}

fn value(record: &StringRecord, index: usize, row: usize) -> Result<&str, DataError> {
    record.get(index).ok_or(DataError::ShortRow(row))
}

fn parse_f64(
    record: &StringRecord,
    index: usize,
    row: usize,
    field: &'static str,
) -> Result<f64, DataError> {
    let raw = value(record, index, row)?;
    let parsed = raw.parse::<f64>().map_err(|_| DataError::InvalidNumber {
        row,
        field,
        value: raw.into(),
    })?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(DataError::InvalidNumber {
            row,
            field,
            value: raw.into(),
        })
    }
}

fn parse_optional<T: std::str::FromStr>(
    record: &StringRecord,
    index: Option<usize>,
    row: usize,
    field: &'static str,
) -> Result<Option<T>, DataError> {
    let Some(index) = index else {
        return Ok(None);
    };
    let raw = value(record, index, row)?;
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<T>()
        .map(Some)
        .map_err(|_| DataError::InvalidNumber {
            row,
            field,
            value: raw.into(),
        })
}

fn parse_timestamp(value: &str, source_timezone: SourceTimezone) -> Result<i64, &'static str> {
    if let Ok(seconds) = value.parse::<i64>() {
        return Ok(if seconds.abs() < 10_000_000_000 {
            seconds * 1_000
        } else {
            seconds
        });
    }
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(timestamp.timestamp_millis());
    }
    for format in [
        "%Y.%m.%d %H:%M:%S",
        "%Y.%m.%d %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M:%S",
    ] {
        if let Ok(timestamp) = NaiveDateTime::parse_from_str(value, format) {
            return match source_timezone.localize(timestamp) {
                LocalResult::Single(timestamp) => Ok(timestamp.timestamp_millis()),
                LocalResult::Ambiguous(_, _) => {
                    Err("local time is ambiguous at a daylight-saving transition")
                }
                LocalResult::None => {
                    Err("local time does not exist at a daylight-saving transition")
                }
            };
        }
    }
    Err("unsupported timestamp format")
}

fn timeframe_seconds(value: &str) -> Option<u64> {
    match value.trim().to_ascii_uppercase().as_str() {
        "PERIOD_M1" | "M1" => Some(60),
        "PERIOD_M5" | "M5" => Some(300),
        "PERIOD_M15" | "M15" => Some(900),
        "PERIOD_M30" | "M30" => Some(1_800),
        "PERIOD_H1" | "H1" => Some(3_600),
        "PERIOD_H4" | "H4" => Some(14_400),
        "PERIOD_D1" | "D1" => Some(86_400),
        _ => None,
    }
}

/// Computes the canonical QuantForge identity for an ordered bar slice.
///
/// This is also used to bind chronological development, validation and sealed
/// partitions without serializing or exposing the bars themselves.
pub fn bar_content_hash(bars: &[Bar]) -> ContentHash {
    let mut bytes = Vec::with_capacity(bars.len() * 64);
    for bar in bars {
        bytes.extend_from_slice(&bar.timestamp_ms.to_le_bytes());
        bytes.extend_from_slice(&bar.open.to_bits().to_le_bytes());
        bytes.extend_from_slice(&bar.high.to_bits().to_le_bytes());
        bytes.extend_from_slice(&bar.low.to_bits().to_le_bytes());
        bytes.extend_from_slice(&bar.close.to_bits().to_le_bytes());
        bytes.extend_from_slice(&bar.tick_volume.to_le_bytes());
        bytes.extend_from_slice(&bar.real_volume.to_le_bytes());
        bytes.extend_from_slice(&bar.spread_points.unwrap_or(u32::MAX).to_le_bytes());
    }
    ContentHash::sha256(bytes)
}

fn median_u64(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut values = values.to_vec();
    values.sort_unstable();
    Some(values[values.len() / 2])
}

fn median_f64(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    Some(values[values.len() / 2])
}

fn all_prices_finite(bar: &Bar) -> bool {
    [bar.open, bar.high, bar.low, bar.close]
        .into_iter()
        .all(f64::is_finite)
}

fn source_wall_time(bar: &Bar, source_timezone: Option<SourceTimezone>) -> Option<NaiveDateTime> {
    source_timezone
        .and_then(|timezone| timezone.source_wall_time(bar.timestamp_ms))
        .or_else(|| {
            Utc.timestamp_millis_opt(bar.timestamp_ms)
                .single()
                .map(|timestamp| timestamp.naive_utc())
        })
}

fn is_source_weekend(bar: &Bar, source_timezone: Option<SourceTimezone>) -> bool {
    source_wall_time(bar, source_timezone)
        .is_some_and(|time| matches!(time.weekday(), Weekday::Sat | Weekday::Sun))
}

fn is_weekend_closure(previous: &Bar, next: &Bar, source_timezone: Option<SourceTimezone>) -> bool {
    let Some(previous) = source_wall_time(previous, source_timezone) else {
        return false;
    };
    let Some(next) = source_wall_time(next, source_timezone) else {
        return false;
    };

    previous.weekday() == Weekday::Fri && next.weekday() == Weekday::Mon && previous.hour() >= 18
}

#[derive(Debug, Error)]
pub enum DataError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Csv(#[from] csv::Error),
    #[error("input contains no data rows")]
    NoRows,
    #[error("could not detect tab, comma, or semicolon delimiter")]
    UnknownDelimiter,
    #[error("missing required column {0}")]
    MissingColumn(&'static str),
    #[error("input must contain DATETIME/TIMESTAMP or both DATE and TIME columns")]
    MissingTimestampColumns,
    #[error("row {0} has fewer fields than its header")]
    ShortRow(usize),
    #[error("unknown IANA timezone: {0}")]
    UnknownTimezone(String),
    #[error("invalid timestamp on row {row}: {value} ({reason})")]
    InvalidTimestamp {
        row: usize,
        value: String,
        reason: &'static str,
    },
    #[error("invalid numeric {field} on row {row}: {value}")]
    InvalidNumber {
        row: usize,
        field: &'static str,
        value: String,
    },
    #[error("metadata contains duplicate property: {0}")]
    DuplicateMetadataProperty(String),
    #[error("metadata is missing required property: {0}")]
    MissingMetadataProperty(&'static str),
    #[error("invalid metadata property {property}: {reason}")]
    InvalidMetadataProperty {
        property: &'static str,
        reason: &'static str,
    },
    #[error("metadata does not match dataset: {0}")]
    MetadataDatasetMismatch(String),
    #[error(transparent)]
    Hash(#[from] HashError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_mt5_tabs_sorts_and_keeps_last_duplicate() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("EURUSD_M15.csv");
        fs::write(
            &path,
            concat!(
                "<DATE>\t<TIME>\t<OPEN>\t<HIGH>\t<LOW>\t<CLOSE>\t<TICKVOL>\t<VOL>\t<SPREAD>\n",
                "2024.01.01\t00:15:00\t1.1\t1.2\t1.0\t1.15\t10\t0\t8\n",
                "2024.01.01\t00:00:00\t1.0\t1.1\t0.9\t1.05\t12\t0\t7\n",
                "2024.01.01\t00:15:00\t1.1\t1.3\t1.0\t1.20\t11\t0\t9\n",
            ),
        )
        .unwrap();

        let dataset = BarDataset::load_mt5(path, "Etc/UTC".parse().unwrap()).unwrap();
        assert_eq!(dataset.delimiter, '\t');
        assert_eq!(dataset.source_rows, 3);
        assert_eq!(dataset.bars.len(), 2);
        assert_eq!(dataset.duplicate_rows_removed, 1);
        assert!(!dataset.input_was_sorted);
        assert_eq!(dataset.bars[1].high, 1.3);
    }

    #[test]
    fn clean_regular_data_passes_quality_gate() {
        let bars = (0..100)
            .map(|index| Bar {
                timestamp_ms: 1_704_067_200_000 + index * 900_000,
                open: 1.1,
                high: 1.2,
                low: 1.0,
                close: 1.15,
                tick_volume: 10,
                real_volume: 0,
                spread_points: Some(8),
            })
            .collect::<Vec<_>>();
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            bars,
            source_rows: 100,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        };

        let report = DataQualityReport::analyze(&dataset);
        assert_eq!(report.grade, QualityGrade::Pass);
        assert_eq!(report.expected_interval_seconds, Some(900));
    }

    #[test]
    fn naive_broker_time_is_normalized_with_explicit_timezone() {
        let utc = parse_timestamp("2024.01.08 00:00:00", "Etc/UTC".parse().unwrap()).unwrap();
        let helsinki =
            parse_timestamp("2024.01.08 02:00:00", "Europe/Helsinki".parse().unwrap()).unwrap();
        assert_eq!(utc, helsinki);
    }

    #[test]
    fn ic_markets_est_plus_seven_uses_new_york_dst_rules() {
        let winter_utc =
            parse_timestamp("2024.01.08 07:00:00", "Etc/UTC".parse().unwrap()).unwrap();
        let winter_server =
            parse_timestamp("2024.01.08 09:00:00", "ICMarkets/EST+7".parse().unwrap()).unwrap();
        assert_eq!(winter_utc, winter_server);

        let summer_utc =
            parse_timestamp("2024.07.08 06:00:00", "Etc/UTC".parse().unwrap()).unwrap();
        let summer_server =
            parse_timestamp("2024.07.08 09:00:00", "ICMarkets/EST+7".parse().unwrap()).unwrap();
        assert_eq!(summer_utc, summer_server);
    }

    #[test]
    fn quality_classifies_ic_markets_weekends_in_broker_wall_time() {
        let source_timezone: SourceTimezone = "ICMarkets/EST+7".parse().unwrap();
        let bars = [
            "2024.01.05 20:00:00",
            "2024.01.05 21:00:00",
            "2024.01.05 22:00:00",
            "2024.01.05 23:00:00",
            "2024.01.08 00:00:00",
            "2024.01.08 01:00:00",
            "2024.01.08 02:00:00",
            "2024.01.08 03:00:00",
        ]
        .into_iter()
        .map(|timestamp| Bar {
            timestamp_ms: parse_timestamp(timestamp, source_timezone).unwrap(),
            open: 1.1,
            high: 1.2,
            low: 1.0,
            close: 1.15,
            tick_volume: 10,
            real_volume: 0,
            spread_points: Some(8),
        })
        .collect::<Vec<_>>();
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            bars,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: source_timezone.to_string(),
        };

        let report = DataQualityReport::analyze(&dataset);

        assert_eq!(report.expected_interval_seconds, Some(3_600));
        assert_eq!(report.gap_events, 0);
        assert_eq!(report.missing_bar_estimate, 0);
        assert_eq!(report.weekend_bars, 0);
        assert_eq!(report.score, 100);
        assert_eq!(report.grade, QualityGrade::Pass);
    }

    #[test]
    fn quality_still_flags_true_ic_markets_weekend_bars() {
        let source_timezone: SourceTimezone = "ICMarkets/EST+7".parse().unwrap();
        let bars = [
            "2024.01.05 23:00:00",
            "2024.01.06 00:00:00",
            "2024.01.06 01:00:00",
        ]
        .into_iter()
        .map(|timestamp| Bar {
            timestamp_ms: parse_timestamp(timestamp, source_timezone).unwrap(),
            open: 1.1,
            high: 1.2,
            low: 1.0,
            close: 1.15,
            tick_volume: 10,
            real_volume: 0,
            spread_points: Some(8),
        })
        .collect::<Vec<_>>();
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            source_rows: bars.len(),
            bars,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: source_timezone.to_string(),
        };

        let report = DataQualityReport::analyze(&dataset);

        assert_eq!(report.weekend_bars, 2);
    }

    #[test]
    fn exporter_metadata_binds_count_timeframe_and_timezone() {
        let directory = tempfile::tempdir().unwrap();
        let metadata_path = directory.path().join("sample.metadata.csv");
        fs::write(
            &metadata_path,
            concat!(
                "property,value\n",
                "bar_count,2\n",
                "timeframe,PERIOD_M15\n",
                "broker_timezone,Etc/UTC\n",
                "broker,Fixture Broker\n",
                "server,Fixture-Demo\n",
            ),
        )
        .unwrap();
        let bars = vec![
            Bar {
                timestamp_ms: 1_704_067_200_000,
                open: 1.0,
                high: 1.1,
                low: 0.9,
                close: 1.0,
                tick_volume: 1,
                real_volume: 0,
                spread_points: Some(8),
            },
            Bar {
                timestamp_ms: 1_704_068_100_000,
                open: 1.0,
                high: 1.1,
                low: 0.9,
                close: 1.0,
                tick_volume: 1,
                real_volume: 0,
                spread_points: Some(8),
            },
        ];
        let dataset = BarDataset {
            data_hash: bar_content_hash(&bars),
            bars,
            source_rows: 2,
            duplicate_rows_removed: 0,
            input_was_sorted: true,
            delimiter: '\t',
            source_timezone: "Etc/UTC".into(),
        };

        let metadata = Mt5ExportMetadata::load(metadata_path).unwrap();
        metadata.validate_dataset(&dataset).unwrap();
    }
}
