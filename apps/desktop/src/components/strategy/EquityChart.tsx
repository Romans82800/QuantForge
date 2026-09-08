import React, { useState } from 'react';
import type { EliteRow, PartitionEquityView } from '../../types';
import { formatNumber } from '../../view';

function equityCurveValues(
  signature: number[] | undefined,
  initialBalance: number,
  returnPercent: number,
): number[] {
  if (!signature || signature.length === 0) return [initialBalance, initialBalance * (1 + returnPercent / 100)];
  return signature;
}

export function StoredSignatureChart({
  initialBalance,
  returnPercent,
  values,
}: {
  initialBalance: number;
  returnPercent: number;
  values: number[];
}) {
  const curve = equityCurveValues(values, initialBalance, returnPercent);
  if (curve.length < 2) {
    return <p className="cluster-empty">No equity signature recorded for this elite.</p>;
  }
  const minimum = Math.min(...curve);
  const maximum = Math.max(...curve);
  const range = Math.max(maximum - minimum, 1e-9);
  const points = curve
    .map((value, index) => `${(index / Math.max(curve.length - 1, 1)) * 900},${300 - ((value - minimum) / range) * 288}`)
    .join(" ");
  return (
    <div className="equity-chart results-equity-chart">
      <span className="equity-axis equity-high">{formatNumber(maximum, 0)}</span>
      <span className="equity-axis equity-low">{formatNumber(minimum, 0)}</span>
      <svg viewBox="0 0 900 305" role="img" aria-label="Stored equity signature">
        <polyline fill="none" points={points} style={{ stroke: 'var(--positive)', strokeWidth: 1.5 }} />
      </svg>
    </div>
  );
}

export function EquityCurvePanel({
  initialBalance,
  m1FidelityVerified,
  rows,
}: {
  initialBalance: number;
  m1FidelityVerified: boolean;
  rows: EliteRow[];
}) {
  const visible = rows.slice(0, 12);
  const curves = visible.map((row) => ({
    row,
    values: equityCurveValues(
      row.equitySignature,
      initialBalance,
      row.returnPercent,
    ),
  }));
  const allValues = curves.flatMap((curve) => curve.values);
  const minimum = allValues.length > 0 ? Math.min(...allValues) : initialBalance;
  const maximum = allValues.length > 0 ? Math.max(...allValues) : initialBalance;
  const range = Math.max(maximum - minimum, 1e-9);
  const colors = [
    "#5bd7cc", "#78a7e6", "#e7b65c", "#b896ec", "#70d19a", "#ec756d",
    "#9ed36a", "#e58ec8", "#6fc9ef", "#d4c66a", "#8f9df0", "#df9b6c",
  ];

  return (
    <section className="panel equity-panel">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Stored archive paths</p>
          <h2>{m1FidelityVerified ? "M1-verified equity curves" : "Selected-timeframe equity curves"}</h2>
        </div>
        <span className="read-only-badge">
          {visible.length === 0 ? "Select a strategy" : `${visible.length} shown`}
        </span>
      </div>
      {visible.length === 0 ? (
        <p className="cluster-empty">Select a strategy or tick several rows to compare their equity curves.</p>
      ) : (
        <>
          <div className="equity-chart">
            <span className="equity-axis equity-high">{formatNumber(maximum, 0)}</span>
            <span className="equity-axis equity-low">{formatNumber(minimum, 0)}</span>
            <svg viewBox="0 0 900 230" role="img" aria-label="Selected strategy cumulative equity curves">
              {curves.map(({ row, values }, curveIndex) => {
                const points = values
                  .map((value, index) =>
                    `${(index / Math.max(values.length - 1, 1)) * 900},${220 - ((value - minimum) / range) * 210}`)
                  .join(" ");
                return (
                  <polyline
                    fill="none"
                    key={row.fingerprint}
                    points={points}
                    style={{ stroke: colors[curveIndex], strokeWidth: 1.5 }}
                  />
                );
              })}
            </svg>
          </div>
          <div className="equity-legend">
            {curves.map(({ row }, index) => (
              <button key={row.fingerprint} title={row.fingerprint} className="equity-legend-btn">
                <i style={{ background: colors[index], display: 'inline-block', width: '12px', height: '12px', borderRadius: '50%', marginRight: '8px' }} />
                <span>{row.strategyId}</span>
                <strong className={row.returnPercent >= 0 ? "positive" : "negative"} style={{ marginLeft: '8px' }}>
                  {row.returnPercent.toFixed(2)}%
                </strong>
              </button>
            ))}
          </div>
          {rows.length > visible.length && (
            <p className="axis-note" style={{ marginTop: '16px' }}>Showing the first 12 selected strategies to keep the chart readable.</p>
          )}
          <p className="axis-note" style={{ marginTop: '8px' }}>
            {m1FidelityVerified
              ? "Reconstructed from each strategy's stored M1-verified equity signature; start and end balances match its recorded backtest."
              : "Reconstructed from each strategy's stored Selected-TF signature. This is not a M1 or MT5 parity result; use the M1 recheck below and the Fidelity demo before relying on it."}
          </p>
        </>
      )}
    </section>
  );
}

