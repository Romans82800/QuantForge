import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { chooseDatabank, chooseOutputJson } from "./api";

interface Report {
  outputPath: string;
  sample: string;
  dayTimestampsMs: number[];
  clusters: string[][];
  maximumSimultaneousPositions: number;
  currencyAllocationParticipation: Record<string, number>;
  candidates: { id: string; strategyId: string; symbol: string; developmentExpectancyR: number }[];
  report: {
    selected: { strategy_fingerprint: string; symbol: string; weight: number }[];
    expected_return_percent: number;
    path_maximum_drawdown_percent: number;
    maximum_observed_pairwise_correlation: number;
    portfolio_return_path: number[];
    symbol_exposures: Record<string, number>;
    stress: { p05_return_percent: number; p95_maximum_drawdown_percent: number };
  };
}

export function ResearchPortfolio({ onError }: { onError: (message: string | null) => void }) {
  const [paths, setPaths] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Report | null>(null);
  const [correlation, setCorrelation] = useState(0.5);
  const [weight, setWeight] = useState(0.25);
  const [symbolCap, setSymbolCap] = useState(0.5);
  const [familyCap, setFamilyCap] = useState(0.75);
  const [count, setCount] = useState(12);
  async function add() {
    try { const path = await chooseDatabank(); if (path) setPaths((current) => current.includes(path) ? current : [...current, path]); }
    catch (e) { onError(String(e)); }
  }
  async function build() {
    try {
      const outputPath = await chooseOutputJson("Save portfolio report", "development-portfolio.json");
      if (!outputPath) return;
      setBusy(true); setResult(null); onError(null);
      setResult(await invoke<Report>("build_research_portfolio", { request: {
        databankPaths: paths, outputPath, config: {
          maximum_pairwise_correlation: correlation, maximum_weight_per_strategy: weight,
          maximum_symbol_exposure: symbolCap, maximum_cohort_exposure: familyCap,
          maximum_strategies: count, seed: 42,
        },
      }}));
    } catch (e) { onError(String(e)); }
    finally { setBusy(false); }
  }
  const report = result?.report;
  const curve = report?.portfolio_return_path ?? [];
  const low = Math.min(0, ...curve), high = Math.max(0, ...curve);
  const points = curve.map((value, i) => `${20 + i / Math.max(1, curve.length - 1) * 960},${240 - (value-low) / Math.max(high-low, 1e-9) * 220}`).join(" ");
  return <div className="wide-tool-content">
    <section className="panel">
      <div className="panel-heading"><div><p className="eyebrow">Multi-asset portfolio</p><h2>Combine your Databanks</h2></div><button className="secondary" disabled={busy} onClick={() => void add()}>Add Databank</button></div>
      <p className="immutable-note">All saved Databank strategies are replayed sequentially on their bound Development data. Correlation uses the shared calendar period. Large banks can take time; results are saved as a new report.</p>
      {paths.map((path) => <div className="campaign-history-row" key={path}><div><strong>{path.split(/[\\/]/).pop()}</strong><small>{path}</small></div><button className="secondary" disabled={busy} onClick={() => setPaths(paths.filter((p) => p !== path))}>Remove</button></div>)}
      <div className="numeric-grid">
        <label className="number-field"><span>Correlation ceiling</span><input type="number" min={0} max={1} step={0.05} value={correlation} onChange={(e) => setCorrelation(Number(e.target.value))} disabled={busy} /></label>
        <label className="number-field"><span>Maximum strategy weight</span><input type="number" min={0.01} max={1} step={0.05} value={weight} onChange={(e) => setWeight(Number(e.target.value))} disabled={busy} /></label>
        <label className="number-field"><span>Maximum weight per asset</span><input type="number" min={0.01} max={1} step={0.05} value={symbolCap} onChange={(e) => setSymbolCap(Number(e.target.value))} disabled={busy} /></label>
        <label className="number-field"><span>Maximum behaviour-group weight</span><input type="number" min={0.01} max={1} step={0.05} value={familyCap} onChange={(e) => setFamilyCap(Number(e.target.value))} disabled={busy} /></label>
        <label className="number-field"><span>Maximum strategies</span><input type="number" min={1} step={1} value={count} onChange={(e) => setCount(Number(e.target.value))} disabled={busy} /></label>
      </div>
      <div className="form-footer"><p>Equal capital allocation · Development only · fixed seed 42</p><button className="primary" disabled={busy || !paths.length} onClick={() => void build()}>{busy ? "Replaying and combining…" : "Build portfolio report"}</button></div>
    </section>
    {result && report && <>
      <section className="panel">
        <div className="panel-heading"><div><p className="eyebrow">Combined Development performance</p><h2>{report.selected.length} strategies across {Object.keys(report.symbol_exposures).length} assets</h2></div></div>
        <p className="immutable-note">{result.sample}</p>
        <p className="immutable-note">{new Date(result.dayTimestampsMs[0]).toLocaleDateString()} – {new Date(result.dayTimestampsMs.at(-1)!).toLocaleDateString()} · maximum {result.maximumSimultaneousPositions} simultaneous positions</p>
        <svg viewBox="0 0 1000 260" role="img" aria-label="Combined Development portfolio return" style={{ width: "100%" }}><polyline points={points} fill="none" stroke="#35d4bd" strokeWidth={2} /></svg>
        <div className="job-kpis">{[
          ["Return", `${report.expected_return_percent.toFixed(2)}%`],
          ["Daily-close drawdown", `${report.path_maximum_drawdown_percent.toFixed(2)}%`],
          ["Highest selected correlation", report.maximum_observed_pairwise_correlation.toFixed(3)],
          ["Stress P95 drawdown", `${report.stress.p95_maximum_drawdown_percent.toFixed(2)}%`],
        ].map(([label, value]) => <article className="kpi" key={label}><span>{label}</span><strong>{value}</strong></article>)}</div>
        {report.selected.map((allocation) => { const row = result.candidates.find((r) => r.id === allocation.strategy_fingerprint); return <div className="campaign-history-row" key={allocation.strategy_fingerprint}><strong>{allocation.symbol}</strong><span>{row?.strategyId}</span><span>{(allocation.weight * 100).toFixed(1)}% capital</span><span>Original Development {row?.developmentExpectancyR.toFixed(3)}R</span></div>; })}
        <p className="immutable-note">Capital allocated to strategies involving each currency (both FX legs counted; this is allocation participation, not net currency notional): {Object.entries(result.currencyAllocationParticipation).map(([currency, value]) => `${currency} ${(value * 100).toFixed(1)}%`).join(" · ")}</p>
      </section>
      <section className="panel"><div className="panel-heading"><div><p className="eyebrow">Diversity</p><h2>{result.clusters.length} correlation groups from {result.candidates.length} candidates</h2></div></div><p className="immutable-note">Groups connect strategies above your correlation ceiling. A connection can be indirect; allocation still enforces every selected pair’s limit.</p>
        {result.clusters.filter((group) => group.length > 1).map((group, i) => <details className="advanced-settings" key={group[0]}><summary>Group {i + 1} · {group.length} similar strategies</summary><p>{group.map((id) => { const row = result.candidates.find((r) => r.id === id); return `${row?.symbol} ${row?.strategyId}`; }).join(" · ")}</p></details>)}
        <p className="immutable-note">Saved report: {result.outputPath}</p>
      </section>
    </>}
  </div>;
}
