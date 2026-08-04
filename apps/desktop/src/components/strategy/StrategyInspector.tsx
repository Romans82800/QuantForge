import React, { useState, useEffect } from 'react';
import type { EliteDetail } from '../../types';
import { formatNumber, describeStrategyConditions } from '../../view';

export interface StrategyInspectorProps {
  detail: EliteDetail | null;
  loading: boolean;
  tab: 'overview' | 'ir';
  onTab: (tab: 'overview' | 'ir') => void;
  onParity: (fingerprint: string) => void;
  onOpenResearch?: (fingerprint: string) => void;
  researchGrade: boolean;
  m1FidelityVerified: boolean;
  onError: (message: string | null) => void;
}

export function StrategyInspector({
  detail,
  loading,
  tab,
  onTab,
  onParity,
  onOpenResearch,
  researchGrade,
  m1FidelityVerified,
  onError
}: StrategyInspectorProps) {
  if (!detail && !loading) {
    return (
      <aside className="panel inspector strategy-inspector empty">
        <div className="inspector-empty">
          <p>Select a strategy to view details</p>
        </div>
      </aside>
    );
  }

  const conditions = detail ? describeStrategyConditions(detail.strategyIr) : null;
  const metrics = detail?.metrics || {};

  return (
    <aside className="panel inspector strategy-inspector">
      {loading && <div className="inspector-loading">Loading strategy details…</div>}

      {detail && (
        <>
          <div className="inspector-head">
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span className="grade-pill">{detail.grade}</span>
              <h3 style={{ margin: 0 }}>{detail.strategyId}</h3>
            </div>
            {onOpenResearch && (
              <button className="text-action" onClick={() => onOpenResearch(detail.fingerprint)}>
                Open in Research
              </button>
            )}
          </div>

          <div className="segmented inspector-tabs">
            <button
              className={tab === 'overview' ? 'active' : ''}
              onClick={() => onTab('overview')}
            >
              Overview
            </button>
            <button
              className={tab === 'ir' ? 'active' : ''}
              onClick={() => onTab('ir')}
            >
              IR Data
            </button>
          </div>

          <div className="inspector-body" style={{ overflowY: 'auto', flex: 1, padding: '16px' }}>
            {tab === 'overview' && (
              <div className="inspector-overview">
                <div className="kpi-grid" style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '16px', marginBottom: '24px' }}>
                  <div className="kpi-item">
                    <div className="kpi-label">Return</div>
                    <div className="kpi-value">{Number(metrics.return_percent || 0).toFixed(2)}%</div>
                  </div>
                  <div className="kpi-item">
                    <div className="kpi-label">Max DD</div>
                    <div className="kpi-value">-{Number(metrics.max_drawdown_percent || 0).toFixed(2)}%</div>
                  </div>
                  <div className="kpi-item">
                    <div className="kpi-label">Trades</div>
                    <div className="kpi-value">{formatNumber(Number(metrics.trade_count || 0))}</div>
                  </div>
                  <div className="kpi-item">
                    <div className="kpi-label">Win Rate</div>
                    <div className="kpi-value">{Number(metrics.win_rate || 0).toFixed(1)}%</div>
                  </div>
                </div>

                <div className="conditions-section">
                  <h4 style={{ borderBottom: '1px solid var(--border)', paddingBottom: '8px', marginBottom: '12px' }}>
                    Entry Conditions ({detail.entryConditions})
                  </h4>
                  {conditions?.entry.length ? (
                    <ul style={{ margin: 0, paddingLeft: '20px', fontSize: '13px' }}>
                      {conditions.entry.map((cond, i) => <li key={i}>{cond}</li>)}
                    </ul>
                  ) : (
                    <p style={{ fontSize: '13px', color: 'var(--fg-dim)' }}>None</p>
                  )}

                  <h4 style={{ borderBottom: '1px solid var(--border)', paddingBottom: '8px', marginTop: '24px', marginBottom: '12px' }}>
                    Exit Conditions ({detail.exitConditions})
                  </h4>
                  {conditions?.exit.length ? (
                    <ul style={{ margin: 0, paddingLeft: '20px', fontSize: '13px' }}>
                      {conditions.exit.map((cond, i) => <li key={i}>{cond}</li>)}
                    </ul>
                  ) : (
                    <p style={{ fontSize: '13px', color: 'var(--fg-dim)' }}>None</p>
                  )}

                  {(conditions?.stopLoss || conditions?.takeProfit) && (
                    <div style={{ marginTop: '12px', fontSize: '13px' }}>
                      {conditions.stopLoss && <div><strong>Stop:</strong> {conditions.stopLoss}</div>}
                      {conditions.takeProfit && <div><strong>Target:</strong> {conditions.takeProfit}</div>}
                    </div>
                  )}
                </div>

                <div style={{ marginTop: '32px' }}>
                  <button
                    className="primary"
                    style={{ width: '100%' }}
                    onClick={() => onParity(detail.fingerprint)}
                  >
                    Verify in Parity Lab
                  </button>
                  {!m1FidelityVerified && (
                    <p style={{ fontSize: '12px', color: 'var(--fg-dim)', textAlign: 'center', marginTop: '8px' }}>
                      Required for live trading
                    </p>
                  )}
                </div>
              </div>
            )}

            {tab === 'ir' && (
              <div className="inspector-ir">
                <pre style={{
                  background: 'var(--bg-inset)',
                  padding: '12px',
                  borderRadius: '4px',
                  fontSize: '12px',
                  overflowX: 'auto'
                }}>
                  {JSON.stringify(detail.strategyIr, null, 2)}
                </pre>
              </div>
            )}
          </div>
        </>
      )}
    </aside>
  );
}
