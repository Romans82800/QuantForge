use csv::{ReaderBuilder, StringRecord, Trim};
use quantforge_broker::{DayOfWeek, SymbolSpecification};
use quantforge_data::{
    Bar, BarDataset, DataQualityReport, Mt5ExportMetadata, QualityGrade, QuoteBar, QuoteBarDataset,
    SourceTimezone, build_timeframe_from_m1, infer_median_interval_ms, parse_source_timestamp,
    quote_bar_content_hash,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
    /// The execution-feed class inferred from importer metadata. Midpoint
    /// packs remain readable for diagnostics, but are never eligible for a
    /// certified discovery run.
    feed_mode: String,
    quote_path: Option<String>,
    certification_ready: bool,
    first_timestamp_ms: i64,
    last_timestamp_ms: i64,
    quality: QualityView,
    discover_ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketFolderImportRequest {
    pub source_directory: String,
    pub output_directory: Option<String>,
    pub source_timezone: String,
    #[serde(default = "default_true")]
    pub aggregate_ticks_to_bars: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketFileImportView {
    pub source_path: String,
    pub symbol: Option<String>,
    pub kind: String,
    pub source_rows: usize,
    pub bars: usize,
    pub m1_path: Option<String>,
    pub m1_metadata_path: Option<String>,
    pub h1_path: Option<String>,
    pub h1_metadata_path: Option<String>,
    pub quote_path: Option<String>,
    pub quote_metadata_path: Option<String>,
    pub price_basis: Option<String>,
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketFolderImportView {
    pub source_directory: String,
    pub output_directory: String,
    pub source_timezone: String,
    pub files: Vec<MarketFileImportView>,
    pub imported_count: usize,
    pub skipped_count: usize,
}

fn default_true() -> bool {
    true
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
    let import_kind = metadata
        .and_then(|value| value.properties.get("import_kind"))
        .map(String::as_str);
    let price_basis = metadata
        .and_then(|value| value.properties.get("price_basis"))
        .map(String::as_str);
    let is_midpoint = price_basis.is_some_and(|value| value.eq_ignore_ascii_case("midpoint"))
        || import_kind.is_some_and(|value| value.to_ascii_lowercase().contains("midpoint"));
    let is_canonical_quotes = price_basis.is_some_and(|value| {
        value.eq_ignore_ascii_case("bid") || value.eq_ignore_ascii_case("bid_ask")
    }) && import_kind
        .is_some_and(|value| value.to_ascii_lowercase().contains("bid_ask"));
    let feed_mode = if is_canonical_quotes {
        "canonical_bid_ask"
    } else if is_midpoint {
        "diagnostic_midpoint"
    } else {
        "legacy_ohlc"
    };
    let quote_path = if is_canonical_quotes {
        let data_path = Path::new(&request.data_path);
        let stem = data_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let mut candidates = vec![data_path.with_file_name(format!("{stem}.quotes.csv"))];
        for suffix in ["_H1", "_M15"] {
            if let Some(base) = stem.strip_suffix(suffix) {
                candidates.push(data_path.with_file_name(format!("{base}_M1.quotes.csv")));
            }
        }
        candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .map(|candidate| display_path(&candidate))
    } else {
        None
    };
    let certification_ready = is_canonical_quotes && quote_path.is_some() && broker.is_some();
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
        feed_mode: feed_mode.into(),
        quote_path,
        certification_ready,
        first_timestamp_ms,
        last_timestamp_ms,
        discover_ready: quality.grade != QualityGrade::Fail && broker.is_some() && !is_midpoint,
        quality: QualityView::from(&quality),
    })
}

/// Import a folder of broker exports without making the user hand-bind every
/// file. IC Markets' downloadable `*_TickData.csv` files are aggregated to
/// bid M1 bars and a sidecar carrying the matching bid/ask OHLC quotes. H1 is
/// always derived from the canonical bid M1 stream. Non-market CSVs (trade
/// lists, Databento files, etc.) are reported as skipped rather than
/// accidentally being treated as prices.
#[tauri::command]
pub async fn import_market_folder(
    request: MarketFolderImportRequest,
) -> Result<MarketFolderImportView, String> {
    tauri::async_runtime::spawn_blocking(move || import_market_folder_sync(&request))
        .await
        .map_err(|error| format!("market import task failed: {error}"))?
}

pub fn import_market_folder_sync(
    request: &MarketFolderImportRequest,
) -> Result<MarketFolderImportView, String> {
    let source_directory = PathBuf::from(request.source_directory.trim());
    if !source_directory.is_dir() {
        return Err(format!(
            "market-data folder does not exist: {}",
            source_directory.display()
        ));
    }
    let timezone: SourceTimezone = request
        .source_timezone
        .parse()
        .map_err(|error: quantforge_data::DataError| error.to_string())?;
    let output_directory = request
        .output_directory
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_import_directory);
    fs::create_dir_all(&output_directory)
        .map_err(|error| format!("cannot create import directory: {error}"))?;

    let mut paths = Vec::new();
    collect_market_files(&source_directory, &mut paths)?;
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    let mut imported_count = 0;
    let mut skipped_count = 0;

    for source_path in paths {
        let result = import_market_file(
            &source_path,
            &output_directory,
            timezone,
            request.aggregate_ticks_to_bars,
        );
        match result {
            Ok(mut view) => {
                if view.status == "imported" {
                    imported_count += 1;
                } else {
                    skipped_count += 1;
                }
                view.source_path = display_path(&source_path);
                files.push(view);
            }
            Err(error) => {
                skipped_count += 1;
                files.push(MarketFileImportView {
                    source_path: display_path(&source_path),
                    symbol: symbol_from_path(&source_path),
                    kind: "unknown".into(),
                    source_rows: 0,
                    bars: 0,
                    m1_path: None,
                    m1_metadata_path: None,
                    h1_path: None,
                    h1_metadata_path: None,
                    quote_path: None,
                    quote_metadata_path: None,
                    price_basis: None,
                    status: "error".into(),
                    message: Some(error),
                });
            }
        }
    }

    Ok(MarketFolderImportView {
        source_directory: display_path(&source_directory),
        output_directory: display_path(&output_directory),
        source_timezone: timezone.to_string(),
        files,
        imported_count,
        skipped_count,
    })
}

