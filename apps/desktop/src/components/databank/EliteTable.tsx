import React, { useState, useMemo, useEffect } from 'react';
import type { EliteRow } from '../../types';
import { conditionLabel, formatNumber } from '../../view';
import { ColumnPicker, ColumnConfig, DEFAULT_COLUMNS } from './ColumnPicker';

function formatRecoveryFactor(row: EliteRow): string {
  if (row.recoveryFactor === null) return "—";
  return row.recoveryFactor.toFixed(2);
}

function Sparkline({ values }: { values: number[] }) {
  if (!values || values.length === 0) return <div className="sparkline empty" />;

  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const width = 60;
  const height = 24;

  const points = values.map((val, i) => {
    const x = (i / (values.length - 1)) * width;
    const y = height - ((val - min) / range) * height;
    return `${x},${y}`;
  }).join(' ');

  const endVal = values[values.length - 1] ?? 0;
  const startVal = values[0] ?? 0;
  const isPositive = endVal >= startVal;

  return (
    <svg width={width} height={height} className="sparkline" viewBox={`0 0 ${width} ${height}`}>
      <polyline
        points={points}
        fill="none"
        stroke={isPositive ? 'var(--positive)' : 'var(--negative)'}
        strokeWidth="1.5"
      />
    </svg>
  );
}

export interface EliteTableProps {
  rows: EliteRow[];
  selected: string | null;
  batchSelection: Set<string>;
  onSelect: (fingerprint: string) => void;
  onToggle: (fingerprint: string) => void;
  onDoubleClick: (fingerprint: string) => void;
  showBatch?: boolean;
  showActions?: boolean;
  onExportIr?: (fingerprint: string) => void;
  onExportMq5?: (fingerprint: string) => void;
  onStage?: (fingerprint: string) => void;
  rowHeightOverride?: number;
  viewportHeightOverride?: number;
}

