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

function partitionSplitLabel(view: PartitionEquityView): string {
  const totalBars = view.isBars + view.oos1Bars + view.oos2Bars;
  if (totalBars <= 0) return "IS / OOS1 / OOS2";
  const isShare = (view.isBars / totalBars) * 100;
  const oos1Share = (view.oos1Bars / totalBars) * 100;
  const oos2Share = (view.oos2Bars / totalBars) * 100;
  return `${formatNumber(isShare, 0)}% · ${formatNumber(oos1Share, 0)}% · ${formatNumber(oos2Share, 0)}%`;
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

function Kpi({ label, value, note }: { label: string; value: string; note: string }) {
  return (
    <article className="kpi">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>{note}</small>
    </article>
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
  const [sample, setSample] = useState<"full" | "is" | "oos1" | "oos2">("full");

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

  const fullT0 = view.points[0].timestampMs;
  const fullT1 = view.points[view.points.length - 1].timestampMs;
  const fullTSpan = Math.max(fullT1 - fullT0, 1);
  const xAtFull = (timestamp: number) =>
    pad + ((timestamp - fullT0) / fullTSpan) * (width - pad * 2);
  const isX = xAtFull(view.isEndTimestampMs);
  const oos1X = xAtFull(view.oos1EndTimestampMs);
  const splitLabel = partitionSplitLabel(view);
  const gradientId = large ? "equity-area-large" : "equity-area-small";

  const horizontalGuides = [0, 1, 2, 3, 4];
  const verticalGuides = [0, 1, 2, 3, 4, 5, 6];
  const isSegment = view.points.filter((point) => point.timestampMs < view.isEndTimestampMs);
  const oos1Segment = view.points.filter(
    (point) => point.timestampMs >= view.isEndTimestampMs && point.timestampMs < view.oos1EndTimestampMs,
  );
  const oos2Segment = view.points.filter((point) => point.timestampMs >= view.oos1EndTimestampMs);
  const bridgedOos1 = isSegment.length > 0 && oos1Segment.length > 0
    ? [isSegment[isSegment.length - 1], ...oos1Segment]
    : oos1Segment;
  const bridgedOos2 = bridgedOos1.length > 0 && oos2Segment.length > 0
    ? [bridgedOos1[bridgedOos1.length - 1], ...oos2Segment]
    : oos2Segment;
  const segmentPath = (segment: typeof view.points) => segment
    .map((point, index) => `${index === 0 ? "M" : "L"}${xAt(point.timestampMs).toFixed(1)} ${yAt(point.equity).toFixed(1)}`)
    .join(" ");

  return (
    <section className={large ? "partition-equity large" : "partition-equity"}>
      <div className="partition-equity-head">
        <div>
          <p className="eyebrow">M1-chronology full-run equity</p>
          <small>
            {view.executionEngine} · IS / OOS1 / OOS2 {splitLabel} (OOS2 display only)
            {researchGrade && !m1FidelityVerified ? " · research recheck; not an external parity pass" : ""}
          </small>
        </div>
        <div className="partition-equity-scale">
          <label className="partition-sample-select">Sample
            <select value={sample} onChange={(event) => setSample(event.target.value as typeof sample)}>
              <option value="full">Full (IS + OOS)</option>
              <option value="is">IS</option>
              <option value="oos1">OOS 1</option>
              <option value="oos2">OOS 2</option>
            </select>
          </label>
          <span>${formatNumber(max, 0)} peak</span>
          <span>${formatNumber(min, 0)} trough</span>
        </div>
      </div>
      <svg viewBox={`0 0 ${width} ${height}`} className="partition-equity-svg" role="img" aria-label="Partitioned equity curve">
        <defs>
          <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor="#35d4bd" stopOpacity=".30" />
            <stop offset="68%" stopColor="#1eae9a" stopOpacity=".08" />
            <stop offset="100%" stopColor="#1eae9a" stopOpacity="0" />
          </linearGradient>
        </defs>
        {horizontalGuides.map((guide) => {
          const y = pad + (guide / 4) * (height - pad * 2);
          const value = max - (guide / 4) * span;
          return (
            <g key={`h-${guide}`}>
              <line x1={pad} y1={y} x2={width - pad} y2={y} className="chart-grid-line" />
              {large && <text x={width - pad - 4} y={y - 4} textAnchor="end" className="chart-axis-label">${formatNumber(value, 0)}</text>}
            </g>
          );
        })}
        {verticalGuides.map((guide) => {
          const fraction = guide / 6;
          const x = pad + fraction * (width - pad * 2);
          const timestamp = t0 + fraction * tSpan;
          return (
            <g key={`v-${guide}`}>
              <line x1={x} y1={pad} x2={x} y2={height - pad} className="chart-grid-line vertical" />
              {large && <text x={x} y={height - 4} textAnchor={guide === 0 ? "start" : guide === 6 ? "end" : "middle"} className="chart-axis-label">{new Date(timestamp).getUTCFullYear()}</text>}
            </g>
          );
        })}
        <path d={areaPath} fill={`url(#${gradientId})`} className="equity-area" />
        {sample === "full" ? (
          <>
            {isSegment.length >= 2 && <path d={segmentPath(isSegment)} className="equity-path equity-path-is" />}
            {bridgedOos1.length >= 2 && <path d={segmentPath(bridgedOos1)} className="equity-path equity-path-oos1" />}
            {bridgedOos2.length >= 2 && <path d={segmentPath(bridgedOos2)} className="equity-path equity-path-oos2" />}
          </>
        ) : (
          <path d={path} className="equity-path" />
        )}
        {sample === "full" && <>
          <rect x={pad} y={pad} width={Math.max(0, isX - pad)} height={height - pad * 2} className="region-is region-overlay" />
          <rect x={isX} y={pad} width={Math.max(0, oos1X - isX)} height={height - pad * 2} className="region-oos1 region-overlay" />
          <rect x={oos1X} y={pad} width={Math.max(0, width - pad - oos1X)} height={height - pad * 2} className="region-oos2 region-overlay" />
          <line x1={isX} y1={pad} x2={isX} y2={height - pad} className="divider" />
          <line x1={oos1X} y1={pad} x2={oos1X} y2={height - pad} className="divider" />
        </>}
        <circle cx={lastX} cy={yAt(chartPoints.at(-1)!.equity)} r={large ? 3.2 : 2.5} className="equity-endpoint" />
        {sample === "full" ? <>
          <text x={pad + 6} y={pad + 14} className="region-label">IS</text>
          <text x={isX + 6} y={pad + 14} className="region-label">OOS1</text>
          <text x={oos1X + 6} y={pad + 14} className="region-label">OOS2</text>
        </> : <text x={pad + 6} y={pad + 14} className="region-label">{sample === "is" ? "IS" : sample.toUpperCase()}</text>}
      </svg>
      <div className="partition-kpis">
        <Kpi label="IS expectancy (R)" value={formatNumber(view.isExpectancy / 1000, 2)} note={`${formatNumber(view.isTrades)} trades · ${formatNumber(view.isReturnPercent, 2)}% return`} />
        <Kpi label="OOS1 expectancy (R)" value={formatNumber(view.oos1Expectancy / 1000, 2)} note={view.oos1ExpectancyRatio === null ? "no ratio" : `${view.oos1ExpectancyRatio.toFixed(2)}× IS · chart split`} />
        <Kpi label="OOS2 expectancy (R)" value={formatNumber(view.oos2Expectancy / 1000, 2)} note="display only · not a gate" />
        <Kpi label="Bars" value={`${formatNumber(view.isBars)} / ${formatNumber(view.oos1Bars)} / ${formatNumber(view.oos2Bars)}`} note="IS / OOS1 / OOS2 · databank split" />
      </div>
    </section>
  );
}
