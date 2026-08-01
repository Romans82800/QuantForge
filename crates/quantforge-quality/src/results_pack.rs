//! SQX Saver-style results pack: HTML report + trades CSV + metrics JSON.
//!
//! Pure-Rust “print PDF” is a minimal PDF 1.4 wrapper around the same KPI table
//! (no external renderer). Prefer the HTML for interactive review; PDF is the
//! archive/hand-off artifact.

use quantforge_eval::{BacktestMetrics, ScoutResult, Trade};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

pub const RESULTS_PACK_PROTOCOL: &str = "results-pack-v1";

#[derive(Debug, Clone)]
pub struct ResultsPackPaths {
    pub directory: PathBuf,
    pub html: PathBuf,
    pub trades_csv: PathBuf,
    pub metrics_json: PathBuf,
    pub pdf: PathBuf,
}

/// Write a self-contained results directory (HTML + CSV + metrics + PDF).
pub fn write_results_pack(
    out_dir: impl AsRef<Path>,
    title: &str,
    strategy_id: &str,
    metrics: &BacktestMetrics,
    trades: &[Trade],
    extras: &[(String, String)],
) -> Result<ResultsPackPaths, String> {
    let directory = out_dir.as_ref().to_path_buf();
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let html = directory.join("results.html");
    let trades_csv = directory.join("trades.csv");
    let metrics_json = directory.join("metrics.json");
    let pdf = directory.join("results.pdf");

    let html_body = crate::results_html::render_results_html(title, strategy_id, metrics, trades, extras);
    fs::write(&html, html_body).map_err(|e| e.to_string())?;
    fs::write(&trades_csv, render_trades_csv(trades)).map_err(|e| e.to_string())?;
    let metrics_doc = json!({
        "protocol": RESULTS_PACK_PROTOCOL,
        "title": title,
        "strategy_id": strategy_id,
        "metrics": metrics,
        "extras": extras.iter().map(|(k,v)| json!({"key": k, "value": v})).collect::<Vec<_>>(),
        "trade_count": trades.len(),
    });
    fs::write(
        &metrics_json,
        serde_json::to_string_pretty(&metrics_doc).map_err(|e| e.to_string())? + "\n",
    )
    .map_err(|e| e.to_string())?;
    fs::write(&pdf, render_results_pdf(title, strategy_id, metrics, trades)?)
        .map_err(|e| e.to_string())?;

    Ok(ResultsPackPaths {
        directory,
        html,
        trades_csv,
        metrics_json,
        pdf,
    })
}