export function EliteTable({
  rows,
  selected,
  batchSelection,
  onSelect,
  onToggle,
  onDoubleClick,
  showBatch = true,
  showActions = false,
  onExportIr,
  onExportMq5,
  onStage,
  rowHeightOverride,
  viewportHeightOverride,
}: EliteTableProps) {
  const [scrollTop, setScrollTop] = useState(0);
  const [columns, setColumns] = useState<ColumnConfig[]>(() => {
    const saved = localStorage.getItem('quantforge_elitetable_cols');
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        // Merge saved visibility with defaults to handle new/removed columns
        return DEFAULT_COLUMNS.map(def => {
          const found = parsed.find((p: any) => p.id === def.id);
          return found ? { ...def, visible: found.visible } : def;
        });
      } catch (e) {
        return DEFAULT_COLUMNS;
      }
    }
    return DEFAULT_COLUMNS;
  });

  const [sortCol, setSortCol] = useState<string | null>(null);
  const [sortDesc, setSortDesc] = useState<boolean>(true);
  const [secondarySortCol, setSecondarySortCol] = useState<string | null>(null);
  const [secondarySortDesc, setSecondarySortDesc] = useState<boolean>(true);

  useEffect(() => {
    localStorage.setItem('quantforge_elitetable_cols', JSON.stringify(columns.map(c => ({ id: c.id, visible: c.visible }))));
  }, [columns]);

  const handleSort = (colId: string, isShift: boolean) => {
    if (isShift) {
      if (secondarySortCol === colId) {
        if (!secondarySortDesc) setSecondarySortCol(null);
        else setSecondarySortDesc(false);
      } else {
        setSecondarySortCol(colId);
        setSecondarySortDesc(true);
      }
    } else {
      if (sortCol === colId) {
        if (!sortDesc) setSortCol(null);
        else setSortDesc(false);
      } else {
        setSortCol(colId);
        setSortDesc(true);
        setSecondarySortCol(null);
      }
    }
  };

  const sortedRows = useMemo(() => {
    if (!sortCol) return rows;

    return [...rows].sort((a, b) => {
      const cmp = (col: string, rowA: any, rowB: any): number => {
        let valA, valB;
        switch (col) {
          case 'strategyId': valA = rowA.strategyId; valB = rowB.strategyId; break;
          case 'conditions': valA = rowA.entryConditions + rowA.exitConditions; valB = rowB.entryConditions + rowB.exitConditions; break;
          case 'evidence': valA = rowA.evidence; valB = rowB.evidence; break;
          case 'expectancyR': valA = rowA.expectancyR ?? -Infinity; valB = rowB.expectancyR ?? -Infinity; break;
          case 'foldMedianR': valA = rowA.foldMedianR ?? -Infinity; valB = rowB.foldMedianR ?? -Infinity; break;
          case 'novelty': valA = rowA.novelty; valB = rowB.novelty; break;
          case 'returnPercent': valA = rowA.returnPercent; valB = rowB.returnPercent; break;
          case 'trades': valA = rowA.trades; valB = rowB.trades; break;
          case 'drawdownPercent': valA = rowA.drawdownPercent; valB = rowB.drawdownPercent; break;
          case 'recoveryFactor': valA = rowA.recoveryFactor ?? -Infinity; valB = rowB.recoveryFactor ?? -Infinity; break;
          case 'sharpe': valA = rowA.sharpeRatio ?? -Infinity; valB = rowB.sharpeRatio ?? -Infinity; break;
          case 'grade': valA = rowA.grade; valB = rowB.grade; break;
          default: return 0;
        }

        if (typeof valA === 'string' && typeof valB === 'string') return valA.localeCompare(valB);
        return (valA as number) - (valB as number);
      };

      let res = cmp(sortCol, a, b);
      if (!sortDesc) res = -res;

      if (res === 0 && secondarySortCol) {
        let secRes = cmp(secondarySortCol, a, b);
        if (!secondarySortDesc) secRes = -secRes;
        return secRes;
      }

      return res;
    });
  }, [rows, sortCol, sortDesc, secondarySortCol, secondarySortDesc]);

  const rowHeight = rowHeightOverride ?? (showActions ? 56 : 48);
  const viewportHeight = viewportHeightOverride ?? 360;
  const overscan = 5;
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(sortedRows.length, start + Math.ceil(viewportHeight / rowHeight) + overscan * 2);

  const visibleCols = columns.filter(c => c.visible);

  return (
    <div className={`elite-table ${showBatch ? "" : "no-batch"} ${showActions ? "with-actions" : ""}`} style={{ display: 'flex', flexDirection: 'column' }}>
      <div style={{ display: 'flex', justifyContent: 'flex-end', marginBottom: '8px' }}>
        <ColumnPicker columns={columns} onChange={setColumns} />
      </div>

      <div className="elite-row elite-header">
        {showBatch && <span style={{ width: 40 }}>Select</span>}
        {visibleCols.map(col => (
          <span
            key={col.id}
            style={{ width: col.width, cursor: 'pointer', userSelect: 'none', flex: col.width ? `0 0 ${col.width}px` : 1 }}
            onClick={(e) => handleSort(col.id, e.shiftKey)}
            title={col.id !== 'sparkline' ? "Click to sort, Shift+Click for secondary sort" : ""}
          >
            {col.label}
            {sortCol === col.id && (sortDesc ? ' ↓' : ' ↑')}
            {secondarySortCol === col.id && (secondarySortDesc ? ' ₂↓' : ' ₂↑')}
          </span>
        ))}
        {showActions && <span style={{ width: 120 }}>Actions</span>}
      </div>

      <div
        className="elite-viewport"
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        style={{ height: viewportHeight, overflowY: 'auto', position: 'relative' }}
      >
        <div className="elite-virtual-body" style={{ height: sortedRows.length * rowHeight, position: 'relative' }}>
          {sortedRows.slice(start, end).map((row, offset) => (
            <div
              className={`elite-row interactive ${selected === row.fingerprint ? "selected" : ""}`}
              key={row.fingerprint}
              onClick={() => onSelect(row.fingerprint)}
              onDoubleClick={() => onDoubleClick?.(row.fingerprint)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") onSelect(row.fingerprint);
              }}
              role="button"
              style={{
                height: rowHeight,
                position: 'absolute',
                top: 0,
                left: 0,
                right: 0,
                transform: `translateY(${(start + offset) * rowHeight}px)`
              }}
              tabIndex={0}
            >
              {showBatch && (
                <span className="batch-check" style={{ width: 40 }}>
                  <input
                    aria-label={`Select ${row.strategyId} for batch export`}
                    checked={batchSelection.has(row.fingerprint)}
                    onChange={() => onToggle(row.fingerprint)}
                    onClick={(event) => event.stopPropagation()}
                    type="checkbox"
                  />
                </span>
              )}

              {visibleCols.map(col => {
                const style = { width: col.width, flex: col.width ? `0 0 ${col.width}px` : 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' as const };
                switch (col.id) {
                  case 'sparkline':
                    return <span key={col.id} style={style}><Sparkline values={row.equitySignature || []} /></span>;
                  case 'strategyId':
                    return <span key={col.id} className="strategy-cell" style={style}><strong>{row.strategyId}</strong><small>{row.fingerprint.slice(0, 10)}</small></span>;
                  case 'conditions':
                    return <span key={col.id} style={style}>{conditionLabel(row.entryConditions, row.exitConditions)}</span>;
                  case 'evidence':
                    return <span key={col.id} style={style}>{row.evidence.toFixed(2)}</span>;
                  case 'expectancyR':
                    return <span key={col.id} style={style}>{(row.expectancyR ?? 0).toFixed(3)}</span>;
                  case 'foldMedianR':
                    return <span key={col.id} style={style} title={row.foldUsable ? `spread ${(row.foldSpread ?? 0).toFixed(3)} · ${row.foldCount} years` : 'too few trades per year; pooled R'}>{(row.foldMedianR ?? 0).toFixed(3)}</span>;
                  case 'novelty':
                    return <span key={col.id} style={style}>{row.novelty.toFixed(3)}</span>;
                  case 'returnPercent':
                    return <span key={col.id} style={style} className={row.returnPercent >= 0 ? "positive" : "negative"}>{row.returnPercent.toFixed(2)}%</span>;
                  case 'trades':
                    return <span key={col.id} style={style}>{formatNumber(row.trades)}</span>;
                  case 'drawdownPercent':
                    return <span key={col.id} style={style}>{row.drawdownPercent.toFixed(2)}%</span>;
                  case 'recoveryFactor':
                    return <span key={col.id} style={style}>{formatRecoveryFactor(row)}</span>;
                  case 'sharpe':
                    return <span key={col.id} style={style}>{row.sharpeRatio === null ? "—" : row.sharpeRatio.toFixed(2)}</span>;
                  case 'profitFactor':
                    return <span key={col.id} style={style}>{row.profitFactor === null ? "—" : row.profitFactor.toFixed(2)}</span>;
                  case 'grade':
                    return <span key={col.id} style={style}>{row.grade}</span>;
                  case 'family':
                  case 'winRate':
                  case 'avgTrade':
                    return <span key={col.id} style={style}>—</span>;
                  default:
                    return <span key={col.id} style={style}>—</span>;
                }
              })}

              {showActions && (
                <span className="row-actions" onClick={(event) => event.stopPropagation()} style={{ width: 120 }}>
                  <button type="button" className="text-action" onClick={() => onExportIr?.(row.fingerprint)}>IR</button>
                  <button type="button" className="text-action" onClick={() => onExportMq5?.(row.fingerprint)}>MQ5</button>
                  <button type="button" className="text-action" onClick={() => onStage?.(row.fingerprint)} title="Stage for certification">Stage</button>
                </span>
              )}
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