fn default_import_directory() -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("Documents")
        .join("QuantForge")
        .join("imported-data")
        .join(stamp.to_string())
}

fn collect_market_files(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read folder entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            collect_market_files(&path, output)?;
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| {
                matches!(value.to_ascii_lowercase().as_str(), "csv" | "tsv" | "txt")
            })
        {
            output.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct TickColumns {
    date: Option<usize>,
    time: Option<usize>,
    timestamp: Option<usize>,
    ask: usize,
    bid: usize,
    volume: Option<usize>,
}

impl TickColumns {
    fn from_headers(headers: &StringRecord) -> Option<Self> {
        let headers: Vec<String> = headers.iter().map(normalize_market_header).collect();
        let find = |names: &[&str]| {
            headers
                .iter()
                .position(|value| names.contains(&value.as_str()))
        };
        let ask = find(&["ASK"])?;
        let bid = find(&["BID"])?;
        let date = find(&["DATE"]);
        let time = find(&["TIME"]);
        let timestamp = find(&["DATETIME", "TIMESTAMP", "DATE_TIME"])
            .or_else(|| date.is_none().then_some(time).flatten());
        if timestamp.is_none() && (date.is_none() || time.is_none()) {
            return None;
        }
        Some(Self {
            date,
            time,
            timestamp,
            ask,
            bid,
            volume: find(&["VOLUME", "VOL", "TICKVOL", "TICK_VOLUME"]),
        })
    }
}

fn import_market_file(
    source_path: &Path,
    output_directory: &Path,
    timezone: SourceTimezone,
    aggregate_ticks_to_bars: bool,
) -> Result<MarketFileImportView, String> {
    let bytes = fs::read(source_path).map_err(|error| format!("cannot read source: {error}"))?;
    let delimiter = detect_market_delimiter(&bytes)?;
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(Trim::All)
        .flexible(true)
        .from_reader(bytes.as_slice());
    let headers = reader
        .headers()
        .map_err(|error| format!("invalid CSV header: {error}"))?
        .clone();
    let normalized: Vec<String> = headers.iter().map(normalize_market_header).collect();
    let symbol = symbol_from_headers(&normalized).or_else(|| symbol_from_path(source_path));

    // Databento's multi-symbol OHLCV export has price columns but is not an
    // IC Markets/MT5 source. It must not silently enter a broker data pack.
    if normalized.iter().any(|value| value == "RTYPE")
        && normalized.iter().any(|value| value == "PUBLISHER_ID")
    {
        return Ok(skipped_view(
            source_path,
            symbol,
            "unsupported",
            "Databento multi-symbol OHLCV export",
        ));
    }

    if let Some(columns) = TickColumns::from_headers(&headers) {
        if !aggregate_ticks_to_bars {
            return Ok(skipped_view(
                source_path,
                symbol,
                "tick",
                "tick aggregation is disabled",
            ));
        }
        return import_tick_file(
            source_path,
            output_directory,
            timezone,
            symbol,
            delimiter,
            columns,
        );
    }

    // HISTDATA and a few broker download tools emit headerless
    // `DATE,TIME,OPEN,HIGH,LOW,CLOSE,VOLUME` rows. The first row is exposed as
    // the CSV header by the reader above, so detect it before normal OHLC
    // classification and reopen the file with `has_headers(false)`.
    if looks_like_headerless_m1_row(&headers, timezone) {
        return import_headerless_m1_file(
            source_path,
            output_directory,
            timezone,
            symbol,
            delimiter,
        );
    }

    let has_ohlc = ["OPEN", "HIGH", "LOW", "CLOSE"]
        .into_iter()
        .all(|name| normalized.iter().any(|value| value == name));
    if !has_ohlc {
        return Ok(skipped_view(
            source_path,
            symbol,
            "unknown",
            "header is not an OHLC or IC Markets tick export",
        ));
    }

    let dataset = BarDataset::load_mt5(source_path, timezone).map_err(|error| error.to_string())?;
    let interval_ms = infer_median_interval_ms(&dataset.bars).unwrap_or_default();
    let base_stem = unique_import_stem(output_directory, symbol.as_deref().unwrap_or("MARKET"));
    if interval_ms <= 60_000 {
        let (m1_path, m1_metadata_path) = write_imported_bars(
            &dataset,
            output_directory,
            &base_stem,
            "M1",
            source_path,
            timezone,
            "ohlc",
        )?;
        let h1 = build_timeframe_from_m1(&dataset, 3_600_000, None)
            .map_err(|error| error.to_string())?;
        let (h1_path, h1_metadata_path) = write_imported_bars(
            &h1,
            output_directory,
            &base_stem,
            "H1",
            source_path,
            timezone,
            "ohlc_aggregated",
        )?;
        Ok(MarketFileImportView {
            source_path: display_path(source_path),
            symbol,
            kind: "ohlc_m1".into(),
            source_rows: dataset.source_rows,
            bars: dataset.bars.len(),
            m1_path: Some(display_path(&m1_path)),
            m1_metadata_path: Some(display_path(&m1_metadata_path)),
            h1_path: Some(display_path(&h1_path)),
            h1_metadata_path: Some(display_path(&h1_metadata_path)),
            quote_path: None,
            quote_metadata_path: None,
            price_basis: Some("source_ohlc".into()),
            status: "imported".into(),
            message: None,
        })
    } else if interval_ms == 3_600_000 {
        let (h1_path, h1_metadata_path) = write_imported_bars(
            &dataset,
            output_directory,
            &base_stem,
            "H1",
            source_path,
            timezone,
            "ohlc",
        )?;
        Ok(MarketFileImportView {
            source_path: display_path(source_path),
            symbol,
            kind: "ohlc_h1".into(),
            source_rows: dataset.source_rows,
            bars: dataset.bars.len(),
            m1_path: None,
            m1_metadata_path: None,
            h1_path: Some(display_path(&h1_path)),
            h1_metadata_path: Some(display_path(&h1_metadata_path)),
            quote_path: None,
            quote_metadata_path: None,
            price_basis: Some("source_ohlc".into()),
            status: "imported".into(),
            message: None,
        })
    } else {
        Ok(skipped_view(
            source_path,
            symbol,
            "ohlc",
            &format!("unsupported source interval: {interval_ms}ms"),
        ))
    }
}

fn looks_like_headerless_m1_row(record: &StringRecord, timezone: SourceTimezone) -> bool {
    if record.len() < 6 {
        return false;
    }
    let Some(date) = record.get(0) else {
        return false;
    };
    let Some(time) = record.get(1) else {
        return false;
    };
    if parse_source_timestamp(&format!("{date} {time}"), timezone).is_err() {
        return false;
    }
    (2..=5).all(|index| {
        record
            .get(index)
            .and_then(|value| value.parse::<f64>().ok())
            .is_some()
    })
}

fn import_headerless_m1_file(
    source_path: &Path,
    output_directory: &Path,
    timezone: SourceTimezone,
    symbol: Option<String>,
    delimiter: u8,
) -> Result<MarketFileImportView, String> {
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .trim(Trim::All)
        .flexible(true)
        .from_path(source_path)
        .map_err(|error| format!("cannot open headerless OHLC CSV: {error}"))?;
    let mut bars_by_timestamp = BTreeMap::new();
    let mut source_rows = 0;
    for (index, record) in reader.records().enumerate() {
        let record =
            record.map_err(|error| format!("OHLC row {} is invalid: {error}", index + 1))?;
        if record.len() < 6 {
            return Err(format!("OHLC row {} has fewer than six fields", index + 1));
        }
        let timestamp_text = format!(
            "{} {}",
            record.get(0).unwrap_or_default(),
            record.get(1).unwrap_or_default()
        );
        let timestamp_ms = parse_source_timestamp(&timestamp_text, timezone)
            .map_err(|reason| format!("OHLC row {} has invalid timestamp: {reason}", index + 1))?;
        let open = parse_market_number(&record, 2, index + 1, "OPEN")?;
        let high = parse_market_number(&record, 3, index + 1, "HIGH")?;
        let low = parse_market_number(&record, 4, index + 1, "LOW")?;
        let close = parse_market_number(&record, 5, index + 1, "CLOSE")?;
        let tick_volume = record
            .get(6)
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        bars_by_timestamp.insert(
            timestamp_ms,
            Bar {
                timestamp_ms,
                open,
                high,
                low,
                close,
                tick_volume,
                real_volume: 0,
                spread_points: None,
            },
        );
        source_rows += 1;
    }
    if bars_by_timestamp.is_empty() {
        return Err("headerless OHLC CSV contains no rows".into());
    }
    let bars: Vec<Bar> = bars_by_timestamp.into_values().collect();
    let bar_count = bars.len();
    let data_hash = quantforge_data::bar_content_hash(&bars);
    let dataset = BarDataset {
        bars,
        source_rows,
        duplicate_rows_removed: source_rows.saturating_sub(bar_count),
        input_was_sorted: true,
        delimiter: delimiter as char,
        source_timezone: timezone.to_string(),
        data_hash,
    };
    let base_stem = unique_import_stem(output_directory, symbol.as_deref().unwrap_or("MARKET"));
    let (m1_path, m1_metadata_path) = write_imported_bars(
        &dataset,
        output_directory,
        &base_stem,
        "M1",
        source_path,
        timezone,
        "ohlc_headerless",
    )?;
    let h1 =
        build_timeframe_from_m1(&dataset, 3_600_000, None).map_err(|error| error.to_string())?;
    let (h1_path, h1_metadata_path) = write_imported_bars(
        &h1,
        output_directory,
        &base_stem,
        "H1",
        source_path,
        timezone,
        "ohlc_headerless_aggregated",
    )?;
    Ok(MarketFileImportView {
        source_path: display_path(source_path),
        symbol,
        kind: "ohlc_m1_headerless".into(),
        source_rows,
        bars: dataset.bars.len(),
        m1_path: Some(display_path(&m1_path)),
        m1_metadata_path: Some(display_path(&m1_metadata_path)),
        h1_path: Some(display_path(&h1_path)),
        h1_metadata_path: Some(display_path(&h1_metadata_path)),
        quote_path: None,
        quote_metadata_path: None,
        price_basis: Some("source_ohlc".into()),
        status: "imported".into(),
        message: None,
    })
}

fn import_tick_file(
    source_path: &Path,
    output_directory: &Path,
    timezone: SourceTimezone,
    symbol: Option<String>,
    delimiter: u8,
    columns: TickColumns,
) -> Result<MarketFileImportView, String> {
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(Trim::All)
        .flexible(true)
        .from_path(source_path)
        .map_err(|error| format!("cannot open tick CSV: {error}"))?;
    let mut buckets: BTreeMap<i64, TickBar> = BTreeMap::new();
    let mut source_rows = 0;
    let mut input_was_sorted = true;
    let mut previous_timestamp = None;
    for (index, record) in reader.records().enumerate() {
        let record =
            record.map_err(|error| format!("tick row {} is invalid: {error}", index + 2))?;
        let timestamp_text = tick_timestamp(&record, columns, index + 2)?;
        let timestamp_ms = match parse_source_timestamp(&timestamp_text, timezone) {
            Ok(value) => value,
            Err(reason) if reason.contains("daylight-saving") || reason.contains("ambiguous") => {
                // ICMarkets/EST+7 follows New York DST under a +7 wall shift.
                // Quote dumps occasionally stamp the spring-forward gap; skip.
                continue;
            }
            Err(reason) => {
                return Err(format!(
                    "tick row {} has invalid timestamp {timestamp_text}: {reason}",
                    index + 2
                ));
            }
        };
        if previous_timestamp.is_some_and(|value| timestamp_ms < value) {
            input_was_sorted = false;
        }
        previous_timestamp = Some(timestamp_ms);
        let ask = parse_market_number(&record, columns.ask, index + 2, "ASK")?;
        let bid = parse_market_number(&record, columns.bid, index + 2, "BID")?;
        if !ask.is_finite() || !bid.is_finite() || ask < bid {
            return Err(format!(
                "tick row {} has invalid bid/ask geometry",
                index + 2
            ));
        }
        let volume = columns
            .volume
            .and_then(|column| record.get(column))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1);
        let bucket = timestamp_ms - timestamp_ms.rem_euclid(60_000);
        buckets
            .entry(bucket)
            .and_modify(|bar| bar.push(bid, ask, volume))
            .or_insert_with(|| TickBar::new(bid, ask, volume));
        source_rows += 1;
    }
    if buckets.is_empty() {
        return Err("tick CSV contains no usable rows".into());
    }
    let mut quote_bars = Vec::with_capacity(buckets.len());
    let bars: Vec<Bar> = buckets
        .into_iter()
        .map(|(timestamp_ms, bar)| {
            let (price_bar, quote_bar) = bar.finish(timestamp_ms);
            quote_bars.push(quote_bar);
            price_bar
        })
        .collect();
    let data_hash = quantforge_data::bar_content_hash(&bars);
    let dataset = BarDataset {
        bars,
        source_rows,
        duplicate_rows_removed: 0,
        input_was_sorted,
        delimiter: delimiter as char,
        source_timezone: timezone.to_string(),
        data_hash,
    };
    let base_stem = unique_import_stem(output_directory, symbol.as_deref().unwrap_or("MARKET"));
    let (m1_path, m1_metadata_path) = write_imported_bars(
        &dataset,
        output_directory,
        &base_stem,
        "M1",
        source_path,
        timezone,
        "tick_bid_ask",
    )?;
    let (quote_path, quote_metadata_path) = write_quote_bars(
        &quote_bars,
        source_rows,
        output_directory,
        &base_stem,
        source_path,
        timezone,
    )?;
    let h1 =
        build_timeframe_from_m1(&dataset, 3_600_000, None).map_err(|error| error.to_string())?;
    let (h1_path, h1_metadata_path) = write_imported_bars(
        &h1,
        output_directory,
        &base_stem,
        "H1",
        source_path,
        timezone,
        "tick_bid_ask_aggregated",
    )?;
    Ok(MarketFileImportView {
        source_path: display_path(source_path),
        symbol,
        kind: "tick".into(),
        source_rows,
        bars: dataset.bars.len(),
        m1_path: Some(display_path(&m1_path)),
        m1_metadata_path: Some(display_path(&m1_metadata_path)),
        h1_path: Some(display_path(&h1_path)),
        h1_metadata_path: Some(display_path(&h1_metadata_path)),
        quote_path: Some(display_path(&quote_path)),
        quote_metadata_path: Some(display_path(&quote_metadata_path)),
        price_basis: Some("bid".into()),
        status: "imported".into(),
        message: Some(
            "Bid M1 bars plus matching Ask/Bid quote sidecar; H1 is derived from the canonical bid M1 stream"
                .into(),
        ),
    })
}

