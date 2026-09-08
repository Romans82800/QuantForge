//! Date-aligned, Development-only portfolio research across saved Databanks.
use crate::databank::{DesktopState, EvolveArtifact, install_live_databank_artifact};
use quantforge_core::{ContentHash, stable_json_hash};
use quantforge_portfolio::{PortfolioCandidate, PortfolioConfig, PortfolioReport, pack_portfolio};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    databank_paths: Vec<String>,
    output_path: String,
    config: PortfolioConfig,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchPortfolio {
    schema_version: u32,
    output_path: String,
    source_hashes: BTreeMap<String, ContentHash>,
    sample: String,
    day_timestamps_ms: Vec<i64>,
    candidates: Vec<CandidateRecord>,
    clusters: Vec<Vec<String>>,
    maximum_simultaneous_positions: usize,
    currency_allocation_participation: BTreeMap<String, f64>,
    report: PortfolioReport,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CandidateRecord {
    id: String,
    fingerprint: String,
    strategy_id: String,
    symbol: String,
    source: String,
    development_expectancy_r: f64,
}

pub(crate) struct DailyReplay {
    pub fingerprint: String,
    pub strategy_id: String,
    pub symbol: String,
    pub cohort: String,
    pub expectancy_r: f64,
    pub start_day: i64,
    pub end_day: i64,
    pub daily_returns: BTreeMap<i64, f64>,
    pub trade_windows: Vec<(i64, i64)>,
}

#[tauri::command]
pub async fn build_research_portfolio(request: Request) -> Result<ResearchPortfolio, String> {
    tauri::async_runtime::spawn_blocking(move || build(request)).await.map_err(|e| e.to_string())?
}

fn build(request: Request) -> Result<ResearchPortfolio, String> {
    request.config.validate().map_err(|e| e.to_string())?;
    if request.databank_paths.is_empty() { return Err("Choose at least one Databank.".into()); }
    if Path::new(&request.output_path).exists() { return Err("Choose a new report filename.".into()); }
    let mut sources = BTreeMap::new();
    let mut replays = Vec::new();
    for path in &request.databank_paths {
        let canonical = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        let key = canonical.to_string_lossy().to_string();
        if sources.contains_key(&key) { return Err("The same Databank was selected twice.".into()); }
        let bytes = std::fs::read(&canonical).map_err(|e| e.to_string())?;
        let state = DesktopState::default();
        let artifact: EvolveArtifact = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        install_live_databank_artifact(artifact, canonical.clone(), &state).map_err(|e| e.to_string())?;
        let rows = crate::databank::replay_development_daily(&state)?;
        if rows.is_empty() { return Err(format!("{path} has no Databank strategies.")); }
        sources.insert(key.clone(), ContentHash::sha256(&bytes));
        replays.extend(rows.into_iter().map(|row| (key.clone(), row)));
    }
    let start = replays.iter().map(|(_, r)| r.start_day).max().unwrap();
    let end = replays.iter().map(|(_, r)| r.end_day).min().unwrap();
    if end - start < 30 { return Err("Databanks need at least 30 overlapping Development days.".into()); }
    let days: Vec<i64> = (start..=end).collect();
    let mut records = Vec::new();
    let mut candidates = Vec::new();
    let mut identities = BTreeSet::new();
    let mut trade_windows = BTreeMap::new();
    for (source, row) in replays {
        // The same trading rules on two markets are separate candidates; exact
        // duplicates across banks of one market are refused, never double weighted.
        let id = stable_json_hash(&(&row.symbol, &row.fingerprint)).map_err(|e| e.to_string())?;
        if !identities.insert(id.clone()) { return Err(format!("Duplicate strategy {} on {} across Databanks.", row.strategy_id, row.symbol)); }
        trade_windows.insert(id.clone(), row.trade_windows);
        let returns: Vec<_> = days.iter().map(|day| row.daily_returns.get(day).copied().unwrap_or(0.0)).collect();
        candidates.push(PortfolioCandidate {
            strategy_fingerprint: id.clone(), symbol: row.symbol.clone(), cohort: row.cohort,
            initial_balance: 1.0, return_percent: returns.iter().sum::<f64>() * 100.0,
            maximum_drawdown_percent: drawdown(&returns), equity_signature: returns,
        });
        records.push(CandidateRecord { id: id.to_string(), fingerprint: row.fingerprint,
            strategy_id: row.strategy_id, symbol: row.symbol, source,
            development_expectancy_r: row.expectancy_r });
    }
    let clusters = correlation_clusters(&candidates, request.config.maximum_pairwise_correlation);
    let data_hash = stable_json_hash(&(&sources, &days)).map_err(|e| e.to_string())?;
    let bindings = stable_json_hash(&records).map_err(|e| e.to_string())?;
    let report = pack_portfolio(&candidates, data_hash, bindings, request.config).map_err(|e| e.to_string())?;
    let windows: Vec<_> = report.selected.iter().flat_map(|allocation| trade_windows[&allocation.strategy_fingerprint].iter().copied()).collect();
    let maximum_simultaneous_positions = simultaneous_positions(&windows, start * 86_400_000, (end + 1) * 86_400_000);
    let mut currency_allocation_participation = BTreeMap::new();
    const CURRENCIES: &[&str] = &["USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD"];
    for allocation in &report.selected {
        if allocation.symbol.len() == 6 && allocation.symbol.is_ascii() {
            let (base, quote) = allocation.symbol.split_at(3);
            if CURRENCIES.contains(&base) && CURRENCIES.contains(&quote) {
                for currency in [base, quote] { *currency_allocation_participation.entry(currency.into()).or_insert(0.0) += allocation.weight; }
            }
        }
    }
    let result = ResearchPortfolio { schema_version: 1, output_path: request.output_path.clone(),
        source_hashes: sources, sample: "Overlapping Development only; UTC daily marked-to-market returns; equal capital weights; drawdown measured at daily closes".into(),
        day_timestamps_ms: days.iter().map(|day| day * 86_400_000).collect(), candidates: records, clusters, maximum_simultaneous_positions, currency_allocation_participation, report };
    quantforge_storage::write_json_new(&request.output_path, &result).map_err(|e| e.to_string())?;
    Ok(result)
}

fn drawdown(returns: &[f64]) -> f64 {
    let (mut equity, mut peak, mut dd) = (1.0_f64, 1.0_f64, 0.0_f64);
    for change in returns { equity += change; peak = peak.max(equity); dd = dd.max((peak - equity) / peak * 100.0); }
    dd
}

fn simultaneous_positions(windows: &[(i64, i64)], start: i64, end: i64) -> usize {
    let mut events = Vec::new();
    for &(entry, exit) in windows {
        let (entry, exit) = (entry.max(start), exit.min(end));
        if entry < exit { events.push((entry, 1_i64)); events.push((exit, -1_i64)); }
    }
    events.sort_unstable(); // closes precede opens at equal timestamps
    let (mut active, mut maximum) = (0_i64, 0_i64);
    for (_, change) in events { active += change; maximum = maximum.max(active); }
    maximum as usize
}

fn corr(a: &[f64], b: &[f64]) -> f64 {
    let ma = a.iter().sum::<f64>() / a.len() as f64;
    let mb = b.iter().sum::<f64>() / b.len() as f64;
    let (mut ab, mut aa, mut bb) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (a, b) in a.iter().zip(b) { let (a, b) = (a-ma, b-mb); ab += a*b; aa += a*a; bb += b*b; }
    if aa * bb <= f64::EPSILON { 0.0 } else { ab / (aa * bb).sqrt() }
}

fn correlation_clusters(rows: &[PortfolioCandidate], threshold: f64) -> Vec<Vec<String>> {
    let mut remaining: BTreeSet<_> = (0..rows.len()).collect();
    let mut groups = Vec::new();
    while let Some(first) = remaining.pop_first() {
        let mut group = vec![first];
        let mut cursor = 0;
        while cursor < group.len() {
            let found: Vec<_> = remaining.iter().copied().filter(|&other|
                corr(&rows[group[cursor]].equity_signature, &rows[other].equity_signature) > threshold).collect();
            for index in found { remaining.remove(&index); group.push(index); }
            cursor += 1;
        }
        groups.push(group.iter().map(|&i| rows[i].strategy_fingerprint.to_string()).collect());
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correlation_distinguishes_clones_from_opposite_returns() {
        let a = [0.01, -0.02, 0.03, 0.01];
        assert!((corr(&a, &a) - 1.0).abs() < 1e-9);
        assert!((corr(&a, &a.map(|x| -x)) + 1.0).abs() < 1e-9);
        assert_eq!(corr(&[0.0; 4], &a), 0.0);
        assert!((drawdown(&[0.1, -0.11]) - 10.0).abs() < 1e-9);
    }
    #[test]
    fn position_overlap_clips_to_shared_period_and_handles_touching_trades() {
        assert_eq!(simultaneous_positions(&[(0, 10), (10, 20)], 0, 20), 1);
        assert_eq!(simultaneous_positions(&[(0, 12), (10, 20), (40, 80)], 5, 30), 2);
    }
}
