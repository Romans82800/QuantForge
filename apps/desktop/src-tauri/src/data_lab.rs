use quantforge_broker::{DayOfWeek, SymbolSpecification};
use quantforge_data::{
    BarDataset, DataQualityReport, Mt5ExportMetadata, QualityGrade, SourceTimezone,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataLabRequest {
    data_path: String,
    metadata_path: Option<String>,
    source_timezone: Option<String>,
    broker_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataLabView {
    source_path: String,
    metadata_path: Option<String>,
    broker_path: Option<String>,
    data_hash: String,
    metadata_hash: Option<String>,
    broker_spec_hash: Option<String>,
    symbol: Option<String>,
    timeframe: Option<String>,
    broker_profile: Option<String>,
    source_rows: usize,
    bars: usize,
    duplicate_rows_removed: usize,
    input_was_sorted: bool,
    delimiter: String,
    source_timezone: String,
    first_timestamp_ms: i64,
    last_timestamp_ms: i64,
    quality: QualityView,
    discover_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityView {
    grade: String,
    score: u8,
    bar_count: usize,
    expected_interval_seconds: Option<u64>,
    missing_bar_estimate: u64,
    gap_events: usize,
    duplicate_rows_removed: usize,
    zero_range_bars: usize,
    ohlc_violations: usize,
    spike_bars: usize,
    weekend_bars: usize,
    input_was_sorted: bool,
}

pub(crate) struct LoadedDataSource {
    pub(crate) dataset: BarDataset,
    pub(crate) metadata: Option<Mt5ExportMetadata>,
}

#[tauri::command]
pub async fn inspect_data(request: DataLabRequest) -> Result<DataLabView, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_data_sync(&request))
        .await
        .map_err(|error| format!("data inspection task failed: {error}"))?
}

fn inspect_data_sync(request: &DataLabRequest) -> Result<DataLabView, String> {
    let loaded = load_data_source(
        &request.data_path,
        request.metadata_path.as_deref(),
        request.source_timezone.as_deref(),
    )?;
    let quality = DataQualityReport::analyze(&loaded.dataset);
    let broker = request
        .broker_path
        .as_deref()
        .map(|path| load_bound_broker(path, loaded.metadata.as_ref()))
        .transpose()?;
    let metadata = loaded.metadata.as_ref();
    let first_timestamp_ms = loaded
        .dataset
        .bars
        .first()
        .ok_or_else(|| "data source contains no bars".to_owned())?
        .timestamp_ms;
    let last_timestamp_ms = loaded
        .dataset
        .bars
        .last()
        .ok_or_else(|| "data source contains no bars".to_owned())?
        .timestamp_ms;
    let broker_spec_hash = broker
        .as_ref()
        .map(SymbolSpecification::content_hash)
        .transpose()
        .map_err(|error| error.to_string())?;

    Ok(DataLabView {
        source_path: display_path(Path::new(&request.data_path)),
        metadata_path: request
            .metadata_path
            .as_deref()
            .map(|path| display_path(Path::new(path))),
        broker_path: request
            .broker_path
            .as_deref()
            .map(|path| display_path(Path::new(path))),
        data_hash: loaded.dataset.data_hash.as_str().into(),
        metadata_hash: metadata.map(|value| value.metadata_hash.as_str().to_owned()),
        broker_spec_hash: broker_spec_hash.map(|value| value.as_str().to_owned()),
        symbol: metadata
            .and_then(|value| value.properties.get("symbol").cloned())
            .or_else(|| broker.as_ref().map(|value| value.symbol.clone())),
        timeframe: metadata.and_then(|value| value.properties.get("timeframe").cloned()),
        broker_profile: broker.as_ref().map(|value| value.profile_name.clone()),
        source_rows: loaded.dataset.source_rows,
        bars: loaded.dataset.bars.len(),
        duplicate_rows_removed: loaded.dataset.duplicate_rows_removed,
        input_was_sorted: loaded.dataset.input_was_sorted,
        delimiter: loaded.dataset.delimiter.to_string(),
        source_timezone: loaded.dataset.source_timezone.clone(),
        first_timestamp_ms,
        last_timestamp_ms,
        discover_ready: quality.grade != QualityGrade::Fail && broker.is_some(),
        quality: QualityView::from(&quality),
    })
}

pub(crate) fn load_data_source(
    data_path: &str,
    metadata_path: Option<&str>,
    source_timezone: Option<&str>,
) -> Result<LoadedDataSource, String> {
    let metadata = metadata_path
        .map(Mt5ExportMetadata::load)
        .transpose()
        .map_err(|error| error.to_string())?;
    let timezone: SourceTimezone = match (&metadata, source_timezone) {
        (Some(metadata), None) => metadata
            .source_timezone()
            .map_err(|error| error.to_string())?,
        (None, Some(timezone)) => timezone
            .parse()
            .map_err(|error: quantforge_data::DataError| error.to_string())?,
        _ => return Err("provide exactly one metadata file or source timezone".into()),
    };
    let dataset = BarDataset::load_mt5(data_path, timezone).map_err(|error| error.to_string())?;
    if let Some(metadata) = &metadata {
        metadata
            .validate_dataset(&dataset)
            .map_err(|error| error.to_string())?;
    }
    Ok(LoadedDataSource { dataset, metadata })
}

pub(crate) fn load_bound_broker(
    path: &str,
    metadata: Option<&Mt5ExportMetadata>,
) -> Result<SymbolSpecification, String> {
    let broker: SymbolSpecification = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("cannot read broker profile: {error}"))?,
    )
    .map_err(|error| format!("broker profile JSON is invalid: {error}"))?;
    broker.validate().map_err(|error| error.to_string())?;
    validate_metadata_broker_binding(metadata, &broker)?;
    Ok(broker)
}