#[derive(Debug, Clone, Copy)]
struct TickBar {
    bid_open: f64,
    bid_high: f64,
    bid_low: f64,
    bid_close: f64,
    ask_open: f64,
    ask_high: f64,
    ask_low: f64,
    ask_close: f64,
    volume: u64,
    tick_count: u64,
}
impl TickBar {
    fn new(bid: f64, ask: f64, volume: u64) -> Self {
        Self {
            bid_open: bid,
            bid_high: bid,
            bid_low: bid,
            bid_close: bid,
            ask_open: ask,
            ask_high: ask,
            ask_low: ask,
            ask_close: ask,
            volume,
            tick_count: 1,
        }
    }
    fn push(&mut self, bid: f64, ask: f64, volume: u64) {
        self.bid_high = self.bid_high.max(bid);
        self.bid_low = self.bid_low.min(bid);
        self.bid_close = bid;
        self.ask_high = self.ask_high.max(ask);
        self.ask_low = self.ask_low.min(ask);
        self.ask_close = ask;
        self.volume = self.volume.saturating_add(volume);
        self.tick_count = self.tick_count.saturating_add(1);
    }
    fn finish(self, timestamp_ms: i64) -> (Bar, QuoteBar) {
        let price = Bar {
            timestamp_ms,
            open: self.bid_open,
            high: self.bid_high,
            low: self.bid_low,
            close: self.bid_close,
            tick_volume: self.volume,
            real_volume: 0,
            spread_points: None,
        };
        let quote = QuoteBar {
            timestamp_ms,
            bid_open: self.bid_open,
            bid_high: self.bid_high,
            bid_low: self.bid_low,
            bid_close: self.bid_close,
            ask_open: self.ask_open,
            ask_high: self.ask_high,
            ask_low: self.ask_low,
            ask_close: self.ask_close,
            tick_count: self.tick_count,
        };
        (price, quote)
    }
}

