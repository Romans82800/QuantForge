import React, { useState, useRef, useEffect } from 'react';

export type ColumnId = 'sparkline' | 'strategyId' | 'family' | 'conditions' | 'evidence' | 'novelty' | 'returnPercent' | 'trades' | 'winRate' | 'profitFactor' | 'sharpe' | 'drawdownPercent' | 'recoveryFactor' | 'avgTrade' | 'grade';

export interface ColumnConfig {
  id: ColumnId;
  label: string;
  visible: boolean;
  width?: number;
}

export const DEFAULT_COLUMNS: ColumnConfig[] = [
  { id: 'sparkline', label: 'Sparkline', visible: true, width: 80 },
  { id: 'strategyId', label: 'Strategy', visible: true, width: 140 },
  { id: 'conditions', label: 'E/X', visible: true, width: 50 },
  { id: 'evidence', label: 'Evidence', visible: true, width: 75 },
  { id: 'novelty', label: 'Novelty', visible: false, width: 70 },
  { id: 'returnPercent', label: 'Return %', visible: true, width: 80 },
  { id: 'trades', label: 'Trades', visible: true, width: 65 },
  { id: 'winRate', label: 'Win %', visible: false, width: 60 },
  { id: 'profitFactor', label: 'PF', visible: false, width: 55 },
  { id: 'sharpe', label: 'Sharpe', visible: true, width: 65 },
  { id: 'drawdownPercent', label: 'Max DD %', visible: true, width: 75 },
  { id: 'recoveryFactor', label: 'RF', visible: true, width: 55 },
  { id: 'grade', label: 'Grade', visible: false, width: 55 },
];

export interface ColumnPickerProps {
  columns: ColumnConfig[];
  onChange: (columns: ColumnConfig[]) => void;
}

export function ColumnPicker({ columns, onChange }: ColumnPickerProps) {
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (containerRef.current && !containerRef.current.contains(event.target as Node)) {
        setIsOpen(false);
      }
    }
    if (isOpen) {
      document.addEventListener('mousedown', handleClickOutside);
    }
    return () => {
      document.removeEventListener('mousedown', handleClickOutside);
    };
  }, [isOpen]);

  const toggleColumn = (id: ColumnId) => {
    onChange(columns.map(col => col.id === id ? { ...col, visible: !col.visible } : col));
  };

  return (
    <div className="column-picker" ref={containerRef} style={{ position: 'relative', display: 'inline-block' }}>
      <button
        type="button"
        className="secondary"
        onClick={() => setIsOpen(!isOpen)}
        title="Toggle columns"
      >
        <svg aria-hidden="true" viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
          <path d="M4 6h16v2H4zm0 5h16v2H4zm0 5h16v2H4z" />
        </svg>
      </button>
      {isOpen && (
        <div className="column-picker-dropdown" style={{
          position: 'absolute',
          top: '100%',
          right: 0,
          marginTop: '4px',
          background: 'var(--bg-panel)',
          border: '1px solid var(--border)',
          borderRadius: '4px',
          padding: '8px',
          zIndex: 100,
          minWidth: '150px',
          boxShadow: '0 4px 12px rgba(0,0,0,0.5)'
        }}>
          {columns.map(col => (
            <label key={col.id} style={{ display: 'flex', alignItems: 'center', gap: '8px', padding: '4px', cursor: 'pointer' }}>
              <input
                type="checkbox"
                checked={col.visible}
                onChange={() => toggleColumn(col.id)}
              />
              <span style={{ fontSize: '13px' }}>{col.label}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  );
}