fn validate_metadata_broker_binding(
    metadata: Option<&Mt5ExportMetadata>,
    broker: &SymbolSpecification,
) -> Result<(), String> {
    let Some(metadata) = metadata else {
        return Ok(());
    };
    if metadata
        .required("symbol")
        .map_err(|error| error.to_string())?
        != broker.symbol
    {
        return Err(format!(
            "metadata symbol does not match broker profile {}",
            broker.symbol
        ));
    }
    if metadata
        .source_timezone()
        .map_err(|error| error.to_string())?
        .name()
        != broker.timezone
    {
        return Err(format!(
            "metadata timezone does not match broker profile {}",
            broker.timezone
        ));
    }
    for (property, expected) in [
        ("account_currency", broker.account_currency.as_str()),
        ("currency_base", broker.base_currency.as_str()),
        ("currency_profit", broker.profit_currency.as_str()),
        ("currency_margin", broker.margin_currency.as_str()),
    ] {
        if let Some(actual) = metadata.properties.get(property)
            && actual != expected
        {
            return Err(format!(
                "metadata {property} {actual} does not match broker profile {expected}"
            ));
        }
    }
    for (property, day) in [
        ("swap_multiplier_sunday", DayOfWeek::Sunday),
        ("swap_multiplier_monday", DayOfWeek::Monday),
        ("swap_multiplier_tuesday", DayOfWeek::Tuesday),
        ("swap_multiplier_wednesday", DayOfWeek::Wednesday),
        ("swap_multiplier_thursday", DayOfWeek::Thursday),
        ("swap_multiplier_friday", DayOfWeek::Friday),
        ("swap_multiplier_saturday", DayOfWeek::Saturday),
    ] {
        if let Some(actual) = metadata.properties.get(property) {
            let actual = actual
                .parse::<f64>()
                .map_err(|_| format!("metadata {property} is not numeric"))?;
            if (actual - f64::from(broker.swap_multiplier(day))).abs() > 1.0e-9 {
                return Err(format!("metadata {property} does not match broker profile"));
            }
        }
    }
    Ok(())
}

pub(crate) fn display_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .display()
        .to_string()
}

impl From<&DataQualityReport> for QualityView {
    fn from(report: &DataQualityReport) -> Self {
        Self {
            grade: format!("{:?}", report.grade).to_ascii_lowercase(),
            score: report.score,
            bar_count: report.bar_count,
            expected_interval_seconds: report.expected_interval_seconds,
            missing_bar_estimate: report.missing_bar_estimate,
            gap_events: report.gap_events,
            duplicate_rows_removed: report.duplicate_rows_removed,
            zero_range_bars: report.zero_range_bars,
            ohlc_violations: report.ohlc_violations,
            spike_bars: report.spike_bars,
            weekend_bars: report.weekend_bars,
            input_was_sorted: report.input_was_sorted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .join("fixtures")
            .join(name)
    }

    #[test]
    fn inspection_binds_fixture_data_metadata_and_broker() {
        let request = DataLabRequest {
            data_path: fixture("EURUSD_M15_sample.tsv").display().to_string(),
            metadata_path: Some(
                fixture("EURUSD_M15_sample.metadata.csv")
                    .display()
                    .to_string(),
            ),
            source_timezone: None,
            broker_path: Some(fixture("EURUSD_fixture_broker.json").display().to_string()),
        };
        let view = inspect_data_sync(&request).expect("fixture should inspect");
        assert_eq!(view.bars, 6);
        assert_eq!(view.quality.grade, "pass");
        assert_eq!(view.symbol.as_deref(), Some("EURUSD"));
        assert!(view.metadata_hash.is_some());
        assert!(view.broker_spec_hash.is_some());
        assert!(view.discover_ready);
    }

    #[test]
    fn inspection_requires_one_timezone_authority() {
        let error = load_data_source("unused.tsv", None, None)
            .err()
            .expect("missing timezone authority must fail");
        assert!(error.contains("exactly one metadata file or source timezone"));
    }
}