fn write_quote_bars(
    bars: &[QuoteBar],
    source_rows: usize,
    output_directory: &Path,
    base_stem: &str,
    source_path: &Path,
    timezone: SourceTimezone,
) -> Result<(PathBuf, PathBuf), String> {
    let data_path = output_directory.join(format!("{base_stem}_M1.quotes.csv"));
    let metadata_path = output_directory.join(format!("{base_stem}_M1.quotes.metadata.csv"));
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(&data_path)
        .map_err(|error| format!("cannot write quote sidecar: {error}"))?;
    writer
        .write_record([
            "timestamp_ms",
            "bid_open",
            "bid_high",
            "bid_low",
            "bid_close",
            "ask_open",
            "ask_high",
            "ask_low",
            "ask_close",
            "tick_count",
        ])
        .map_err(|error| error.to_string())?;
    for bar in bars {
        writer
            .write_record([
                bar.timestamp_ms.to_string(),
                bar.bid_open.to_string(),
                bar.bid_high.to_string(),
                bar.bid_low.to_string(),
                bar.bid_close.to_string(),
                bar.ask_open.to_string(),
                bar.ask_high.to_string(),
                bar.ask_low.to_string(),
                bar.ask_close.to_string(),
                bar.tick_count.to_string(),
            ])
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    let hash = quote_bar_content_hash(bars);
    let symbol = base_stem.split('_').next().unwrap_or(base_stem);
    let mut metadata = csv::Writer::from_path(&metadata_path)
        .map_err(|error| format!("cannot write quote metadata: {error}"))?;
    metadata
        .write_record(["property", "value"])
        .map_err(|error| error.to_string())?;
    for (key, value) in [
        ("schema_version", "1".to_owned()),
        ("symbol", symbol.to_owned()),
        ("timeframe", "PERIOD_M1".to_owned()),
        ("bar_count", bars.len().to_string()),
        ("source_rows", source_rows.to_string()),
        ("broker_timezone", timezone.to_string()),
        ("price_basis", "bid_ask".to_owned()),
        ("import_kind", "tick_bid_ask_sidecar".to_owned()),
        ("source_file", display_path(source_path)),
        ("data_hash", hash.as_str().to_owned()),
    ] {
        metadata
            .write_record([key, value.as_str()])
            .map_err(|error| error.to_string())?;
    }
    metadata.flush().map_err(|error| error.to_string())?;
    Ok((data_path, metadata_path))
}

fn write_imported_bars(
    dataset: &BarDataset,
    output_directory: &Path,
    base_stem: &str,
    timeframe: &str,
    source_path: &Path,
    timezone: SourceTimezone,
    import_kind: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let data_path = output_directory.join(format!("{base_stem}_{timeframe}.csv"));
    let metadata_path = output_directory.join(format!("{base_stem}_{timeframe}.metadata.csv"));
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(&data_path)
        .map_err(|error| format!("cannot write imported data: {error}"))?;
    writer
        .write_record([
            "<DATE>",
            "<TIME>",
            "<OPEN>",
            "<HIGH>",
            "<LOW>",
            "<CLOSE>",
            "<TICKVOL>",
            "<VOL>",
            "<SPREAD>",
        ])
        .map_err(|error| error.to_string())?;
    for bar in &dataset.bars {
        let stamp = timezone
            .format_source_timestamp(bar.timestamp_ms)
            .ok_or_else(|| "cannot format imported timestamp".to_owned())?;
        let (date, time) = stamp
            .split_once(' ')
            .ok_or_else(|| "formatted timestamp is invalid".to_owned())?;
        let row = [
            date.to_owned(),
            time.to_owned(),
            bar.open.to_string(),
            bar.high.to_string(),
            bar.low.to_string(),
            bar.close.to_string(),
            bar.tick_volume.to_string(),
            bar.real_volume.to_string(),
            bar.spread_points.unwrap_or(0).to_string(),
        ];
        writer
            .write_record(row)
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    let mut metadata = csv::Writer::from_path(&metadata_path)
        .map_err(|error| format!("cannot write import metadata: {error}"))?;
    let symbol = base_stem.split('_').next().unwrap_or(base_stem);
    let rows = [
        ("schema_version", "1".to_owned()),
        ("symbol", symbol.to_owned()),
        ("timeframe", format!("PERIOD_{timeframe}")),
        ("bar_count", dataset.bars.len().to_string()),
        ("source_rows", dataset.source_rows.to_string()),
        ("broker_timezone", timezone.to_string()),
        (
            "price_basis",
            if import_kind.contains("bid_ask") {
                "bid".to_owned()
            } else {
                "source_ohlc".to_owned()
            },
        ),
        ("import_kind", import_kind.to_owned()),
        ("source_file", display_path(source_path)),
        ("data_hash", dataset.data_hash.as_str().to_owned()),
    ];
    metadata
        .write_record(["property", "value"])
        .map_err(|error| error.to_string())?;
    for (key, value) in rows {
        metadata
            .write_record([key, value.as_str()])
            .map_err(|error| error.to_string())?;
    }
    metadata.flush().map_err(|error| error.to_string())?;
    Ok((data_path, metadata_path))
}

fn tick_timestamp(
    record: &StringRecord,
    columns: TickColumns,
    row: usize,
) -> Result<String, String> {
    if let Some(index) = columns.timestamp {
        return record
            .get(index)
            .map(str::to_owned)
            .ok_or_else(|| format!("tick row {row} is short"));
    }
    let date = record
        .get(
            columns
                .date
                .ok_or_else(|| format!("tick row {row} has no date"))?,
        )
        .ok_or_else(|| format!("tick row {row} is short"))?;
    let time = record
        .get(
            columns
                .time
                .ok_or_else(|| format!("tick row {row} has no time"))?,
        )
        .ok_or_else(|| format!("tick row {row} is short"))?;
    Ok(format!("{date} {time}"))
}

fn parse_market_number(
    record: &StringRecord,
    column: usize,
    row: usize,
    field: &str,
) -> Result<f64, String> {
    let raw = record
        .get(column)
        .ok_or_else(|| format!("tick row {row} is short"))?;
    raw.parse::<f64>()
        .map_err(|_| format!("tick row {row} has invalid {field}: {raw}"))
}

fn skipped_view(
    source_path: &Path,
    symbol: Option<String>,
    kind: &str,
    message: &str,
) -> MarketFileImportView {
    MarketFileImportView {
        source_path: display_path(source_path),
        symbol,
        kind: kind.into(),
        source_rows: 0,
        bars: 0,
        m1_path: None,
        m1_metadata_path: None,
        h1_path: None,
        h1_metadata_path: None,
        quote_path: None,
        quote_metadata_path: None,
        price_basis: None,
        status: "skipped".into(),
        message: Some(message.into()),
    }
}

fn detect_market_delimiter(bytes: &[u8]) -> Result<u8, String> {
    let line = bytes
        .split(|byte| *byte == b'\n' || *byte == b'\r')
        .find(|line| !line.is_empty())
        .ok_or_else(|| "CSV is empty".to_owned())?;
    [b',', b'\t', b';']
        .into_iter()
        .max_by_key(|delimiter| line.iter().filter(|byte| **byte == *delimiter).count())
        .filter(|delimiter| line.iter().filter(|byte| **byte == *delimiter).count() > 0)
        .ok_or_else(|| "could not detect CSV delimiter".to_owned())
}

fn normalize_market_header(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| character == '<' || character == '>')
        .replace([' ', '-'], "_")
        .to_ascii_uppercase()
}

fn symbol_from_headers(headers: &[String]) -> Option<String> {
    let _ = headers;
    None
}

fn symbol_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy().to_ascii_uppercase();
    for token in stem.split('_').rev() {
        let token = token.trim_matches(|value: char| !value.is_ascii_alphanumeric());
        if (token.len() == 6 && token.chars().all(|value| value.is_ascii_alphabetic()))
            || matches!(
                token,
                "NAS100" | "US100" | "US500" | "XAUUSD" | "BTCUSD" | "XTIUSD" | "US30" | "DE40"
            )
        {
            return Some(token.to_owned());
        }
    }
    // Fallback: SYMBOL_TickData / SYMBOL_M1 style stems.
    let first = stem.split('_').next()?.trim();
    if first.len() >= 3
        && first.len() <= 8
        && first.chars().all(|value| value.is_ascii_alphanumeric())
    {
        return Some(first.to_owned());
    }
    None
}

