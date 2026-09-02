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
  const [sample, setSample] = useState<"full" | "is" | "oos1" | "oos2">("is");

  if (busy && !view) {
    return <div className="partition-equity loading">Replaying full IS / OOS1 / OOS2 equity…</div>;
  }

  if (!view || view.points.length < 2) {
    return <div className="partition-equity empty">Equity unavailable for this elite.</div>;
  }

  const points = sample === "full"
    ? view.points
    : view.points.filter((point) => (
      sample === "is"
        ? point.timestampMs < view.isEndTimestampMs
        : sample === "oos1"
          ? point.timestampMs >= view.isEndTimestampMs && point.timestampMs < view.oos1EndTimestampMs
          : point.timestampMs >= view.oos1EndTimestampMs
    ));

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

  const isX = xAt(view.isEndTimestampMs);
  const oos1X = xAt(view.oos1EndTimestampMs);
  const gradientId = large ? "equity-area-large" : "equity-area-small";

  const horizontalGuides = [0, 1, 2, 3, 4];
  const verticalGuides = [0, 1, 2, 3, 4, 5, 6];
  const totalBars = view.isBars + view.oos1Bars + view.oos2Bars;
  const isPct = totalBars > 0 ? Math.round((view.isBars / totalBars) * 100) : 0;
  const oos2Pct = totalBars > 0 ? Math.round((view.oos2Bars / totalBars) * 100) : 0;
  const twoWay = view.oos1Bars === 0;
  const splitNote = twoWay
    ? `${view.executionEngine} · Development ${isPct}% · Holdout ${oos2Pct}% (display only)`
    : `${view.executionEngine} · Development ${isPct}% · OOS1 ${Math.round((view.oos1Bars / Math.max(totalBars, 1)) * 100)}% · OOS2 ${oos2Pct}% (display only)`;

  return (
    <section className={large ? "partition-equity large" : "partition-equity"}>
      <div className="partition-equity-head" style={{ display: 'flex', justifyContent: 'space-between', marginBottom: '16px' }}>
        <div>
          <p className="eyebrow">{sample === "is" ? "M1-chronology Development equity" : "M1-chronology full-run equity"}</p>
          <small style={{ color: 'var(--fg-dim)', fontSize: '12px' }}>
            {splitNote}
            {researchGrade && !m1FidelityVerified ? " · research recheck; not an external parity pass" : ""}
          </small>
        </div>
        <div className="partition-equity-scale" style={{ display: 'flex', gap: '16px', alignItems: 'center', fontSize: '13px' }}>
          <label className="partition-sample-select" style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
            Sample
            <select
              value={sample}
              onChange={(event) => setSample(event.target.value as typeof sample)}
              style={{ padding: '4px', borderRadius: '4px', background: 'var(--bg-inset)', border: '1px solid var(--border)', color: 'var(--fg)' }}
            >
              <option value="is">Development (matches Databank)</option>
              <option value="full">Full (IS + OOS)</option>
              <option value="oos1">OOS 1</option>
              <option value="oos2">OOS 2</option>
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

        {sample === "full" && (
          <>
            <line x1={isX} y1={pad} x2={isX} y2={height - pad} stroke="var(--negative)" strokeWidth="1.5" strokeDasharray="4 4" />
            <line x1={oos1X} y1={pad} x2={oos1X} y2={height - pad} stroke="var(--negative)" strokeWidth="1.5" strokeDasharray="4 4" />
            <text x={isX - 8} y={pad + 12} fill="var(--fg-dim)" fontSize="10px" textAnchor="end">IS end</text>
            <text x={oos1X - 8} y={pad + 12} fill="var(--fg-dim)" fontSize="10px" textAnchor="end">OOS1 end</text>
          </>
        )}

        <path d={areaPath} fill={`url(#${gradientId})`} />
        <path d={path} fill="none" stroke="var(--accent)" strokeWidth="1.5" />
      </svg>
    </section>
  );
}