pub fn write_results_pack_from_json(
    out_dir: impl AsRef<Path>,
    title: &str,
    value: &Value,
) -> Result<ResultsPackPaths, String> {
    let strategy_id = value
        .pointer("/strategy/id")
        .or_else(|| value.get("strategy_id"))
        .or_else(|| value.pointer("/binding/strategy_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("strategy");
    let result = value
        .get("result")
        .cloned()
        .or_else(|| value.get("scout").cloned())
        .unwrap_or_else(|| value.clone());
    let metrics: BacktestMetrics = serde_json::from_value(
        result
            .get("metrics")
            .cloned()
            .or_else(|| value.get("metrics").cloned())
            .ok_or_else(|| "JSON missing metrics".to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let trades: Vec<Trade> = serde_json::from_value(
        result
            .get("trades")
            .cloned()
            .or_else(|| value.get("trades").cloned())
            .unwrap_or(Value::Array(vec![])),
    )
    .map_err(|e| e.to_string())?;
    write_results_pack(out_dir, title, strategy_id, &metrics, &trades, &[])
}

pub fn write_results_pack_from_scout(
    out_dir: impl AsRef<Path>,
    title: &str,
    strategy_id: &str,
    result: &ScoutResult,
    extras: &[(String, String)],
) -> Result<ResultsPackPaths, String> {
    write_results_pack(
        out_dir,
        title,
        strategy_id,
        &result.metrics,
        &result.trades,
        extras,
    )
}

pub fn render_trades_csv(trades: &[Trade]) -> String {
    let mut out = String::from(
        "index,side,entry_timestamp_ms,exit_timestamp_ms,entry_price,exit_price,volume,gross_profit,commission,swap,net_profit,bars_held,exit_reason\n",
    );
    for (index, trade) in trades.iter().enumerate() {
        let _ = writeln!(
            out,
            "{},{},{},{},{:.8},{:.8},{:.4},{:.4},{:.4},{:.4},{:.4},{},{:?}",
            index + 1,
            match trade.side {
                quantforge_eval::PositionSide::Long => "long",
                quantforge_eval::PositionSide::Short => "short",
            },
            trade.entry_timestamp_ms,
            trade.exit_timestamp_ms,
            trade.entry_price,
            trade.exit_price,
            trade.volume,
            trade.gross_profit,
            trade.commission,
            trade.swap,
            trade.net_profit,
            trade.bars_held,
            trade.exit_reason,
        );
    }
    out
}

/// Minimal PDF 1.4 with Helvetica text lines (SaverPDF stand-in).
pub fn render_results_pdf(
    title: &str,
    strategy_id: &str,
    metrics: &BacktestMetrics,
    trades: &[Trade],
) -> Result<Vec<u8>, String> {
    let mut lines: Vec<String> = vec![
        title.to_string(),
        format!("Strategy: {strategy_id}"),
        format!("Protocol: {RESULTS_PACK_PROTOCOL}"),
        String::new(),
        format!("Trades: {}", metrics.trade_count),
        format!("Net profit: {:.2}", metrics.net_profit),
        format!("Return %: {:.2}", metrics.return_percent),
        format!(
            "Profit factor: {}",
            metrics
                .profit_factor
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "inf".into())
        ),
        format!("Max DD %: {:.2}", metrics.max_drawdown_percent),
        format!("Win rate: {:.1}%", metrics.win_rate * 100.0),
        format!("Expectancy: {:.2}", metrics.expectancy),
        String::new(),
        "Trade blotter (first 40):".into(),
    ];
    for (index, trade) in trades.iter().take(40).enumerate() {
        lines.push(format!(
            "{:>3} {} net={:.2} bars={} {:?}",
            index + 1,
            match trade.side {
                quantforge_eval::PositionSide::Long => "L",
                quantforge_eval::PositionSide::Short => "S",
            },
            trade.net_profit,
            trade.bars_held,
            trade.exit_reason,
        ));
    }
    if trades.len() > 40 {
        lines.push(format!("… {} more trades in trades.csv", trades.len() - 40));
    }
    build_simple_pdf(&lines)
}

fn pdf_escape(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '\\' | '(' | ')' => format!("\\{ch}"),
            c if (c as u32) < 128 => c.to_string(),
            _ => '?'.to_string(),
        })
        .collect()
}

fn build_simple_pdf(lines: &[String]) -> Result<Vec<u8>, String> {
    let mut content = String::from("BT /F1 11 Tf 50 780 Td 14 TL\n");
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            content.push_str("T*\n");
        }
        content.push_str(&format!("({}) Tj\n", pdf_escape(line)));
    }
    content.push_str("ET\n");
    let content_bytes = content.into_bytes();

    let objects: Vec<Vec<u8>> = vec![
        b"1 0 obj<< /Type /Catalog /Pages 2 0 R >>endobj\n".to_vec(),
        b"2 0 obj<< /Type /Pages /Kids [3 0 R] /Count 1 >>endobj\n".to_vec(),
        b"3 0 obj<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>endobj\n".to_vec(),
        {
            let mut obj = format!("4 0 obj<< /Length {} >>stream\n", content_bytes.len()).into_bytes();
            obj.extend_from_slice(&content_bytes);
            obj.extend_from_slice(b"endstream\nendobj\n");
            obj
        },
        b"5 0 obj<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>endobj\n".to_vec(),
    ];

    let mut pdf = Vec::new();
    pdf.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = vec![0usize];
    for obj in &objects {
        offsets.push(pdf.len());
        pdf.extend_from_slice(obj);
    }
    let xref_at = pdf.len();
    let _ = writeln!(pdf, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1);
    for offset in offsets.iter().skip(1) {
        let _ = writeln!(pdf, "{offset:010} 00000 n ");
    }
    let _ = write!(
        pdf,
        "trailer<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        objects.len() + 1,
        xref_at
    );
    Ok(pdf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quantforge_eval::{ExitReason, PositionSide};

    fn sample_metrics() -> BacktestMetrics {
        BacktestMetrics {
            initial_balance: 10_000.0,
            ending_balance: 10_125.0,
            net_profit: 125.0,
            return_percent: 1.25,
            trade_count: 1,
            winning_trades: 1,
            losing_trades: 0,
            win_rate: 1.0,
            profit_factor: Some(1.8),
            max_drawdown: 40.0,
            max_drawdown_percent: 0.4,
            sharpe_ratio: None,
            expectancy: 125.0,
        }
    }

    fn sample_trade() -> Trade {
        Trade {
            side: PositionSide::Long,
            entry_timestamp_ms: 1,
            exit_timestamp_ms: 2,
            entry_price: 1.1,
            exit_price: 1.2,
            volume: 1.0,
            initial_stop_loss: 1.0,
            initial_take_profit: 1.3,
            gross_profit: 10.0,
            commission: 0.0,
            swap: 0.0,
            net_profit: 10.0,
            bars_held: 1,
            exit_reason: ExitReason::TakeProfit,
        }
    }

    #[test]
    fn pdf_starts_with_header() {
        let pdf = render_results_pdf("Demo", "id", &sample_metrics(), &[sample_trade()]).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.windows(5).any(|w| w == b"%%EOF"));
    }

    #[test]
    fn pack_writes_four_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let paths = write_results_pack(
            dir.path().join("pack"),
            "Demo",
            "id",
            &sample_metrics(),
            &[sample_trade()],
            &[("Note".into(), "ok".into())],
        )
        .unwrap();
        assert!(paths.html.is_file());
        assert!(paths.trades_csv.is_file());
        assert!(paths.metrics_json.is_file());
        assert!(paths.pdf.is_file());
        let csv = fs::read_to_string(paths.trades_csv).unwrap();
        assert!(csv.contains("net_profit"));
        assert!(csv.contains("TakeProfit"));
    }
}
