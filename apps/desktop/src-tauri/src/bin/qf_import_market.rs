//! Import IC Markets `*_TickData.csv` folders into QuantForge OHLC packs.

use quantforge_desktop_lib::data_lab::{import_market_folder_sync, MarketFolderImportRequest};
use std::env;

fn main() {
    let mut args = env::args().skip(1);
    let source = args
        .next()
        .expect("usage: qf-import-market <source_dir> <output_dir> [timezone]");
    let output = args
        .next()
        .expect("usage: qf-import-market <source_dir> <output_dir> [timezone]");
    let timezone = args
        .next()
        .unwrap_or_else(|| "ICMarkets/EST+7".to_owned());
    let report = import_market_folder_sync(&MarketFolderImportRequest {
        source_directory: source,
        output_directory: Some(output),
        source_timezone: timezone,
        aggregate_ticks_to_bars: true,
    })
    .unwrap_or_else(|error| {
        eprintln!("import failed: {error}");
        std::process::exit(1);
    });
    println!(
        "imported {} · skipped {} · out {}",
        report.imported_count, report.skipped_count, report.output_directory
    );
    for file in report.files {
        println!(
            "  [{}] {} → {:?} ({})",
            file.status,
            file.symbol.unwrap_or_else(|| "?".into()),
            file.m1_path,
            file.message.unwrap_or_default()
        );
    }
}