fn unique_import_stem(output_directory: &Path, symbol: &str) -> String {
    let clean: String = symbol
        .chars()
        .filter(|value| value.is_ascii_alphanumeric())
        .collect();
    for index in 1..10_000 {
        let suffix = if index == 1 {
            String::new()
        } else {
            format!("_{index}")
        };
        if !output_directory
            .join(format!("{clean}{suffix}_M1.csv"))
            .exists()
            && !output_directory
                .join(format!("{clean}{suffix}_H1.csv"))
                .exists()
        {
            return format!("{clean}{suffix}");
        }
    }
    format!("{clean}_import")
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

/// Load a bid/ask quote sidecar in the clock used by its bound M1 pack.
/// Imported packs are normalized to UTC; the capture EA writes raw MT5
/// server-wall timestamps and therefore needs the metadata timezone mapping.
pub(crate) fn load_quote_sidecar(
    path: &Path,
    metadata: Option<&Mt5ExportMetadata>,
) -> Result<QuoteBarDataset, String> {
    let tester_ticks = metadata.is_some_and(|metadata| {
        metadata
            .properties
            .get("execution_model")
            .is_some_and(|value| value.contains("TESTER_TICKS"))
    });
    if tester_ticks {
        let timezone = metadata
            .ok_or_else(|| "quote sidecar requires M1 metadata for timezone conversion".to_owned())?
            .source_timezone()
            .map_err(|error| error.to_string())?;
        QuoteBarDataset::load_csv_server_epoch(path, timezone).map_err(|error| error.to_string())
    } else {
        QuoteBarDataset::load_csv(path).map_err(|error| error.to_string())
    }
}

/// SQX-style: build decision bars from M1. Optional exported H1 supplies the open grid
/// and interval; otherwise H1 (3_600_000ms) is assumed from M1 timestamps.
pub(crate) fn build_decision_from_m1(
    m1: &BarDataset,
    exported_decision: Option<&BarDataset>,
) -> Result<BarDataset, String> {
    let interval_ms = exported_decision
        .and_then(|dataset| infer_median_interval_ms(&dataset.bars))
        .unwrap_or(3_600_000);
    let grid = exported_decision.map(|dataset| {
        dataset
            .bars
            .iter()
            .map(|bar| bar.timestamp_ms)
            .collect::<Vec<_>>()
    });
    build_timeframe_from_m1(m1, interval_ms, grid.as_deref()).map_err(|error| error.to_string())
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

    #[test]
    fn imports_ic_markets_tick_csv_to_m1_and_h1_with_metadata() {
        let directory = tempfile::tempdir().expect("temp directory");
        let source = directory.path().join("EURUSD_TickData.csv");
        fs::write(
            &source,
            concat!(
                "Time,Ask,Bid,Volume\n",
                "2020.01.02 00:00:01,1.1002,1.1000,1\n",
                "2020.01.02 00:00:30,1.1004,1.1001,2\n",
                "2020.01.02 00:01:02,1.1001,1.0999,1\n",
            ),
        )
        .unwrap();
        let output = directory.path().join("out");
        let report = import_market_folder_sync(&MarketFolderImportRequest {
            source_directory: directory.path().display().to_string(),
            output_directory: Some(output.display().to_string()),
            source_timezone: "ICMarkets/EST+7".into(),
            aggregate_ticks_to_bars: true,
        })
        .expect("tick import should succeed");
        assert_eq!(report.imported_count, 1);
        let file = report
            .files
            .iter()
            .find(|file| file.status == "imported")
            .unwrap();
        assert_eq!(file.symbol.as_deref(), Some("EURUSD"));
        assert_eq!(file.bars, 2);
        let m1 = BarDataset::load_mt5(
            file.m1_path.as_ref().unwrap(),
            "ICMarkets/EST+7".parse().unwrap(),
        )
        .unwrap();
        assert_eq!(m1.bars.len(), 2);
        assert!(file.h1_path.is_some());
    }

    #[test]
    fn imports_headerless_histdata_m1_csv() {
        let directory = tempfile::tempdir().expect("temp directory");
        let source = directory.path().join("DAT_MT_EURUSD_M1_2000.csv");
        fs::write(
            &source,
            concat!(
                "2000.05.30,17:27,0.930200,0.930200,0.930200,0.930200,0\n",
                "2000.05.30,17:28,0.930200,0.930300,0.930100,0.930250,1\n",
            ),
        )
        .unwrap();
        let output = directory.path().join("out");
        let report = import_market_folder_sync(&MarketFolderImportRequest {
            source_directory: directory.path().display().to_string(),
            output_directory: Some(output.display().to_string()),
            source_timezone: "Etc/UTC".into(),
            aggregate_ticks_to_bars: true,
        })
        .expect("headerless import should succeed");
        assert_eq!(report.imported_count, 1);
        assert_eq!(report.files[0].kind, "ohlc_m1_headerless");
        assert_eq!(report.files[0].bars, 2);
    }
}