export function PartitionEquityChart({
  view,
  busy,
  researchGrade,
  m1FidelityVerified,
  large = false,
}: {
  view: PartitionEquityView | null;
  busy: boolean;
  researchGrade: boolean;
  m1FidelityVerified: boolean;
  large?: boolean;
}) {
  const [sample, setSample] = useState<"full" | "training" | "validation" | "holdout">("full");

  if (busy && !view) {
    return <div className="partition-equity loading">Replaying full IS / OOS1 / OOS2 equity…</div>;
  }

  if (!view || view.points.length < 2) {
    return <div className="partition-equity empty">Equity unavailable for this elite.</div>;
  }

  const segments = view.segments?.length
    ? view.segments
    : [
        { id: "Development", kind: "training", startTimestampMs: view.points[0].timestampMs, endTimestampMs: view.isEndTimestampMs, bars: view.isBars, trades: view.isTrades, expectancy: view.isExpectancy, returnPercent: view.isReturnPercent },
        { id: "OOS1", kind: "validation", startTimestampMs: view.isEndTimestampMs, endTimestampMs: view.oos1EndTimestampMs, bars: view.oos1Bars, trades: view.oos1Trades, expectancy: view.oos1Expectancy, returnPercent: view.oos1ReturnPercent },
        { id: "OOS2", kind: "holdout", startTimestampMs: view.oos1EndTimestampMs, endTimestampMs: view.oos2EndTimestampMs, bars: view.oos2Bars, trades: view.oos2Trades, expectancy: view.oos2Expectancy, returnPercent: view.oos2ReturnPercent },
      ];
  const selected = sample === "full" ? segments : segments.filter((segment) => segment.kind === sample);
  const start = selected[0]?.startTimestampMs ?? view.points[0].timestampMs;
  const end = selected.at(-1)?.endTimestampMs ?? view.points.at(-1)!.timestampMs;
  const points = sample === "full" ? view.points : view.points.filter((point) => point.timestampMs >= start && point.timestampMs <= end);

  const chartPoints = points.length >= 2 ? points : view.points;
  const width = large ? 1000 : 520;
  const height = large ? 340 : 300;
  const pad = 18;
  const equities = chartPoints.map((point) => point.equity);
  const min = Math.min(...equities);
  const max = Math.max(...equities);
  const span = Math.max(max - min, 1e-9);
  const t0 = chartPoints[0].timestampMs;
  const t1 = chartPoints[chartPoints.length - 1].timestampMs;
  const tSpan = Math.max(t1 - t0, 1);
  const xAt = (timestamp: number) => pad + ((timestamp - t0) / tSpan) * (width - pad * 2);
  const yAt = (equity: number) => height - pad - ((equity - min) / span) * (height - pad * 2);

  const path = chartPoints
    .map((point, index) => `${index === 0 ? "M" : "L"}${xAt(point.timestampMs).toFixed(1)} ${yAt(point.equity).toFixed(1)}`)
    .join(" ");

  const firstX = xAt(chartPoints[0].timestampMs);
  const lastX = xAt(chartPoints.at(-1)!.timestampMs);
  const chartBottom = height - pad;
  const areaPath = `${path} L${lastX.toFixed(1)} ${chartBottom} L${firstX.toFixed(1)} ${chartBottom} Z`;

  const gradientId = large ? "equity-area-large" : "equity-area-small";

  const horizontalGuides = [0, 1, 2, 3, 4];
  const verticalGuides = [0, 1, 2, 3, 4, 5, 6];
  const splitNote = `${view.executionEngine} · ${segments.length} saved phases · display only`;
  const kindLabel = (kind: string) => kind === "training" ? "IST" : kind === "validation" ? "ISV" : "OOS";

  return (
    <section className={large ? "partition-equity large" : "partition-equity"}>
      <div className="partition-equity-head" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
        <div>
          <p className="eyebrow">M1-chronology full-run equity</p>
          <small style={{ color: 'var(--fg-dim)', fontSize: '12px' }}>
            {splitNote}
            {researchGrade && !m1FidelityVerified ? " · research recheck; not an external parity pass" : ""}
          </small>
        </div>
        <div className="partition-equity-scale" style={{ display: 'flex', gap: '16px', alignItems: 'center', fontSize: '13px' }}>
          <label className="partition-sample-select" style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
            View
            <select
              value={sample}
              onChange={(event) => setSample(event.target.value as typeof sample)}
              style={{ padding: '4px', borderRadius: '4px', background: 'var(--bg-inset)', border: '1px solid var(--border)', color: 'var(--fg)' }}
            >
              <option value="full">Full timeline</option>
              {segments.some((segment) => segment.kind === "training") && <option value="training">IST · training</option>}
              {segments.some((segment) => segment.kind === "validation") && <option value="validation">ISV · validation</option>}
              {segments.some((segment) => segment.kind === "holdout") && <option value="holdout">OOS · sealed holdout</option>}
            </select>
          </label>
          <span style={{ color: 'var(--positive)' }}>${formatNumber(max, 0)} peak</span>
          <span style={{ color: 'var(--negative)' }}>${formatNumber(min, 0)} trough</span>
        </div>
      </div>
      <svg viewBox={`0 0 ${width} ${height}`} className="partition-equity-svg" role="img" aria-label="Partitioned equity curve" style={{ width: '100%', height: 'auto', background: 'var(--bg-inset)', borderRadius: '8px' }}>
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="var(--accent)" stopOpacity=".30" />
            <stop offset="68%" stopColor="var(--accent)" stopOpacity=".08" />
            <stop offset="100%" stopColor="var(--accent)" stopOpacity="0" />
          </linearGradient>
        </defs>

        {horizontalGuides.map((guide) => {
          const y = pad + (guide / 4) * (height - pad * 2);
          const value = max - (guide / 4) * span;
          return (
            <g key={`h-${guide}`}>
              <line x1={pad} y1={y} x2={width - pad} y2={y} stroke="var(--border)" strokeDasharray="4 4" />
              {large && <text x={width - pad - 4} y={y - 4} textAnchor="end" fill="var(--fg-dim)" fontSize="10px">${formatNumber(value, 0)}</text>}
            </g>
          );
        })}

        {verticalGuides.map((guide) => {
          const fraction = guide / 6;
          const x = pad + fraction * (width - pad * 2);
          return (
            <g key={`v-${guide}`}>
              <line x1={x} y1={pad} x2={x} y2={height - pad} stroke="var(--border)" strokeDasharray="4 4" />
            </g>
          );
        })}

        {segments.map((segment) => {
          const startX = xAt(segment.startTimestampMs);
          const endX = xAt(segment.endTimestampMs);
          const active = sample === "full" || segment.kind === sample;
          const regionClass = segment.kind === "training" ? "region-is" : segment.kind === "validation" ? "region-oos1" : "region-oos2";
          return <g key={`${segment.id}-${segment.startTimestampMs}`} opacity={active ? 1 : 0.28}>
            <rect x={startX} y={pad} width={Math.max(0, endX - startX)} height={height - pad * 2} className={regionClass} />
            {startX > pad + 0.5 && <line x1={startX} y1={pad} x2={startX} y2={height - pad} stroke="var(--negative)" strokeWidth="1.5" strokeDasharray="4 4" />}
            <text x={Math.min(width - pad - 4, startX + 6)} y={pad + 12} fill="var(--fg-dim)" fontSize="10px">{segment.id} · {kindLabel(segment.kind)}</text>
          </g>;
        })}

        <path d={areaPath} fill={`url(#${gradientId})`} />
        <path d={path} fill="none" stroke="var(--accent)" strokeWidth="1.5" />
      </svg>
    </section>
  );
}
