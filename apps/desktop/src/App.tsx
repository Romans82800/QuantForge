import { useEffect, useMemo, useRef, useState } from "react";
import {
  assembleEvidence,
  buildDeploymentPack,
  buildPortfolio,
  certifyToVault,
  chooseAndLoadDatabank,
  chooseBrokerFile,
  chooseDataFile,
  chooseDatabank,
  chooseDirectory,
  chooseJsonFile,
  chooseMetadataFile,
  chooseMql5File,
  chooseNewDirectory,
  chooseNewDatabank,
  chooseOutputJson,
  chooseTesterCsv,
  compareExternalParity,
  compareIndicatorParity,
  exportMql5,
  exportEliteStrategy,
  exportEliteStrategies,
  getDiscoverJob,
  getElitePartitionEquity,
  inspectData,
  inspectVault,
  listSymbols,
  loadDatabankPath,
  loadElite,
  pauseDiscover,
  recordIncubation,
  resumeDiscover,
  runFidelityDemo,
  runM1Judge,
  runSealedFinal,
  startIncubation,
  finalizeIncubation,
  startDiscover,
  stopDiscover,
} from "./api";
import type {
  AssembleEvidenceRequest,
  DatabankWorkspace,
  DataLabView,
  DeployRequest,
  DeployView,
  DiscoverJobView,
  DiscoverRequest,
  EliteDetail,
  EliteRow,
  FidelityDemoView,
  EliteSort,
  ExportRequest,
  ExportView,
  EvidenceView,
  FamilyCoverage,
  JudgeRequest,
  JudgeView,
  IncubationStartRequest,
  IncubationView,
  IndicatorParityRequest,
  IndicatorParityView,
  ParityRequest,
  ParityView,
  PartitionEquityView,
  PortfolioRequest,
  PortfolioView,
  SealedRequest,
  SealedView,
  SymbolPack,
  VaultView,
  WorkspaceName,
} from "./types";
import {
  bindDiscoverTimezone,
  discoverProgress,
  discoverProgressLabel,
  filterAndSortElites,
  formatNumber,
  selectionBiasTone,
  visibleEliteFingerprint,
} from "./view";

const workspaces: { name: WorkspaceName; enabled: boolean }[] = [
  { name: "Home", enabled: true },
  { name: "Databank", enabled: true },
  { name: "Discover", enabled: true },
  { name: "Vault", enabled: true },
  { name: "Data Lab", enabled: true },
  { name: "Parity Lab", enabled: true },
  { name: "Portfolio", enabled: true },
  { name: "Deploy", enabled: true },
];

const workspaceTitles: Record<WorkspaceName, string> = {
  Home: "Research cockpit",
  Databank: "IS / OOS1 / OOS2 strategy lab",
  Discover: "Deterministic strategy discovery",
  Vault: "Certified strategy vault",
  "Data Lab": "MT5 data diagnostics",
  "Parity Lab": "External execution parity",
  Portfolio: "Portfolio construction",
  Deploy: "Deployment controls",
};

const gradeLegend = [
  "Scouted",
  "Accepted",
  "Illuminated",
  "Challenged",
  "Parity-Passed",
  "Certified",
  "Deployed",
];

function App() {
  const [activeWorkspace, setActiveWorkspace] = useState<WorkspaceName>("Home");
  const [workspace, setWorkspace] = useState<DatabankWorkspace | null>(null);
  const [detail, setDetail] = useState<EliteDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [family, setFamily] = useState("all");
  const [sort, setSort] = useState<EliteSort>("evidence");
  const [detailTab, setDetailTab] = useState<"overview" | "ir">("overview");
  const [discoverPreset, setDiscoverPreset] = useState<Partial<DiscoverRequest>>({});
  const [judgePreset, setJudgePreset] = useState<Partial<JudgeRequest>>({});
  const [exportPreset, setExportPreset] = useState<Partial<ExportRequest>>({});
  const [batchSelection, setBatchSelection] = useState<Set<string>>(new Set());
  const [batchSize, setBatchSize] = useState(10);
  const [batchMessage, setBatchMessage] = useState<string | null>(null);
  const [batchBusy, setBatchBusy] = useState(false);
  const detailRequest = useRef(0);

  async function selectElite(fingerprint: string) {
    const request = ++detailRequest.current;
    setDetailLoading(true);
    setError(null);
    try {
      const loaded = await loadElite(fingerprint);
      if (request === detailRequest.current) setDetail(loaded);
    } catch (reason) {
      if (request === detailRequest.current) setError(String(reason));
    } finally {
      if (request === detailRequest.current) setDetailLoading(false);
    }
  }

  async function openDatabank() {
    setLoading(true);
    setError(null);
    try {
      const loaded = await chooseAndLoadDatabank();
      if (!loaded) return;
      detailRequest.current += 1;
      setWorkspace(loaded);
      setActiveWorkspace("Databank");
      setDetail(null);
      setDetailLoading(false);
      setBatchSelection(new Set());
      setBatchMessage(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  async function openDatabankPath(path: string) {
    setLoading(true);
    setError(null);
    try {
      const loaded = await loadDatabankPath(path);
      detailRequest.current += 1;
      setWorkspace(loaded);
      setDetail(null);
      setDetailLoading(false);
      setBatchSelection(new Set());
      setBatchMessage(null);
      setActiveWorkspace("Databank");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  function useDataInDiscover(report: DataLabView) {
    setDiscoverPreset({
      dataPath: report.sourcePath,
      metadataPath: report.metadataPath,
      sourceTimezone: report.metadataPath ? null : report.sourceTimezone,
      brokerPath: report.brokerPath ?? "",
    });
    setError(null);
    setActiveWorkspace("Discover");
  }

  function toggleBatchElite(fingerprint: string) {
    setBatchMessage(null);
    setBatchSelection((current) => {
      const next = new Set(current);
      if (next.has(fingerprint)) next.delete(fingerprint);
      else next.add(fingerprint);
      return next;
    });
  }

  function selectTopElites() {
    setBatchMessage(null);
    setBatchSelection(
      new Set(filtered.slice(0, Math.max(1, batchSize)).map((row) => row.fingerprint)),
    );
  }

  async function exportSelectedElites() {
    setBatchBusy(true);
    setBatchMessage(null);
    setError(null);
    try {
      const result = await exportEliteStrategies([...batchSelection]);
      if (result) {
        setBatchMessage(
          `Exported ${result.strategyPaths.length} strategies and batch index to ${result.directory}`,
        );
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatchBusy(false);
    }
  }

  async function continueToParity(fingerprint: string) {
    if (!workspace) return;
    setError(null);
    try {
      const strategyPath = await exportEliteStrategy(fingerprint);
      if (!strategyPath) return;
      setJudgePreset({
        decisionDataPath: workspace.dataPath,
        decisionMetadataPath: workspace.metadataPath,
        decisionSourceTimezone: workspace.metadataPath ? null : "Etc/UTC",
        m1DataPath: workspace.m1DataPath ?? "",
        m1MetadataPath: workspace.m1MetadataPath,
        m1SourceTimezone: workspace.m1MetadataPath ? null : "Etc/UTC",
        strategyPath,
        brokerPath: workspace.brokerPath,
        commissionPerLotRoundTurn: workspace.commissionPerLotRoundTurn,
        slippagePointsPerSide: workspace.slippagePointsPerSide,
        initialBalance: workspace.initialBalance,
      });
      setExportPreset({
        strategyPath,
        brokerPath: workspace.brokerPath,
        commissionPerLotRoundTurn: workspace.commissionPerLotRoundTurn,
        slippagePointsPerSide: workspace.slippagePointsPerSide,
        deposit: workspace.initialBalance,
      });
      setActiveWorkspace("Parity Lab");
    } catch (reason) {
      setError(String(reason));
    }
  }

  const filtered = useMemo(
    () =>
      workspace
        ? filterAndSortElites(workspace.elites, query, family, sort)
        : [],
    [workspace, query, family, sort],
  );

  useEffect(() => {
    if (!workspace) return;
    const visibleFingerprint = visibleEliteFingerprint(
      filtered,
      detail?.fingerprint ?? null,
    );
    if (visibleFingerprint === detail?.fingerprint) return;
    if (!visibleFingerprint) {
      detailRequest.current += 1;
      setDetail(null);
      setDetailLoading(false);
      return;
    }
    void selectElite(visibleFingerprint);
  }, [workspace, filtered, detail?.fingerprint]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">QF</span>
          <div>
            <strong>QuantForge</strong>
            <small>Research cockpit</small>
          </div>
        </div>
        <nav aria-label="Workspaces">
          {workspaces.map(({ name, enabled }) => (
            <button
              className={name === activeWorkspace ? "nav-item active" : "nav-item"}
              disabled={!enabled}
              key={name}
              onClick={() => {
                setError(null);
                setActiveWorkspace(name);
              }}
              title={enabled ? undefined : "Workspace UI is not wired yet"}
            >
              <span className="nav-dot" />
              {name}
              {!enabled && <small>soon</small>}
            </button>
          ))}
        </nav>
        <div className="grade-legend">
          <p>Strategy grades</p>
          {gradeLegend.map((grade) => (
            <div key={grade}>
              <span className={`grade-dot grade-${grade.toLowerCase()}`} />
              {grade}
            </div>
          ))}
        </div>
        <div className="engine-status">
          <span className="status-light" />
          Rust engine local
        </div>
      </aside>

      <main>
        <header className="topbar">
          <div>
            <p className="eyebrow">Workspace / {activeWorkspace}</p>
            <h1>{workspaceTitles[activeWorkspace]}</h1>
          </div>
          {activeWorkspace === "Databank" && (
            <button className="primary" onClick={openDatabank} disabled={loading}>
              {loading ? "Validating…" : workspace ? "Open another" : "Open databank"}
            </button>
          )}
        </header>

        {error && (
          <div className="error-banner" role="alert">
            <strong>Operation failed</strong>
            <span>{error}</span>
          </div>
        )}

        {activeWorkspace === "Home" ? (
          <HomeWorkspace
            hasDatabank={workspace !== null}
            onNavigate={setActiveWorkspace}
            onOpen={openDatabank}
          />
        ) : activeWorkspace === "Data Lab" ? (
          <DataLabWorkspace onError={setError} onUse={useDataInDiscover} />
        ) : activeWorkspace === "Discover" ? (
          <DiscoverWorkspace
            onError={setError}
            onOpenDatabank={openDatabankPath}
            preset={discoverPreset}
          />
        ) : activeWorkspace === "Parity Lab" ? (
          <ParityWorkspace
            exportPreset={exportPreset}
            judgePreset={judgePreset}
            onError={setError}
          />
        ) : activeWorkspace === "Portfolio" ? (
          <PortfolioWorkspace onError={setError} workspace={workspace} />
        ) : activeWorkspace === "Vault" ? (
          <VaultWorkspace onError={setError} />
        ) : activeWorkspace === "Deploy" ? (
          <DeployWorkspace onError={setError} />
        ) : activeWorkspace === "Databank" ? (!workspace ? (
          <EmptyState onOpen={openDatabank} loading={loading} />
        ) : (
          <div className="workspace-content">
            <section className="artifact-strip">
              <div>
                <span className="file-kicker">Validated evolve artifact</span>
                <strong>{workspace.sourcePath.split("/").at(-1)}</strong>
                <small title={workspace.sourcePath}>{workspace.sourcePath}</small>
              </div>
              <dl>
                <div>
                  <dt>Run</dt>
                  <dd>{workspace.runId}</dd>
                </div>
                <div>
                  <dt>Grammar</dt>
                  <dd>{workspace.grammarVersion}</dd>
                </div>
                <div>
                  <dt>Data quality</dt>
                  <dd className={`quality-${workspace.qualityGrade}`}>
                    {workspace.qualityGrade} · {workspace.qualityScore}
                  </dd>
                </div>
                <div>
                  <dt>Grade</dt>
                  <dd>{workspace.researchGrade ? "research (H1 scout)" : workspace.m1FidelityVerified ? "M1 verified" : "promotion"}</dd>
                </div>
              </dl>
            </section>

            <section className="kpi-grid" aria-label="Databank summary">
              <Kpi
                label="Niches illuminated"
                value={`${workspace.coverage} / ${workspace.totalNiches}`}
                note={`${((workspace.coverage / workspace.totalNiches) * 100).toFixed(1)}% coverage`}
              />
              <Kpi
                label="QD score"
                value={formatNumber(workspace.qdScore, 2)}
                note="Positive evidence mass"
              />
              <Kpi
                label="Evaluations touched"
                value={formatNumber(workspace.selectionBias.evaluationCount)}
                note={`${workspace.completedGenerations} generations completed`}
              />
              <Kpi
                label="Clone + correlation rejects"
                value={formatNumber(
                  workspace.rejections.clone + workspace.rejections.correlated,
                )}
                note={`${formatNumber(workspace.rejections.oos1)} OOS1 pick fails · ${formatNumber(workspace.rejections.total)} total`}
              />
            </section>

            <section
              className={`bias-banner ${selectionBiasTone(workspace.selectionBias.level)}`}
            >
              <div>
                <span>Selection-bias record</span>
                <strong>
                  {formatNumber(workspace.selectionBias.evaluationCount)} evaluations
                </strong>
              </div>
              <p>{workspace.selectionBias.message}</p>
            </section>

            {workspace.researchGrade && (
              <FidelityDemoPanel
                onError={setError}
                onLoaded={openDatabankPath}
                workspace={workspace}
              />
            )}

            <div className="main-grid">
              <div className="primary-column">
                <section className="panel coverage-panel">
                  <div className="panel-heading">
                    <div>
                      <p className="eyebrow">Behavioral map</p>
                      <h2>Niche coverage</h2>
                    </div>
                    <div className="heat-legend">
                      <span>Empty</span>
                      <i className="heat low" />
                      <i className="heat medium" />
                      <i className="heat high" />
                      <span>Evidence</span>
                    </div>
                  </div>
                  <div className="family-grids">
                    {workspace.families.map((item) => (
                      <CoverageGrid
                        family={item}
                        key={item.family}
                        onSelect={selectElite}
                      />
                    ))}
                  </div>
                  <p className="axis-note">
                    Columns combine trade frequency, hold time and drawdown; rows combine
                    win rate and long/short skew. Hover a cell for its exact niche.
                  </p>
                </section>

                <ClusterView
                  rows={workspace.elites}
                  selected={detail?.fingerprint ?? null}
                  onSelect={selectElite}
                />

                <EquityCurvePanel
                  rows={workspace.elites.filter((row) =>
                    batchSelection.size > 0
                      ? batchSelection.has(row.fingerprint)
                      : row.fingerprint === detail?.fingerprint,
                  )}
                  initialBalance={workspace.initialBalance}
                />

                <section className="panel elites-panel">
                  <div className="panel-heading table-heading">
                    <div>
                      <p className="eyebrow">Archive champions</p>
                      <h2>{filtered.length} elites</h2>
                    </div>
                    <div className="table-controls">
                      <input
                        aria-label="Search elites"
                        onChange={(event) => setQuery(event.target.value)}
                        placeholder="Search ID or fingerprint"
                        value={query}
                      />
                      <select
                        aria-label="Filter family"
                        onChange={(event) => setFamily(event.target.value)}
                        value={family}
                      >
                        <option value="all">All families</option>
                        {workspace.families.map((item) => (
                          <option key={item.family} value={item.family}>
                            {item.family}
                          </option>
                        ))}
                      </select>
                      <select
                        aria-label="Sort elites"
                        onChange={(event) => setSort(event.target.value as EliteSort)}
                        value={sort}
                      >
                        <option value="evidence">Evidence ↓</option>
                        <option value="novelty">Novelty ↓</option>
                        <option value="trades">Trades ↓</option>
                        <option value="drawdown">Drawdown ↑</option>
                        <option value="returnDrawdown">Return / DD ↓</option>
                        <option value="sharpe">Sharpe ↓</option>
                        <option value="family">Family A–Z</option>
                        <option value="grade">Grade A–Z</option>
                      </select>
                    </div>
                    <div className="batch-controls">
                      <label>
                        Top
                        <input
                          aria-label="Batch selection count"
                          min={1}
                          onChange={(event) => setBatchSize(Number(event.target.value) || 1)}
                          type="number"
                          value={batchSize}
                        />
                      </label>
                      <button className="secondary" onClick={selectTopElites}>
                        Select top
                      </button>
                      <button
                        className="secondary"
                        disabled={batchSelection.size === 0}
                        onClick={() => setBatchSelection(new Set())}
                      >
                        Clear
                      </button>
                      <button
                        className="primary"
                        disabled={batchBusy || batchSelection.size === 0}
                        onClick={() => void exportSelectedElites()}
                      >
                        {batchBusy ? "Exporting…" : `Export ${batchSelection.size} selected`}
                      </button>
                    </div>
                  </div>
                  {batchMessage && <p className="batch-message">{batchMessage}</p>}
                  <EliteTable
                    rows={filtered}
                    selected={detail?.fingerprint ?? null}
                    batchSelection={batchSelection}
                    onSelect={selectElite}
                    onToggle={toggleBatchElite}
                  />
                </section>
              </div>

              <EliteInspector
                detail={detail}
                loading={detailLoading}
                onError={setError}
                tab={detailTab}
                onTab={setDetailTab}
                onParity={continueToParity}
              />
            </div>
          </div>
        )) : null}
      </main>
    </div>
  );
}

function HomeWorkspace({
  hasDatabank,
  onNavigate,
  onOpen,
}: {
  hasDatabank: boolean;
  onNavigate: (workspace: WorkspaceName) => void;
  onOpen: () => void;
}) {
  return (
    <div className="landing-content">
      <section className="hero-panel">
        <div>
          <p className="eyebrow">Local-first research</p>
          <h2>From broker-bound OHLC to an auditable strategy archive</h2>
          <p>
            Inspect the source, run deterministic quality-diversity discovery, then
            interrogate every occupied niche. Inputs and outputs stay on this machine.
          </p>
          <div className="hero-actions">
            <button className="primary" onClick={() => onNavigate("Data Lab")}>Inspect data</button>
            <button className="secondary" onClick={() => onNavigate("Discover")}>Start discovery</button>
            <button className="secondary" onClick={onOpen}>{hasDatabank ? "Open another archive" : "Open archive"}</button>
          </div>
        </div>
        <div className="truth-chain" aria-label="QuantForge truth chain">
          <span>OHLC + metadata</span><i />
          <span>Broker profile</span><i />
          <span>Deterministic search</span><i />
          <span>Immutable databank</span>
        </div>
      </section>
      <section className="workflow-grid">
        <WorkflowCard
          action="Open Data Lab"
          description="Parse MT5 exports, bind timezone and broker metadata, and surface quality defects before evaluation."
          eyebrow="01 / Establish truth"
          onClick={() => onNavigate("Data Lab")}
          title="Data Lab"
        />
        <WorkflowCard
          action="Configure Discover"
          description="Generate four strategy families through a seeded, parallel MAP-Elites search with explicit gates and costs."
          eyebrow="02 / Illuminate"
          onClick={() => onNavigate("Discover")}
          title="Discover"
        />
        <WorkflowCard
          action={hasDatabank ? "View loaded archive" : "Choose archive"}
          description="Inspect elites with IS / OOS1 / OOS2 equity on one chart. Pipeline: IS robustness → OOS1 expectancy ≥ 0.7× IS → databank. OOS2 display-only."
          eyebrow="03 / Databank lab"
          onClick={hasDatabank ? () => onNavigate("Databank") : onOpen}
          title="Databank"
        />
        <WorkflowCard
          action="Open Parity Lab"
          description="Replay decisions on M1, generate the guarded MT5 expert, and compare trade-level and equity-path output with Strategy Tester."
          eyebrow="04 / Reconcile"
          onClick={() => onNavigate("Parity Lab")}
          title="Parity Lab"
        />
        <WorkflowCard
          action="Open Vault"
          description="Admit only complete evidence chains, inspect immutable Certified entries, and hand a verified candidate to deployment packaging."
          eyebrow="05 / Certify"
          onClick={() => onNavigate("Vault")}
          title="Vault + Deploy"
        />
      </section>
      <section className="stage-note">
        <span className="status-light" />
        <p><strong>All required workspaces are functional:</strong> data, discovery, robustness, parity, certification, portfolio and guarded deployment.</p>
        <p>Every promotion artifact is local, immutable and content-bound.</p>
      </section>
    </div>
  );
}

function WorkflowCard({
  action,
  description,
  eyebrow,
  onClick,
  title,
}: {
  action: string;
  description: string;
  eyebrow: string;
  onClick: () => void;
  title: string;
}) {
  return (
    <article className="workflow-card">
      <p className="eyebrow">{eyebrow}</p>
      <h3>{title}</h3>
      <p>{description}</p>
      <button className="text-action" onClick={onClick}>{action} →</button>
    </article>
  );
}

function DataLabWorkspace({
  onError,
  onUse,
}: {
  onError: (message: string | null) => void;
  onUse: (report: DataLabView) => void;
}) {
  const [dataPath, setDataPath] = useState("");
  const [metadataPath, setMetadataPath] = useState("");
  const [sourceTimezone, setSourceTimezone] = useState("Etc/UTC");
  const [brokerPath, setBrokerPath] = useState("");
  const [report, setReport] = useState<DataLabView | null>(null);
  const [busy, setBusy] = useState(false);

  async function analyze() {
    setBusy(true);
    setReport(null);
    onError(null);
    try {
      const loaded = await inspectData({
        dataPath,
        metadataPath: metadataPath || null,
        sourceTimezone: metadataPath ? null : sourceTimezone || null,
        brokerPath: brokerPath || null,
      });
      setReport(loaded);
    } catch (reason) {
      onError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="tool-content">
      <section className="panel setup-panel">
        <div className="panel-heading">
          <div><p className="eyebrow">Source binding</p><h2>Inspect an MT5 OHLC export</h2></div>
          <span className="read-only-badge">Read-only</span>
        </div>
        <div className="form-stack">
          <SymbolSelect
            onError={onError}
            onSelect={(symbol) => {
              setDataPath(symbol.dataPath);
              setMetadataPath(symbol.metadataPath);
              setSourceTimezone("Etc/UTC");
              setBrokerPath(symbol.brokerPath);
            }}
            selectedDataPath={dataPath}
          />
          <details className="advanced-settings">
            <summary>Bound paths (auto-filled)</summary>
            <div className="form-stack compact">
              <label className="field-row"><span>H1</span><code>{dataPath || "—"}</code></label>
              <label className="field-row"><span>Metadata</span><code>{metadataPath || "—"}</code></label>
              <label className="field-row"><span>Broker</span><code>{brokerPath || "—"}</code></label>
            </div>
          </details>
        </div>
        <div className="form-footer">
          <p>Metadata and a manual timezone are mutually exclusive. Adding a broker profile also verifies symbol, timezone and currency bindings.</p>
          <button className="primary" disabled={busy || !dataPath || (!metadataPath && !sourceTimezone)} onClick={analyze}>
            {busy ? "Parsing and hashing…" : "Run diagnostics"}
          </button>
        </div>
      </section>

      {report ? (
        <DataReport report={report} onUse={() => onUse(report)} />
      ) : (
        <section className="panel waiting-panel">
          <div className="data-glyph"><i /><i /><i /><i /><i /></div>
          <h3>No source inspected yet</h3>
          <p>The Rust parser will normalize ordering, remove exact duplicate timestamps, calculate an immutable content hash and grade the source.</p>
        </section>
      )}
    </div>
  );
}

function DataReport({ report, onUse }: { report: DataLabView; onUse: () => void }) {
  const q = report.quality;
  const issues = [
    ["Missing bars", q.missingBarEstimate],
    ["Gap events", q.gapEvents],
    ["OHLC violations", q.ohlcViolations],
    ["Spike bars", q.spikeBars],
    ["Zero-range bars", q.zeroRangeBars],
    ["Weekend bars", q.weekendBars],
  ] as const;
  return (
    <div className="report-stack">
      <section className="quality-hero panel">
        <div className={`quality-orb quality-${q.grade}`}><strong>{q.score}</strong><span>{q.grade}</span></div>
        <div>
          <p className="eyebrow">Quality assessment</p>
          <h2>{report.discoverReady ? "Bound and ready for discovery" : q.grade === "fail" ? "Source is blocked" : "Broker binding still required"}</h2>
          <p>{formatNumber(report.bars)} normalized bars from {new Date(report.firstTimestampMs).toLocaleString()} to {new Date(report.lastTimestampMs).toLocaleString()}.</p>
        </div>
        <button className="primary" disabled={!report.discoverReady} onClick={onUse}>Use in Discover</button>
      </section>
      <section className="kpi-grid diagnostics-grid">
        <Kpi label="Rows read" value={formatNumber(report.sourceRows)} note={`${formatNumber(report.duplicateRowsRemoved)} exact duplicates removed`} />
        <Kpi label="Interval" value={q.expectedIntervalSeconds ? `${q.expectedIntervalSeconds}s` : "Unknown"} note={`Delimiter ${JSON.stringify(report.delimiter)}`} />
        <Kpi label="Symbol / timeframe" value={`${report.symbol ?? "—"} · ${report.timeframe ?? "—"}`} note={report.brokerProfile ?? "No broker profile bound"} />
        <Kpi label="Ordering" value={report.inputWasSorted ? "Sorted" : "Normalized"} note={report.sourceTimezone} />
      </section>
      <section className="report-grid">
        <article className="panel issue-panel">
          <div className="panel-heading"><div><p className="eyebrow">Diagnostics</p><h2>Observed defects</h2></div></div>
          <div className="issue-list">
            {issues.map(([label, value]) => <div key={label}><span>{label}</span><strong className={value > 0 ? "warning" : "positive"}>{formatNumber(value)}</strong></div>)}
          </div>
        </article>
        <article className="panel hash-panel">
          <div className="panel-heading"><div><p className="eyebrow">Immutable identity</p><h2>Content bindings</h2></div></div>
          <HashLine label="Data" value={report.dataHash} />
          <HashLine label="Metadata" value={report.metadataHash} />
          <HashLine label="Broker" value={report.brokerSpecHash} />
        </article>
      </section>
    </div>
  );
}

function HashLine({ label, value }: { label: string; value: string | null }) {
  return <div className="hash-line"><span>{label}</span><code>{value ?? "not bound"}</code></div>;
}

function DiscoverWorkspace({
  onError,
  onOpenDatabank,
  preset,
}: {
  onError: (message: string | null) => void;
  onOpenDatabank: (path: string) => void;
  preset: Partial<DiscoverRequest>;
}) {
  const [form, setForm] = useState<DiscoverRequest>(() => ({
    mode: "new",
    dataPath: "",
    metadataPath: null,
    sourceTimezone: "Etc/UTC",
    m1DataPath: "",
    m1MetadataPath: null,
    m1SourceTimezone: "Etc/UTC",
    brokerPath: "",
    databankPath: "",
    generations: 0,
    runUntilStopped: true,
    initialCandidates: 500,
    batchSize: 200,
    correlationThreshold: 0.85,
    noveltyWeight: 10,
    seed: 42,
    minimumTrades: 10,
    maximumDrawdownPercent: 40,
    minimumReturnPercent: 0,
    minimumProfitFactor: 1,
    minimumReturnDrawdown: 0,
    depositMinimumTrades: 20,
    depositMaximumDrawdownPercent: 30,
    depositMinimumReturnPercent: 0,
    depositMinimumProfitFactor: 1,
    depositMinimumReturnDrawdown: 0,
    minimumM1ReturnRetention: 0.95,
    oos1ExpectancyRetention: 0.7,
    requireM1Precision: false,
    simpleExits: true,
    flattenAt22: false,
    maxOneEntryPerDay: true,
    mutateAfterElites: 300,
    randomFillFraction: 0.4,
    workerThreads: 11,
    requireM1Robustness: true,
    robustnessFolds: 3,
    robustnessMonteCarloTrials: 250,
    robustnessNeighborhoodSamples: 8,
    commissionPerLotRoundTurn: 7,
    slippagePointsPerSide: 0,
    fallbackSpreadPoints: null,
    maxSpreadPoints: null,
    initialBalance: 100000,
    promotionSplit: true,
    validationFraction: 0.2,
    sealedFraction: 0.2,
    ...preset,
  }));
  const [job, setJob] = useState<DiscoverJobView | null>(null);
  const [busy, setBusy] = useState(false);
  const active = job?.status === "running" || job?.status === "paused";

  useEffect(() => {
    void getDiscoverJob().then(setJob).catch((reason) => onError(String(reason)));
  }, []);

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => {
      void getDiscoverJob().then(setJob).catch((reason) => onError(String(reason)));
    }, 500);
    return () => window.clearInterval(timer);
  }, [active]);

  function update<K extends keyof DiscoverRequest>(key: K, value: DiscoverRequest[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }

  async function chooseOutput() {
    return form.mode === "new" ? chooseNewDatabank() : chooseDatabank();
  }

  async function start() {
    setBusy(true);
    onError(null);
    try {
      const sourceBoundForm = bindDiscoverTimezone({
        ...form,
        promotionSplit: true,
      });
      const request = form.mode === "continue"
        ? {
            ...sourceBoundForm,
            initialCandidates: null,
            batchSize: null,
            correlationThreshold: null,
            noveltyWeight: null,
            seed: null,
            minimumTrades: null,
            maximumDrawdownPercent: null,
            minimumReturnPercent: null,
            minimumProfitFactor: null,
            minimumReturnDrawdown: null,
            depositMinimumTrades: null,
            depositMaximumDrawdownPercent: null,
            depositMinimumReturnPercent: null,
            depositMinimumProfitFactor: null,
            depositMinimumReturnDrawdown: null,
            minimumM1ReturnRetention: null,
            oos1ExpectancyRetention: null,
            requireM1Precision: null,
            simpleExits: null,
            flattenAt22: null,
            maxOneEntryPerDay: null,
            mutateAfterElites: null,
            randomFillFraction: null,
            workerThreads: null,
            requireM1Robustness: null,
            robustnessFolds: null,
            robustnessMonteCarloTrials: null,
            robustnessNeighborhoodSamples: null,
            commissionPerLotRoundTurn: null,
            slippagePointsPerSide: null,
            fallbackSpreadPoints: null,
            maxSpreadPoints: null,
            initialBalance: null,
            promotionSplit: null,
            validationFraction: null,
            sealedFraction: null,
          }
        : sourceBoundForm;
      setJob(await startDiscover(request));
    } catch (reason) {
      onError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  async function control(action: () => Promise<DiscoverJobView>) {
    onError(null);
    try {
      setJob(await action());
    } catch (reason) {
      onError(String(reason));
    }
  }

  const progress = discoverProgress(
    job?.completedGenerations ?? 0,
    job?.requestedGenerations ?? 0,
    job?.runUntilStopped ?? form.runUntilStopped ?? true,
  );
  const progressLabel = discoverProgressLabel(
    job?.completedGenerations ?? 0,
    job?.requestedGenerations ?? 0,
    job?.runUntilStopped ?? form.runUntilStopped ?? true,
  );
  const lookedAt = job?.evaluationCount ?? 0;
  const accepted = job?.acceptedTotal ?? 0;
  const rejected = job?.rejectedTotal ?? 0;
  return (
    <div className="discover-layout">
      <section className="panel discover-form">
        <div className="panel-heading">
          <div><p className="eyebrow">Job contract</p><h2>Configure deterministic evolution</h2></div>
          <div className="mode-toggle">
            <button className={form.mode === "new" ? "active" : ""} disabled={active} onClick={() => update("mode", "new")}>New</button>
            <button className={form.mode === "continue" ? "active" : ""} disabled={active} onClick={() => update("mode", "continue")}>Continue</button>
          </div>
        </div>
        <fieldset disabled={active || busy}>
          <div className="form-stack compact">
            <SymbolSelect
              disabled={active || busy}
              onError={onError}
              onSelect={(symbol) => {
                setForm((current) => ({
                  ...current,
                  dataPath: symbol.dataPath,
                  metadataPath: symbol.metadataPath,
                  sourceTimezone: null,
                  m1DataPath: symbol.m1DataPath,
                  m1MetadataPath: symbol.m1MetadataPath,
                  m1SourceTimezone: null,
                  brokerPath: symbol.brokerPath,
                  databankPath:
                    current.mode === "new" && !current.databankPath
                      ? symbol.defaultDatabankPath
                      : current.databankPath || symbol.defaultDatabankPath,
                }));
              }}
              selectedDataPath={form.dataPath}
            />
            <PathField
              label={form.mode === "new" ? "New databank" : "Existing databank"}
              path={form.databankPath}
              choose={chooseOutput}
              onChange={(value) => update("databankPath", value)}
              required
            />
            <details className="advanced-settings">
              <summary>Bound paths (auto-filled from symbol)</summary>
              <div className="form-stack compact">
                <label className="field-row"><span>H1</span><code>{form.dataPath || "—"}</code></label>
                <label className="field-row"><span>H1 metadata</span><code>{form.metadataPath || "—"}</code></label>
                <label className="field-row"><span>M1</span><code>{form.m1DataPath || "—"}</code></label>
                <label className="field-row"><span>M1 metadata</span><code>{form.m1MetadataPath || "—"}</code></label>
                <label className="field-row"><span>Broker</span><code>{form.brokerPath || "—"}</code></label>
              </div>
            </details>
          </div>
          <div className="numeric-grid core-numbers">
            <label className="check-field discover-split" style={{ gridColumn: "1 / -1" }}>
              <input
                type="checkbox"
                checked={form.runUntilStopped ?? true}
                onChange={(event) => update("runUntilStopped", event.target.checked)}
              />
              <span>Run until stopped (SQX-style continuous bank growth)</span>
            </label>
            <NumberField
              label={form.runUntilStopped ? "Soft generation budget (0 = none)" : "Generations"}
              value={form.generations}
              onChange={(value) => update("generations", value ?? 0)}
              min={0}
            />
            {form.mode === "new" && <>
              <NumberField label="Initial candidates" value={form.initialCandidates} onChange={(value) => update("initialCandidates", value)} min={1} />
              <NumberField label="Batch / generation" value={form.batchSize} onChange={(value) => update("batchSize", value)} min={1} />
              <NumberField label="Seed" value={form.seed} onChange={(value) => update("seed", value)} min={0} />
              <NumberField label="Worker threads (0 = all)" value={form.workerThreads} onChange={(value) => update("workerThreads", value)} min={0} />
              <NumberField label="Breed after pot elites" value={form.mutateAfterElites} onChange={(value) => update("mutateAfterElites", value)} min={0} />
              <NumberField label="Random fill fraction" value={form.randomFillFraction} onChange={(value) => update("randomFillFraction", value)} step={0.05} min={0} />
              <NumberField label="OOS1 reserve" value={form.validationFraction} onChange={(value) => update("validationFraction", value)} step={0.05} />
              <NumberField label="OOS2 display reserve" value={form.sealedFraction} onChange={(value) => update("sealedFraction", value)} step={0.05} />
            </>}
          </div>
          {form.mode === "new" ? (
            <>
              <details className="advanced-settings" open>
                <summary>Scout gates — random search screen</summary>
                <p className="immutable-note">Cheap H1 filter. Keep these loose so the databank pot can fill before breeding.</p>
                <div className="numeric-grid">
                  <NumberField label="Minimum trades" value={form.minimumTrades} onChange={(value) => update("minimumTrades", value)} min={0} />
                  <NumberField label="Maximum drawdown %" value={form.maximumDrawdownPercent} onChange={(value) => update("maximumDrawdownPercent", value)} step={0.1} />
                  <NumberField label="Minimum return %" value={form.minimumReturnPercent} onChange={(value) => update("minimumReturnPercent", value)} step={0.1} />
                  <NumberField label="Minimum profit factor" value={form.minimumProfitFactor} onChange={(value) => update("minimumProfitFactor", value)} step={0.01} />
                  <NumberField label="Minimum return / DD" value={form.minimumReturnDrawdown} onChange={(value) => update("minimumReturnDrawdown", value)} min={0} step={0.05} />
                </div>
              </details>
              <details className="advanced-settings" open>
                <summary>Deposit gates — enter the initial pot</summary>
                <p className="immutable-note">H1 metrics to join the breeding pot. Databank needs M1 IS robustness then OOS1 ≥0.7×.</p>
                <div className="numeric-grid">
                  <NumberField label="Minimum trades" value={form.depositMinimumTrades} onChange={(value) => update("depositMinimumTrades", value)} min={0} />
                  <NumberField label="Maximum drawdown %" value={form.depositMaximumDrawdownPercent} onChange={(value) => update("depositMaximumDrawdownPercent", value)} step={0.1} />
                  <NumberField label="Minimum return %" value={form.depositMinimumReturnPercent} onChange={(value) => update("depositMinimumReturnPercent", value)} step={0.1} />
                  <NumberField label="Minimum profit factor" value={form.depositMinimumProfitFactor} onChange={(value) => update("depositMinimumProfitFactor", value)} step={0.01} />
                  <NumberField label="Minimum return / DD" value={form.depositMinimumReturnDrawdown} onChange={(value) => update("depositMinimumReturnDrawdown", value)} min={0} step={0.05} />
                </div>
              </details>
              <details className="advanced-settings" open>
                <summary>M1 robustness — on IS, before OOS1</summary>
                <p className="immutable-note">SQX-style: M1 WFO/MC/±10% on IS only. OOS1 0.7× expectancy retention runs after. Pot fills on deposit; databank needs robustness + OOS1.</p>
                <label className="check-field discover-split"><input type="checkbox" checked={form.requireM1Robustness ?? true} onChange={(event) => update("requireM1Robustness", event.target.checked)} /><span>Require M1 WFO + MC + param neighborhood before deposit</span></label>
                <div className="numeric-grid">
                  <NumberField label="WFO folds" value={form.robustnessFolds} onChange={(value) => update("robustnessFolds", value)} min={2} />
                  <NumberField label="MC trials" value={form.robustnessMonteCarloTrials} onChange={(value) => update("robustnessMonteCarloTrials", value)} min={1} />
                  <NumberField label="Param samples (±10%)" value={form.robustnessNeighborhoodSamples} onChange={(value) => update("robustnessNeighborhoodSamples", value)} min={1} />
                </div>
              </details>
              <details className="advanced-settings">
                <summary>Diversity and cost model</summary>
                <div className="numeric-grid">
                  <NumberField label="Correlation ceiling" value={form.correlationThreshold} onChange={(value) => update("correlationThreshold", value)} step={0.01} />
                  <NumberField label="Novelty weight" value={form.noveltyWeight} onChange={(value) => update("noveltyWeight", value)} step={0.1} />
                  <NumberField label="Minimum M1 return retention" value={form.minimumM1ReturnRetention} onChange={(value) => update("minimumM1ReturnRetention", value)} step={0.01} />
                  <NumberField label="OOS1 expectancy retention" value={form.oos1ExpectancyRetention} onChange={(value) => update("oos1ExpectancyRetention", value)} step={0.05} />
                  <NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value)} step={0.01} />
                  <NumberField label="Slippage points / side" value={form.slippagePointsPerSide} onChange={(value) => update("slippagePointsPerSide", value)} step={0.1} />
                  <NumberField label="Fallback spread points" value={form.fallbackSpreadPoints} onChange={(value) => update("fallbackSpreadPoints", value)} step={0.1} optional />
                  <NumberField label="Maximum spread points" value={form.maxSpreadPoints} onChange={(value) => update("maxSpreadPoints", value)} step={0.1} optional />
                  <NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value)} min={1} />
                </div>
              </details>
            </>
          ) : (
            <p className="immutable-note">Continuation loads the grammar, seed, gates and cost model from the verified databank. Only the generation count can change.</p>
          )}
          {form.mode === "new" && <>
            <p className="immutable-note">SQX-style default: H1 Selected-TF scout (fast). Turn on M1 precision only if you want in-loop fidelity. OOS1 pick still applies. OOS2 never used in Discover.</p>
            <label className="check-field discover-split"><input type="checkbox" checked={!(form.requireM1Precision ?? false)} onChange={(event) => update("requireM1Precision", !event.target.checked)} /><span>H1 scout only (SQX Selected TF — defer M1 to Fidelity demo)</span></label>
            <label className="check-field discover-split"><input type="checkbox" checked={form.simpleExits ?? true} onChange={(event) => update("simpleExits", event.target.checked)} /><span>Simple exits (market + SL/TP + max 16 bars; no trail/BE/stop-limit)</span></label>
            <label className="check-field discover-split"><input type="checkbox" checked={form.maxOneEntryPerDay ?? true} onChange={(event) => update("maxOneEntryPerDay", event.target.checked)} /><span>Max one signal/day (first market or pending locks the day; entries only 02:00–19:00 broker)</span></label>
            <label className="check-field discover-split"><input type="checkbox" checked={form.flattenAt22 ?? false} onChange={(event) => update("flattenAt22", event.target.checked)} /><span>Close positions and cancel pending orders at 22:00 broker time</span></label>
          </>}
        </fieldset>
        <div className="form-footer">
          <p>Pipeline: H1 scout → deposit → initial pot → M1 robustness on IS → OOS1 (≥0.7× IS expectancy) → databank. Expectancy shown in R ($1,000 risk).</p>
          <button className="primary" disabled={active || busy || !form.dataPath || !form.m1DataPath || !form.brokerPath || !form.databankPath || (!(form.runUntilStopped ?? true) && form.generations < 1)} onClick={start}>{busy ? "Starting…" : form.mode === "new" ? "Start discovery" : "Continue discovery"}</button>
        </div>
      </section>

      <section className={`panel job-panel job-${job?.status ?? "idle"}`}>
        <div className="panel-heading"><div><p className="eyebrow">Live worker</p><h2>{job?.phase ?? "No job loaded"}</h2></div><span className="job-status">{job?.status ?? "idle"}</span></div>
        <div className="job-body">
          <p className="job-message">{job?.message ?? "Configure a search to start the Rust discovery worker."}</p>
          <div className="progress-track"><i style={{ width: `${progress}%` }} /></div>
          <div className="progress-label"><span>{progressLabel}</span><strong>{(job?.runUntilStopped && (job?.requestedGenerations ?? 0) <= 0) ? "∞" : `${progress.toFixed(0)}%`}</strong></div>
          <div className="job-kpis">
            <Kpi label="Looked at" value={formatNumber(lookedAt)} note="Candidates evaluated" />
            <Kpi
              label="Initial pot"
              value={formatNumber(job?.potElites ?? 0)}
              note={`${formatNumber(job?.potNewNiches ?? 0)} new · ${job?.breedingActive ? "breeding" : `fill → ${job?.mutateAfterElites ?? 300}`}`}
            />
            <Kpi
              label="Databank"
              value={formatNumber(job?.databankElites ?? job?.coverage ?? 0)}
              note="Passed M1 + WFO + MC + params"
            />
            <Kpi label="Accepted" value={formatNumber(accepted)} note="Pot + databank events" />
            <Kpi label="Rejected" value={formatNumber(rejected)} note="Did not stay" />
            <Kpi label="Evals / hour" value={formatNumber(job?.evaluationsPerHour ?? 0, 0)} note={`${formatNumber(job?.acceptsPerHour ?? 0, 1)} accepts/hr · ${job?.workerThreads ?? "—"} threads`} />
            <Kpi label="QD score" value={formatNumber(job?.qdScore ?? 0, 2)} note="Databank evidence mass" />
          </div>
          <div className="reject-funnel">
            <p className="eyebrow">Reject reasons</p>
            <ul>
              <li><span>Scout gate</span><strong>{formatNumber(job?.rejectedGate ?? 0)}</strong></li>
              <li><span>Deposit gate</span><strong>{formatNumber(job?.rejectedDepositGate ?? 0)}</strong></li>
              <li><span>OOS1 pick</span><strong>{formatNumber(job?.rejectedOos1 ?? 0)}</strong></li>
              <li><span>M1 fidelity</span><strong>{formatNumber(job?.rejectedM1Fidelity ?? 0)}</strong></li>
              <li><span>Walk-forward</span><strong>{formatNumber(job?.rejectedWalkForward ?? 0)}</strong></li>
              <li><span>Monte Carlo</span><strong>{formatNumber(job?.rejectedMonteCarlo ?? 0)}</strong></li>
              <li><span>Param ±10%</span><strong>{formatNumber(job?.rejectedParamNeighborhood ?? 0)}</strong></li>
              <li><span>M1 precision</span><strong>{formatNumber(job?.rejectedPrecision ?? 0)}</strong></li>
              <li><span>Clone</span><strong>{formatNumber(job?.rejectedClone ?? 0)}</strong></li>
              <li><span>Correlation</span><strong>{formatNumber(job?.rejectedCorrelated ?? 0)}</strong></li>
              <li><span>Niche not improved</span><strong>{formatNumber(job?.rejectedNicheNotImproved ?? 0)}</strong></li>
              <li><span>Eval error</span><strong>{formatNumber(job?.rejectedEvaluation ?? 0)}</strong></li>
            </ul>
            {(job?.bestIsExpectancy != null || job?.bestOos1Expectancy != null) && (
              <p className="funnel-best">
                Best banked expectancy (R) — IS {job?.bestIsExpectancy != null ? formatNumber(job.bestIsExpectancy, 2) : "—"}
                {" · "}
                OOS1 {job?.bestOos1Expectancy != null ? formatNumber(job.bestOos1Expectancy, 2) : "—"}
                {" · "}
                $1,000 risk → 0.35R = $350/trade
              </p>
            )}
            {(job?.m1BarsRepaired ?? 0) > 0 && (
              <p className="funnel-best">Aligned {formatNumber(job!.m1BarsRepaired)} H1 bars to M1 OHLC before search.</p>
            )}
            {(job?.topEvaluationErrors?.length ?? 0) > 0 && (
              <div className="eval-errors">
                <p className="eyebrow">Top evaluation errors</p>
                <ul>
                  {job!.topEvaluationErrors.map((entry) => (
                    <li key={entry.message}><code>{entry.message}</code><strong>{formatNumber(entry.count)}</strong></li>
                  ))}
                </ul>
              </div>
            )}
          </div>
          {active && <div className="job-controls">
            {job?.status === "running" ? <button className="secondary" onClick={() => void control(pauseDiscover)}>Pause</button> : <button className="secondary" onClick={() => void control(resumeDiscover)}>Resume</button>}
            <button className="danger" onClick={() => void control(stopDiscover)}>Stop & checkpoint</button>
            <small>Pause and stop take effect at a generation boundary. Elites are written to the databank as soon as they pass.</small>
          </div>}
          {job?.status === "completed" && job.outputPath && <div className="completion-card"><div><span>Checkpoint</span><code>{job.outputPath}</code></div><button className="primary" onClick={() => onOpenDatabank(job.outputPath!)}>Open in Databank</button></div>}
          {job?.status === "completed" && !job.outputPath && <div className="job-empty-note">{job.message}</div>}
          {job?.status === "failed" && <div className="job-error">{job.message}</div>}
        </div>
      </section>
    </div>
  );
}

function SymbolSelect({
  disabled = false,
  onError,
  onSelect,
  selectedDataPath,
}: {
  disabled?: boolean;
  onError: (message: string | null) => void;
  onSelect: (symbol: SymbolPack) => void;
  selectedDataPath: string;
}) {
  const [symbols, setSymbols] = useState<SymbolPack[]>([]);
  const [selected, setSelected] = useState("");
  useEffect(() => {
    void listSymbols()
      .then((items) => {
        setSymbols(items);
        if (!selectedDataPath && items[0]) {
          setSelected(items[0].symbol);
          onSelect(items[0]);
        } else {
          const match = items.find((item) => item.dataPath === selectedDataPath);
          if (match) setSelected(match.symbol);
        }
      })
      .catch((reason) => onError(String(reason)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <label className="field-row">
      <span>Symbol <small>required</small></span>
      <select
        disabled={disabled || symbols.length === 0}
        value={selected}
        onChange={(event) => {
          const next = symbols.find((item) => item.symbol === event.target.value);
          setSelected(event.target.value);
          if (next) onSelect(next);
        }}
      >
        <option value="" disabled>{symbols.length ? "Choose symbol…" : "Scanning data pack…"}</option>
        {symbols.map((item) => (
          <option key={item.symbol} value={item.symbol}>{item.symbol}</option>
        ))}
      </select>
    </label>
  );
}

function ParityWorkspace({
  exportPreset,
  judgePreset,
  onError,
}: {
  exportPreset: Partial<ExportRequest>;
  judgePreset: Partial<JudgeRequest>;
  onError: (message: string | null) => void;
}) {
  const [tab, setTab] = useState<"judge" | "export" | "compare" | "indicators">("judge");
  return <div className="wide-tool-content">
    <div className="workspace-tabs"><button className={tab === "judge" ? "active" : ""} onClick={() => setTab("judge")}>1 · M1 Judge</button><button className={tab === "export" ? "active" : ""} onClick={() => setTab("export")}>2 · EA Export</button><button className={tab === "compare" ? "active" : ""} onClick={() => setTab("compare")}>3 · MT5 Compare</button><button className={tab === "indicators" ? "active" : ""} onClick={() => setTab("indicators")}>4 · Indicators</button></div>
    {tab === "judge" ? <JudgePanel onError={onError} preset={judgePreset} /> : tab === "export" ? <ExportPanel onError={onError} preset={exportPreset} /> : tab === "compare" ? <ParityComparePanel onError={onError} /> : <IndicatorParityPanel onError={onError} />}
  </div>;
}

function JudgePanel({
  onError,
  preset,
}: {
  onError: (message: string | null) => void;
  preset: Partial<JudgeRequest>;
}) {
  const [form, setForm] = useState<JudgeRequest>({ decisionDataPath: "", decisionMetadataPath: null, decisionSourceTimezone: "Etc/UTC", m1DataPath: "", m1MetadataPath: null, m1SourceTimezone: "Etc/UTC", splitPlanPath: null, strategyPath: "", brokerPath: "", outputPath: "", commissionPerLotRoundTurn: 7, slippagePointsPerSide: 0, fallbackSpreadPoints: null, maxSpreadPoints: null, initialBalance: 100000 });
  const [result, setResult] = useState<JudgeView | null>(null); const [busy, setBusy] = useState(false);
  useEffect(() => {
    setForm((current) => ({ ...current, ...preset }));
  }, [preset]);
  function update<K extends keyof JudgeRequest>(key: K, value: JudgeRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await runM1Judge(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content">
    <section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Internal acceptance</p><h2>Replay decision signals through M1 chronology</h2></div></div>
      <div className="form-stack compact">
        <SymbolSelect
          onError={onError}
          onSelect={(symbol) => {
            setForm((current) => ({
              ...current,
              decisionDataPath: symbol.dataPath,
              decisionMetadataPath: symbol.metadataPath,
              decisionSourceTimezone: null,
              m1DataPath: symbol.m1DataPath,
              m1MetadataPath: symbol.m1MetadataPath,
              m1SourceTimezone: null,
              brokerPath: symbol.brokerPath,
            }));
          }}
          selectedDataPath={form.decisionDataPath}
        />
        <PathField label="Split plan (uses validation only)" path={form.splitPlanPath ?? ""} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => update("splitPlanPath", value || null)} />
        <PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required />
        <PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required />
        <PathField label="New Judge artifact" path={form.outputPath} choose={() => chooseOutputJson("Save Judge artifact", "judge.json")} onChange={(value) => update("outputPath", value)} required />
      </div>
      <div className="numeric-grid"><NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value ?? 0)} step={.01} /><NumberField label="Slippage / side" value={form.slippagePointsPerSide} onChange={(value) => update("slippagePointsPerSide", value ?? 0)} step={.1} /><NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value ?? 100000)} min={1} /></div>
      <div className="form-footer"><p>M1 files contain only minutes that received ticks. Gaps pass only when H1 and M1 volumes reconcile exactly; missing tick history remains a hard failure.</p><button className="primary" disabled={busy || !form.decisionDataPath || !form.m1DataPath || !form.strategyPath || !form.brokerPath || !form.outputPath} onClick={run}>{busy ? "Replaying…" : "Run M1 Judge"}</button></div>
    </section>
    {result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">M1 Judge</p><h2>Execution chronology replayed</h2></div><span className="grade-pill">{result.grade}</span></div><div className="job-kpis"><Kpi label="Trades" value={formatNumber(result.trades)} note={`${formatNumber(result.returnPercent, 2)}% return`} /><Kpi label="Drawdown" value={`${formatNumber(result.maximumDrawdownPercent, 2)}%`} note={`PF ${result.profitFactor === null ? "∞" : formatNumber(result.profitFactor, 2)}`} /><Kpi label="Decision / M1 bars" value={`${formatNumber(result.decisionBars)} / ${formatNumber(result.m1Bars)}`} note={`${formatNumber(result.verifiedNoTickMinutes)} verified no-tick min`} /><Kpi label="Managed exits" value={formatNumber(result.partialExits + result.endOfDayFlattens)} note={`${result.breakEvenMoves} BE · ${result.trailingMoves} trail`} /></div><ArtifactPath label="Judge artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No M1 replay yet" copy="Use matching decision-timeframe and M1 exports with their metadata files. The Judge enforces pending-order chronology and every active trade-management gene." />}
  </div>;
}

function ExportPanel({
  onError,
  preset,
}: {
  onError: (message: string | null) => void;
  preset: Partial<ExportRequest>;
}) {
  const [form, setForm] = useState<ExportRequest>({ strategyPath: "", brokerPath: "", outputDirectory: "", expertName: "QuantForgeStrategy", expertDirectory: "QuantForge", timeframe: "H1", magic: 42424242, deviationPoints: 10, maxSpreadPoints: null, slippagePointsPerSide: 0, commissionPerLotRoundTurn: 7, deposit: 100000, currency: "USD", leverage: 100, testerModel: 1 });
  const [result, setResult] = useState<ExportView | null>(null); const [busy, setBusy] = useState(false);
  useEffect(() => {
    setForm((current) => ({ ...current, ...preset }));
  }, [preset]);
  function update<K extends keyof ExportRequest>(key: K, value: ExportRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await exportMql5(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Guarded compiler target</p><h2>Generate MT5 expert + tester pack</h2></div><span className="read-only-badge">Live off</span></div><div className="form-stack compact"><PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required /><PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required /><PathField label="New export directory" path={form.outputDirectory} choose={() => chooseNewDirectory("Create EA export directory", "quantforge-ea-export")} onChange={(value) => update("outputDirectory", value)} required /><label className="field-row"><span>Expert name</span><input value={form.expertName} onChange={(event) => update("expertName", event.target.value)} /></label><label className="field-row"><span>Timeframe</span><input value={form.timeframe} onChange={(event) => update("timeframe", event.target.value.toUpperCase())} /></label></div><div className="numeric-grid"><NumberField label="Magic" value={form.magic} onChange={(value) => update("magic", value ?? 42424242)} min={1} /><NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value ?? 0)} step={.01} /><NumberField label="Deposit" value={form.deposit} onChange={(value) => update("deposit", value ?? 100000)} min={1} /><NumberField label="Leverage" value={form.leverage} onChange={(value) => update("leverage", value ?? 100)} min={1} /></div><div className="form-footer"><p>The generated set file keeps AllowLiveTrading=false. Protective stops and all selected management/order genes are emitted into the EA.</p><button className="primary" disabled={busy || !form.strategyPath || !form.brokerPath || !form.outputDirectory} onClick={run}>{busy ? "Generating…" : "Generate EA pack"}</button></div></section>{result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">EA export</p><h2>{result.symbol} · {result.timeframe}</h2></div><span className="grade-pill">guarded</span></div><ArtifactPath label="MQL5 source" value={result.sourcePath} /><ArtifactPath label="Tester config" value={result.testerPath} /><ArtifactPath label="Evidence" value={result.evidencePath} /><div className="safety-card"><strong>Live trading default</strong><span>{String(result.liveTradingDefault)}</span></div></section> : <WorkspacePrimer title="No EA generated yet" copy="Export writes MQL5 source, guarded settings, Strategy Tester configuration and a source-hash evidence card into one new directory." />}</div>;
}

function ParityComparePanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<ParityRequest>({ referencePath: "", evidencePath: "", mq5Path: "", mt5DealsPath: "", mt5EquityPath: "", mt5MetadataPath: "", outputPath: "", initialBalance: 100000, tradeCountRelative: .1, tradeCountAbsolute: 3, netProfitRelative: .15, maxDrawdownRelative: .15, maxEquityDivergencePercent: 5, tradeTimestampToleranceMs: 0, minimumAlignedTradeFraction: .9 });
  const [result, setResult] = useState<ParityView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof ParityRequest>(key: K, value: ParityRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await compareExternalParity(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">External truth</p><h2>Compare QuantForge with MT5 Strategy Tester</h2></div></div><div className="form-stack compact"><PathField label="Scout reference artifact" path={form.referencePath} choose={() => chooseJsonFile("Choose Scout artifact")} onChange={(value) => update("referencePath", value)} required /><PathField label="Export evidence" path={form.evidencePath} choose={() => chooseJsonFile("Choose export evidence")} onChange={(value) => update("evidencePath", value)} required /><PathField label="Generated MQL5" path={form.mq5Path} choose={chooseMql5File} onChange={(value) => update("mq5Path", value)} required /><PathField label="MT5 deals CSV" path={form.mt5DealsPath} choose={() => chooseTesterCsv("Choose MT5 deals export")} onChange={(value) => update("mt5DealsPath", value)} required /><PathField label="MT5 equity CSV" path={form.mt5EquityPath} choose={() => chooseTesterCsv("Choose MT5 equity export")} onChange={(value) => update("mt5EquityPath", value)} required /><PathField label="MT5 parity metadata" path={form.mt5MetadataPath} choose={() => chooseTesterCsv("Choose MT5 parity metadata")} onChange={(value) => update("mt5MetadataPath", value)} required /><PathField label="New parity artifact" path={form.outputPath} choose={() => chooseOutputJson("Save parity artifact", "parity.json")} onChange={(value) => update("outputPath", value)} required /></div><details className="advanced-settings"><summary>Parity tolerances</summary><div className="numeric-grid"><NumberField label="Trade count relative" value={form.tradeCountRelative} onChange={(value) => update("tradeCountRelative", value ?? .1)} step={.01} /><NumberField label="Trade count absolute" value={form.tradeCountAbsolute} onChange={(value) => update("tradeCountAbsolute", value ?? 3)} min={0} /><NumberField label="Net profit relative" value={form.netProfitRelative} onChange={(value) => update("netProfitRelative", value ?? .15)} step={.01} /><NumberField label="Drawdown relative" value={form.maxDrawdownRelative} onChange={(value) => update("maxDrawdownRelative", value ?? .15)} step={.01} /><NumberField label="Equity divergence %" value={form.maxEquityDivergencePercent} onChange={(value) => update("maxEquityDivergencePercent", value ?? 5)} step={.1} /><NumberField label="Aligned fraction" value={form.minimumAlignedTradeFraction} onChange={(value) => update("minimumAlignedTradeFraction", value ?? .9)} step={.01} /></div></details><div className="form-footer"><p>The source hash and tester metadata must match the evidence card before any comparison is accepted.</p><button className="primary" disabled={busy || Object.entries(form).some(([key, value]) => key.endsWith("Path") && value === "")} onClick={run}>{busy ? "Reconciling…" : "Compare MT5 run"}</button></div></section>{result ? <section className={`panel result-panel result-${result.passed ? "pass" : "fail"}`}><div className="result-hero"><div><p className="eyebrow">External parity</p><h2>{result.passed ? "MT5 parity passed" : "Execution diff exceeds tolerance"}</h2></div><span className="grade-pill">{result.grade}</span></div><div className="job-kpis"><Kpi label="Trades" value={`${result.referenceTrades} / ${result.externalTrades}`} note="QuantForge / MT5" /><Kpi label="Aligned" value={`${result.alignedTrades} / ${result.requiredAlignedTrades}`} note="Required minimum" /><Kpi label="Profit delta" value={`${formatNumber(result.netProfitDeltaRelative * 100, 2)}%`} note="Relative" /><Kpi label="Equity divergence" value={`${formatNumber(result.equityDivergencePercent, 2)}%`} note={result.protectiveOrdersPresent ? "Stops verified" : "Stops missing"} /></div><ArtifactPath label="Parity artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No external run compared" copy="Run the generated EA in MT5 Strategy Tester, then select its deals, equity and metadata exports here. Passing output becomes the external parity gate." />}</div>;
}

function IndicatorParityPanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<IndicatorParityRequest>({ referencePath: "", outputPath: "", warmupRows: 1000, absoluteEpsilon: 1e-10, relativeEpsilon: 1e-9 });
  const [result, setResult] = useState<IndicatorParityView | null>(null); const [busy, setBusy] = useState(false);
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await compareIndicatorParity(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Buffer-level reconciliation</p><h2>Compare Rust indicators with MT5</h2></div></div><div className="form-stack"><PathField label="Indicator probe CSV" path={form.referencePath} choose={() => chooseTesterCsv("Choose MT5 indicator probe export")} onChange={(value) => setForm((current) => ({ ...current, referencePath: value }))} required /><PathField label="New indicator parity artifact" path={form.outputPath} choose={() => chooseOutputJson("Save indicator parity artifact", "indicator-parity.json")} onChange={(value) => setForm((current) => ({ ...current, outputPath: value }))} required /></div><div className="numeric-grid"><NumberField label="Warmup rows" value={form.warmupRows} onChange={(value) => setForm((current) => ({ ...current, warmupRows: value ?? 1000 }))} min={0} /><NumberField label="Absolute epsilon" value={form.absoluteEpsilon} onChange={(value) => setForm((current) => ({ ...current, absoluteEpsilon: value ?? 1e-10 }))} step={1e-10} /><NumberField label="Relative epsilon" value={form.relativeEpsilon} onChange={(value) => setForm((current) => ({ ...current, relativeEpsilon: value ?? 1e-9 }))} step={1e-9} /></div><div className="form-footer"><p>All thirteen supported indicator buffers must match after warmup before certification.</p><button className="primary" disabled={busy || !form.referencePath || !form.outputPath} onClick={run}>{busy ? "Comparing buffers…" : "Compare indicators"}</button></div></section>{result ? <section className={`panel result-panel result-${result.passed ? "pass" : "fail"}`}><div className="result-hero"><div><p className="eyebrow">Indicator parity</p><h2>{result.symbol} · {result.timeframe}</h2></div><span className="grade-pill">{result.passed ? "passed" : "failed"}</span></div><div className="job-kpis"><Kpi label="Fields" value={formatNumber(result.fieldCount)} note="Required buffers" /><Kpi label="Compared rows" value={formatNumber(result.comparedRows)} note={`${formatNumber(result.sourceRows)} source rows`} /><Kpi label="Mismatches" value={formatNumber(result.mismatchCount)} note="Across all fields" /></div><ArtifactPath label="Indicator parity artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No indicator probe compared" copy="Run the bundled QuantForge indicator probe EA in MT5, export its CSV, then verify SMA, EMA, WMA, RSI, ATR, ranges, z-score, percentile and rate-of-change buffers here." />}</div>;
}

function PortfolioWorkspace({ onError, workspace }: { onError: (message: string | null) => void; workspace: DatabankWorkspace | null }) {
  const [form, setForm] = useState<PortfolioRequest>({ databankPath: workspace?.sourcePath ?? "", brokerPath: "", outputPath: "", objective: "risk_adjusted_return", maximumPairwiseCorrelation: .7, maximumWeightPerStrategy: .25, maximumSymbolExposure: 1, maximumFamilyExposure: .5, maximumStrategies: 10, minimumReturnPercent: 0, cvarTailFraction: .05, stressTrials: 1000, stressBlockLength: 5, seed: 42 });
  const [result, setResult] = useState<PortfolioView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof PortfolioRequest>(key: K, value: PortfolioRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await buildPortfolio(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Hard exposure caps</p><h2>Pack a correlation-constrained portfolio</h2></div></div><div className="form-stack compact"><PathField label="Promotion-grade databank" path={form.databankPath} choose={chooseDatabank} onChange={(value) => update("databankPath", value)} required /><PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required /><PathField label="New portfolio artifact" path={form.outputPath} choose={() => chooseOutputJson("Save portfolio artifact", "portfolio.json")} onChange={(value) => update("outputPath", value)} required /><label className="field-row"><span>Objective</span><select value={form.objective} onChange={(event) => update("objective", event.target.value as PortfolioRequest["objective"])}><option value="risk_adjusted_return">Risk-adjusted return</option><option value="cvar">CVaR</option><option value="minimize_drawdown">Minimize drawdown</option></select></label></div><div className="numeric-grid"><NumberField label="Correlation ceiling" value={form.maximumPairwiseCorrelation} onChange={(value) => update("maximumPairwiseCorrelation", value ?? .7)} step={.01} /><NumberField label="Max strategy weight" value={form.maximumWeightPerStrategy} onChange={(value) => update("maximumWeightPerStrategy", value ?? .25)} step={.01} /><NumberField label="Max family exposure" value={form.maximumFamilyExposure} onChange={(value) => update("maximumFamilyExposure", value ?? .5)} step={.01} /><NumberField label="Strategy cap" value={form.maximumStrategies} onChange={(value) => update("maximumStrategies", value ?? 10)} min={1} /><NumberField label="Stress trials" value={form.stressTrials} onChange={(value) => update("stressTrials", value ?? 1000)} min={1} /><NumberField label="Seed" value={form.seed} onChange={(value) => update("seed", value ?? 42)} min={0} /></div><div className="form-footer"><p>Weights, symbol exposure, family exposure and correlation ceilings are hard constraints—not suggestions.</p><button className="primary" disabled={busy || !form.databankPath || !form.brokerPath || !form.outputPath} onClick={run}>{busy ? "Stress packing…" : "Build portfolio"}</button></div></section>{result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">Portfolio pack</p><h2>{result.selectedStrategies} strategies selected</h2></div><span className="grade-pill">audited</span></div><div className="job-kpis"><Kpi label="Expected return" value={`${formatNumber(result.expectedReturnPercent, 2)}%`} note={`${result.sourceCandidates} source candidates`} /><Kpi label="Path drawdown" value={`${formatNumber(result.maximumDrawdownPercent, 2)}%`} note="Combined path" /><Kpi label="Max correlation" value={formatNumber(result.maximumPairwiseCorrelation, 3)} note="Observed pair" /><Kpi label="Stress CVaR" value={`${formatNumber(result.cvarReturnPercent, 2)}%`} note={`P95 DD ${formatNumber(result.p95DrawdownPercent, 2)}%`} /></div><div className="allocation-list">{result.allocations.map((item) => <div key={item.fingerprint}><code>{item.fingerprint.slice(0, 12)}</code><span>{item.family}</span><strong>{formatNumber(item.weight * 100, 1)}%</strong></div>)}</div><ArtifactPath label="Portfolio artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No portfolio packed" copy="The packer selects complementary elites from one immutable databank and stress-tests the combined return path with block bootstrap trials." />}</div>;
}

function VaultWorkspace({ onError }: { onError: (message: string | null) => void }) {
  const [vaultDirectory, setVaultDirectory] = useState("");
  const [view, setView] = useState<VaultView | null>(null);
  const [busy, setBusy] = useState(false);
  const [showCertify, setShowCertify] = useState(false);
  const [showEvidence, setShowEvidence] = useState(false);
  const [cert, setCert] = useState({ strategyPath: "", brokerPath: "", splitPlanPath: "", evidencePath: "", artifactText: "", requireIncubation: true, selectionBiasWarningThreshold: 1500 });
  async function load() { setBusy(true); onError(null); try { setView(await inspectVault({ vaultDirectory })); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  async function admit() { setBusy(true); onError(null); try { setView(await certifyToVault({ strategyPath: cert.strategyPath, brokerPath: cert.brokerPath, splitPlanPath: cert.splitPlanPath, evidencePath: cert.evidencePath, artifactPaths: cert.artifactText.split("\n").map((value) => value.trim()).filter(Boolean), vaultDirectory, requireIncubation: cert.requireIncubation, selectionBiasWarningThreshold: cert.selectionBiasWarningThreshold })); setShowCertify(false); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="wide-tool-content">
    <section className="panel vault-toolbar">
      <PathField label="Certified Vault directory" path={vaultDirectory} choose={() => chooseDirectory("Choose Certified Vault")} onChange={setVaultDirectory} required />
      <div>
        <button className="secondary" onClick={() => setShowEvidence((value) => !value)}>Assemble evidence</button>
        <button className="secondary" disabled={!vaultDirectory || busy} onClick={load}>{busy ? "Verifying…" : "Inspect Vault"}</button>
        <button className="primary" disabled={!vaultDirectory || busy} onClick={() => setShowCertify((value) => !value)}>Admit candidate</button>
      </div>
    </section>
    {showEvidence && <EvidenceAssemblyPanel onError={onError} onReady={(evidence, artifacts) => { setCert((current) => ({ ...current, evidencePath: evidence, artifactText: artifacts.join("\n") })); setShowEvidence(false); setShowCertify(true); }} />}
    {showCertify && <section className="panel certify-panel"><div className="panel-heading"><div><p className="eyebrow">Certified-only admission</p><h2>Bind the complete evidence chain</h2></div></div><div className="certify-grid"><PathField label="Strategy IR" path={cert.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => setCert((current) => ({ ...current, strategyPath: value }))} required /><PathField label="Broker profile" path={cert.brokerPath} choose={chooseBrokerFile} onChange={(value) => setCert((current) => ({ ...current, brokerPath: value }))} required /><PathField label="Split plan" path={cert.splitPlanPath} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => setCert((current) => ({ ...current, splitPlanPath: value }))} required /><PathField label="Certification evidence" path={cert.evidencePath} choose={() => chooseJsonFile("Choose certification evidence")} onChange={(value) => setCert((current) => ({ ...current, evidencePath: value }))} required /><label className="field-row artifact-text"><span>Gate artifact paths <small>one absolute path per line</small></span><textarea value={cert.artifactText} onChange={(event) => setCert((current) => ({ ...current, artifactText: event.target.value }))} placeholder="validation.json&#10;databank.json&#10;challenge.json&#10;judge.json&#10;parity.json&#10;indicator-parity.json&#10;sealed-final.json&#10;incubation-final.json" /></label><label className="check-field"><input type="checkbox" checked={cert.requireIncubation} onChange={(event) => setCert((current) => ({ ...current, requireIncubation: event.target.checked }))} /><span>Require passing paper incubation</span></label></div><div className="form-footer"><p>Each gate path is rehashed and matched to its exact claim. Denied candidates never reach disk.</p><button className="primary" disabled={busy || !cert.strategyPath || !cert.brokerPath || !cert.splitPlanPath || !cert.evidencePath || !cert.artifactText.trim()} onClick={admit}>Recompute & certify</button></div></section>}
    {view ? <section className="panel vault-list"><div className="panel-heading"><div><p className="eyebrow">Immutable entries</p><h2>{view.certifiedEntries.length} Certified strategies</h2></div>{view.rejectedFiles > 0 && <span className="warning">{view.rejectedFiles} invalid files ignored</span>}</div>{view.certifiedEntries.length ? view.certifiedEntries.map((entry) => <article className="vault-entry" key={entry.entryId}><div><span className="grade-pill">{entry.grade}</span><strong>{entry.strategyFingerprint.slice(0, 18)}</strong><small>{new Date(entry.admittedAt).toLocaleString()}</small></div><div><span>Warnings</span><strong>{entry.warnings}</strong></div><div><span>Incubation</span><strong>{entry.incubationRequired ? "required" : "optional"}</strong></div><code title={entry.path}>{entry.path}</code></article>) : <div className="empty-inline">No intact Certified entries found.</div>}</section> : <WorkspacePrimer title="Choose a Vault directory" copy="Vault inspection recomputes every certification decision, payload hash, entry identity and referenced artifact hash before displaying a strategy as Certified." />}
  </div>;
}

function EvidenceAssemblyPanel({ onError, onReady }: { onError: (message: string | null) => void; onReady: (evidencePath: string, artifacts: string[]) => void }) {
  const [form, setForm] = useState<AssembleEvidenceRequest>({ strategyPath: "", brokerPath: "", splitPlanPath: "", databankPath: "", challengePath: "", judgePath: "", parityPath: "", indicatorParityPath: "", sealedFinalPath: "", incubationPath: null, outputDirectory: "" });
  const [result, setResult] = useState<EvidenceView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof AssembleEvidenceRequest>(key: K, value: AssembleEvidenceRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await assembleEvidence(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  const artifacts = result ? [result.validationPath, form.databankPath, form.challengePath, form.judgePath, form.parityPath, form.indicatorParityPath, form.sealedFinalPath, ...(form.incubationPath ? [form.incubationPath] : [])] : [];
  return <section className="panel certify-panel"><div className="panel-heading"><div><p className="eyebrow">Evidence builder</p><h2>Verify and assemble every promotion gate</h2></div><span className="read-only-badge">Strict</span></div><div className="certify-grid"><PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required /><PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required /><PathField label="Split plan" path={form.splitPlanPath} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => update("splitPlanPath", value)} required /><PathField label="Development databank" path={form.databankPath} choose={chooseDatabank} onChange={(value) => update("databankPath", value)} required /><PathField label="Challenge artifact" path={form.challengePath} choose={() => chooseJsonFile("Choose Challenge artifact")} onChange={(value) => update("challengePath", value)} required /><PathField label="M1 Judge artifact" path={form.judgePath} choose={() => chooseJsonFile("Choose Judge artifact")} onChange={(value) => update("judgePath", value)} required /><PathField label="External parity artifact" path={form.parityPath} choose={() => chooseJsonFile("Choose parity artifact")} onChange={(value) => update("parityPath", value)} required /><PathField label="Indicator parity artifact" path={form.indicatorParityPath} choose={() => chooseJsonFile("Choose indicator parity artifact")} onChange={(value) => update("indicatorParityPath", value)} required /><PathField label="Sealed final artifact" path={form.sealedFinalPath} choose={() => chooseJsonFile("Choose sealed-final artifact")} onChange={(value) => update("sealedFinalPath", value)} required /><PathField label="Incubation final" path={form.incubationPath ?? ""} choose={() => chooseJsonFile("Choose incubation-final artifact")} onChange={(value) => update("incubationPath", value || null)} /><PathField label="New evidence directory" path={form.outputDirectory} choose={() => chooseNewDirectory("Create evidence directory", "quantforge-evidence")} onChange={(value) => update("outputDirectory", value)} required /></div><div className="form-footer"><p>The candidate must be an exact elite from a development-only databank; validation, sealed and MT5 gates must all match the same fingerprint, broker and split.</p>{result ? <button className="primary" onClick={() => onReady(result.evidencePath, artifacts)}>Use for admission</button> : <button className="primary" disabled={busy || Object.entries(form).some(([key, value]) => key !== "incubationPath" && key.endsWith("Path") && value === "") || !form.outputDirectory} onClick={run}>{busy ? "Verifying gates…" : "Assemble evidence"}</button>}</div>{result && <div className="completion-card"><div><span>{result.gateCount} verified gates</span><code>{result.bundlePath}</code></div><strong className="positive">Certification ready</strong></div>}</section>;
}

function DeployWorkspace({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<DeployRequest>({ vaultEntryPath: "", outputDirectory: "" }); const [result, setResult] = useState<DeployView | null>(null); const [busy, setBusy] = useState(false);
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await buildDeploymentPack(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Final guarded materialization</p><h2>Build a deployment pack</h2></div><span className="read-only-badge">Live off</span></div><div className="form-stack"><PathField label="Certified Vault entry" path={form.vaultEntryPath} choose={() => chooseJsonFile("Choose Certified Vault entry")} onChange={(value) => setForm((current) => ({ ...current, vaultEntryPath: value }))} required /><PathField label="New deployment directory" path={form.outputDirectory} choose={() => chooseNewDirectory("Create deployment directory", "quantforge-deployment")} onChange={(value) => setForm((current) => ({ ...current, outputDirectory: value }))} required /></div><div className="deployment-checks"><span>✓ Certification recomputed</span><span>✓ Gate files rehashed</span><span>✓ Incubation reverified</span><span>✓ External parity rerun</span><span>✓ EA source regenerated byte-for-byte</span><span>✓ AllowLiveTrading=false</span></div><div className="form-footer"><p>Deployment is refused unless the exact Certified strategy passed mandatory incubation and external MT5 parity.</p><button className="primary" disabled={busy || !form.vaultEntryPath || !form.outputDirectory} onClick={run}>{busy ? "Reverifying chain…" : "Build deployment pack"}</button></div></section>{result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">Deployment pack</p><h2>{result.expertName}</h2></div><span className="grade-pill">{result.grade}</span></div><div className="job-kpis"><Kpi label="Symbol / timeframe" value={`${result.symbol} · ${result.timeframe}`} note={`Magic ${result.magic}`} /><Kpi label="Files" value={formatNumber(result.fileCount)} note="Manifest-bound" /><Kpi label="Warnings" value={formatNumber(result.certificationWarnings)} note="Carried forward" /><Kpi label="Live trading" value={result.liveTradingDefault ? "enabled" : "disabled"} note="Operator lock" /></div><ArtifactPath label="Deployment directory" value={result.outputDirectory} /><ArtifactPath label="Deployment ID" value={result.deploymentId} /><div className="safety-card"><strong>Operator action required</strong><span>Review broker settings and paper operation independently before enabling live orders in MT5.</span></div></section> : <WorkspacePrimer title="No deployment pack built" copy="Only an intact Certified Vault entry can reach this stage. The output includes the parity-passed EA, set file, tester config, strategy IR, broker spec, risk pack, evidence, changelog and deployment manifest." />}</div>;
}

function WorkspacePrimer({ title, copy }: { title: string; copy: string }) { return <section className="panel waiting-panel"><div className="data-glyph"><i /><i /><i /><i /><i /></div><h3>{title}</h3><p>{copy}</p></section>; }
function ArtifactPath({ label, value }: { label: string; value: string }) { return <div className="artifact-path"><span>{label}</span><code>{value}</code></div>; }

function PathField({
  choose,
  label,
  onChange,
  path,
  required = false,
}: {
  choose: () => Promise<string | null>;
  label: string;
  onChange: (value: string) => void;
  path: string;
  required?: boolean;
}) {
  return (
    <label className="field-row path-field">
      <span>{label} {required && <small>required</small>}</span>
      <div><input value={path} onChange={(event) => onChange(event.target.value)} placeholder="Choose a local file…" /><button type="button" className="secondary" onClick={() => void choose().then((value) => value && onChange(value))}>Choose</button></div>
    </label>
  );
}

function NumberField({
  label,
  min,
  onChange,
  optional = false,
  step = 1,
  value,
}: {
  label: string;
  min?: number;
  onChange: (value: number | null) => void;
  optional?: boolean;
  step?: number;
  value: number | null;
}) {
  return <label className="number-field"><span>{label}{optional && <small>optional</small>}</span><input type="number" min={min} step={step} value={value ?? ""} onChange={(event) => onChange(event.target.value === "" ? null : Number(event.target.value))} /></label>;
}

function ClusterView({
  rows,
  selected,
  onSelect,
}: {
  rows: EliteRow[];
  selected: string | null;
  onSelect: (fingerprint: string) => void;
}) {
  const evidenceMinimum = Math.min(...rows.map((row) => row.evidence));
  const evidenceMaximum = Math.max(...rows.map((row) => row.evidence));
  const evidenceRange = Math.max(evidenceMaximum - evidenceMinimum, 1e-9);
  const familyClass = (family: string) =>
    `cluster-${family.toLowerCase().replaceAll(" ", "-")}`;

  return (
    <section className="panel cluster-panel">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Behavioral cluster</p>
          <h2>Evidence × novelty</h2>
        </div>
        <div className="cluster-legend" aria-label="Strategy families">
          {["trend", "momentum", "breakout", "mean reversion"].map((name) => (
            <span key={name}><i className={familyClass(name)} />{name}</span>
          ))}
        </div>
      </div>
      {rows.length === 0 ? (
        <p className="cluster-empty">No elites have been illuminated.</p>
      ) : (
        <div className="cluster-chart" aria-label="Elite evidence and novelty cluster">
          <span className="cluster-y-label">Higher novelty</span>
          <span className="cluster-x-label">Higher evidence</span>
          {rows.map((row) => {
            const x = 4 + ((row.evidence - evidenceMinimum) / evidenceRange) * 92;
            const y = 96 - Math.max(0, Math.min(1, row.novelty)) * 92;
            return (
              <button
                aria-label={`${row.strategyId}: evidence ${row.evidence.toFixed(2)}, novelty ${row.novelty.toFixed(3)}`}
                className={`cluster-point ${familyClass(row.family)} ${selected === row.fingerprint ? "selected" : ""}`}
                key={row.fingerprint}
                onClick={() => onSelect(row.fingerprint)}
                style={{ left: `${x}%`, top: `${y}%` }}
                title={`${row.strategyId} · ${row.family} · evidence ${row.evidence.toFixed(2)} · novelty ${row.novelty.toFixed(3)}`}
              />
            );
          })}
        </div>
      )}
    </section>
  );
}

function FidelityDemoPanel({
  onError,
  onLoaded,
  workspace,
}: {
  onError: (message: string | null) => void;
  onLoaded: (path: string) => Promise<void>;
  workspace: DatabankWorkspace;
}) {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<FidelityDemoView | null>(null);

  async function run() {
    onError(null);
    setBusy(true);
    try {
      const outputPath = await chooseNewDatabank();
      if (!outputPath) return;
      const m1DataPath = workspace.m1DataPath;
      if (!m1DataPath) {
        onError("This databank has no M1 source in its manifest. Re-run Discover with a symbol pack that includes M1.");
        return;
      }
      const view = await runFidelityDemo({
        databankPath: workspace.sourcePath,
        m1DataPath,
        m1MetadataPath: workspace.m1MetadataPath,
        m1SourceTimezone: workspace.m1MetadataPath ? null : "Etc/UTC",
        outputPath,
        returnRetention: 0.8,
        tradeRetention: 0.8,
        drawdownExpansion: 1.3,
      });
      setResult(view);
      if (view.outputPath) {
        await onLoaded(view.outputPath);
      }
    } catch (reason) {
      onError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="panel fidelity-panel">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">SQX RetestWithHigherPrecision</p>
          <h2>Fidelity demo (M1)</h2>
        </div>
        <button className="primary" disabled={busy} onClick={() => void run()}>
          {busy ? "Retesting…" : "Run M1 fidelity demo"}
        </button>
      </div>
      <p className="immutable-note">
        Research bank from H1 scout. Retest elites on M1 with SQX-style bands (≥80% return &amp; trades, DD ≤130% of H1).
        Passers are written to a new promotion-grade databank.
      </p>
      {result && (
        <div className="fidelity-results">
          <p>
            {formatNumber(result.passed)} passed · {formatNumber(result.failed)} failed · {formatNumber(result.evaluated)} evaluated
            {result.outputPath ? ` · wrote ${result.outputPath}` : " · no passers written"}
          </p>
          <ul>
            {result.results.slice(0, 12).map((row) => (
              <li key={row.fingerprint}>
                <strong className={row.passed ? "pass" : "fail"}>{row.strategyId}</strong>
                <span>{row.reason}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}

function EmptyState({ onOpen, loading }: { onOpen: () => void; loading: boolean }) {
  return (
    <section className="empty-state">
      <div className="empty-grid" aria-hidden="true">
        {Array.from({ length: 96 }, (_, index) => <i key={index} />)}
      </div>
      <p className="eyebrow">Databank workspace</p>
      <h2>Open a real illumination archive</h2>
      <p>
        QuantForge validates the immutable run manifest, candidate fingerprints,
        niche map, stored gates and evaluation count before anything is displayed.
      </p>
      <button className="primary" onClick={onOpen} disabled={loading}>
        {loading ? "Validating artifact…" : "Choose databank JSON"}
      </button>
      <small>Read-only · no artifact will be changed</small>
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

function CoverageGrid({
  family,
  onSelect,
}: {
  family: FamilyCoverage;
  onSelect: (fingerprint: string) => void;
}) {
  return (
    <article className="family-grid-card">
      <header>
        <strong>{family.family}</strong>
        <span>{family.occupied} / {family.total}</span>
      </header>
      <div className="coverage-grid">
        {family.cells.map((cell) => (
          <button
            aria-label={`${cell.niche}: ${cell.occupied ? "occupied" : "empty"}`}
            className={cell.occupied ? "coverage-cell occupied" : "coverage-cell"}
            disabled={!cell.fingerprint}
            key={cell.index}
            onClick={() => cell.fingerprint && onSelect(cell.fingerprint)}
            style={{ "--intensity": cell.intensity } as React.CSSProperties}
            title={`${cell.niche}${cell.occupied ? ` · evidence intensity ${(cell.intensity * 100).toFixed(0)}%` : " · empty"}`}
          />
        ))}
      </div>
    </article>
  );
}

function EquityCurvePanel({
  initialBalance,
  rows,
}: {
  initialBalance: number;
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
          <p className="eyebrow">M1 performance paths</p>
          <h2>Backtest cumulative equity curves</h2>
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
                    style={{ stroke: colors[curveIndex] }}
                  />
                );
              })}
            </svg>
          </div>
          <div className="equity-legend">
            {curves.map(({ row }, index) => (
              <button key={row.fingerprint} title={row.fingerprint}>
                <i style={{ background: colors[index] }} />
                <span>{row.strategyId}</span>
                <strong className={row.returnPercent >= 0 ? "positive" : "negative"}>
                  {row.returnPercent.toFixed(2)}%
                </strong>
              </button>
            ))}
          </div>
          {rows.length > visible.length && (
            <p className="axis-note">Showing the first 12 selected strategies to keep the chart readable.</p>
          )}
          <p className="axis-note">
            Reconstructed from each strategy's stored 64-point M1 equity signature; start and end
            balances match its recorded backtest.
          </p>
        </>
      )}
    </section>
  );
}

function EliteTable({
  batchSelection,
  rows,
  selected,
  onSelect,
  onToggle,
}: {
  batchSelection: Set<string>;
  rows: EliteRow[];
  selected: string | null;
  onSelect: (fingerprint: string) => void;
  onToggle: (fingerprint: string) => void;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const rowHeight = 48;
  const viewportHeight = 360;
  const overscan = 5;
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(rows.length, start + Math.ceil(viewportHeight / rowHeight) + overscan * 2);

  return (
    <div className="elite-table">
      <div className="elite-row elite-header">
        <span>Select</span><span>Strategy</span><span>Family</span><span>Evidence</span><span>Novelty</span>
        <span>Trades</span><span>Return</span><span>DD</span><span>Ret/DD</span><span>Sharpe</span><span>OOS1×</span>
      </div>
      <div
        className="elite-viewport"
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        style={{ height: viewportHeight }}
      >
        <div className="elite-virtual-body" style={{ height: rows.length * rowHeight }}>
          {rows.slice(start, end).map((row, offset) => (
            <div
              className={`elite-row interactive ${selected === row.fingerprint ? "selected" : ""}`}
              key={row.fingerprint}
              onClick={() => onSelect(row.fingerprint)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") onSelect(row.fingerprint);
              }}
              role="button"
              style={{ height: rowHeight, transform: `translateY(${(start + offset) * rowHeight}px)` }}
              tabIndex={0}
            >
              <span className="batch-check">
                <input
                  aria-label={`Select ${row.strategyId} for batch export`}
                  checked={batchSelection.has(row.fingerprint)}
                  onChange={() => onToggle(row.fingerprint)}
                  onClick={(event) => event.stopPropagation()}
                  type="checkbox"
                />
              </span>
              <span className="strategy-cell"><strong>{row.strategyId}</strong><small>{row.fingerprint.slice(0, 10)}</small></span>
              <span>{row.family}</span>
              <span>{row.evidence.toFixed(2)}</span>
              <span>{row.novelty.toFixed(3)}</span>
              <span>{formatNumber(row.trades)}</span>
              <span className={row.returnPercent >= 0 ? "positive" : "negative"}>{row.returnPercent.toFixed(2)}%</span>
              <span>{row.drawdownPercent.toFixed(2)}%</span>
              <span>{formatReturnDrawdown(row)}</span>
              <span>{row.sharpeRatio === null ? "—" : row.sharpeRatio.toFixed(2)}</span>
              <span title={row.oos1Expectancy === null ? "No OOS1 pick metrics (legacy databank)" : `OOS1 ${(row.oos1Expectancy / 1000).toFixed(2)}R / IS ${(row.isExpectancy / 1000).toFixed(2)}R`}>
                {row.oos1ExpectancyRatio === null ? "—" : `${row.oos1ExpectancyRatio.toFixed(2)}×`}
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function EliteInspector({
  detail,
  loading,
  onError,
  onParity,
  tab,
  onTab,
}: {
  detail: EliteDetail | null;
  loading: boolean;
  onError: (message: string | null) => void;
  onParity: (fingerprint: string) => Promise<void>;
  tab: "overview" | "ir";
  onTab: (tab: "overview" | "ir") => void;
}) {
  return (
    <aside className="panel inspector inspector-wide">
      {loading && <div className="inspector-loading">Loading elite…</div>}
      {!detail ? (
        <div className="inspector-empty">Select an occupied niche or table row.</div>
      ) : (
        <>
          <div className="inspector-head">
            <span className="grade-pill">{detail.grade}</span>
            <span className="parity-pill">Parity {detail.parity}</span>
            <h2>{detail.strategyId}</h2>
            <code>{detail.fingerprint}</code>
          </div>
          <div className="tabs">
            <button className={tab === "overview" ? "active" : ""} onClick={() => onTab("overview")}>Overview</button>
            <button className={tab === "ir" ? "active" : ""} onClick={() => onTab("ir")}>IR tree</button>
            <button onClick={() => void exportEliteStrategy(detail.fingerprint).catch((reason) => onError(String(reason)))}>Export IR</button>
            <button onClick={() => void onParity(detail.fingerprint)}>Go to M1 / MT5 →</button>
          </div>
          {tab === "overview" ? (
            <div className="inspector-body">
              <section>
                <p className="eyebrow">Thesis</p>
                <p>{detail.thesis}</p>
              </section>
              <PartitionEquityChart fingerprint={detail.fingerprint} onError={onError} />
              <section className="metric-list">
                <Metric label="Evidence" value={Number(detail.evidence.total).toFixed(2)} />
                <Metric label="Return" value={`${Number(detail.metrics.return_percent).toFixed(2)}%`} />
                <Metric label="Max drawdown" value={`${Number(detail.metrics.max_drawdown_percent).toFixed(2)}%`} />
                <Metric
                  label="Return / DD"
                  value={formatReturnDrawdown({
                    returnPercent: Number(detail.metrics.return_percent),
                    drawdownPercent: Number(detail.metrics.max_drawdown_percent),
                    returnDrawdown: finiteRatio(
                      Number(detail.metrics.return_percent),
                      Number(detail.metrics.max_drawdown_percent),
                    ),
                  })}
                />
                <Metric label="Trades" value={formatNumber(Number(detail.metrics.trade_count))} />
                <Metric label="Win rate" value={`${Number(detail.metrics.win_rate).toFixed(1)}%`} />
                <Metric label="Profit factor" value={detail.metrics.profit_factor === null ? "—" : Number(detail.metrics.profit_factor).toFixed(2)} />
                <Metric
                  label="Sharpe"
                  value={effectiveDetailSharpe(detail)?.toFixed(2) ?? "—"}
                />
              </section>
              <section>
                <p className="eyebrow">Behavioral niche</p>
                <code className="niche-code">{detail.niche}</code>
              </section>
              <div className="gate-stack">
                <span className="gate-pass">IS train · OOS1 pick gate</span>
                <span className="gate-pass">OOS2 display only</span>
                <span className="gate-pending">External parity unknown</span>
              </div>
            </div>
          ) : (
            <pre className="ir-tree">{JSON.stringify(detail.strategyIr, null, 2)}</pre>
          )}
        </>
      )}
    </aside>
  );
}

function PartitionEquityChart({
  fingerprint,
  onError,
}: {
  fingerprint: string;
  onError: (message: string | null) => void;
}) {
  const [view, setView] = useState<PartitionEquityView | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setBusy(true);
    setView(null);
    void getElitePartitionEquity(fingerprint)
      .then((next) => {
        if (!cancelled) setView(next);
      })
      .catch((reason) => {
        if (!cancelled) onError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fingerprint, onError]);

  if (busy && !view) {
    return <div className="partition-equity loading">Replaying full IS / OOS1 / OOS2 equity…</div>;
  }
  if (!view || view.points.length < 2) {
    return <div className="partition-equity empty">Equity unavailable for this elite.</div>;
  }

  const width = 520;
  const height = 300;
  const pad = 18;
  const equities = view.points.map((point) => point.equity);
  const min = Math.min(...equities);
  const max = Math.max(...equities);
  const span = Math.max(max - min, 1e-9);
  const t0 = view.points[0].timestampMs;
  const t1 = view.points[view.points.length - 1].timestampMs;
  const tSpan = Math.max(t1 - t0, 1);
  const xAt = (timestamp: number) => pad + ((timestamp - t0) / tSpan) * (width - pad * 2);
  const yAt = (equity: number) => height - pad - ((equity - min) / span) * (height - pad * 2);
  const path = view.points
    .map((point, index) => `${index === 0 ? "M" : "L"}${xAt(point.timestampMs).toFixed(1)} ${yAt(point.equity).toFixed(1)}`)
    .join(" ");
  const isX = xAt(view.isEndTimestampMs);
  const oos1X = xAt(view.oos1EndTimestampMs);

  return (
    <section className="partition-equity">
      <div className="partition-equity-head">
        <div>
          <p className="eyebrow">Full-run equity</p>
          <small>IS 60% · OOS1 20% · OOS2 20% (display only)</small>
        </div>
      </div>
      <svg viewBox={`0 0 ${width} ${height}`} className="partition-equity-svg" role="img" aria-label="Partitioned equity curve">
        <rect x={pad} y={pad} width={isX - pad} height={height - pad * 2} className="region-is" />
        <rect x={isX} y={pad} width={Math.max(0, oos1X - isX)} height={height - pad * 2} className="region-oos1" />
        <rect x={oos1X} y={pad} width={Math.max(0, width - pad - oos1X)} height={height - pad * 2} className="region-oos2" />
        <line x1={isX} y1={pad} x2={isX} y2={height - pad} className="divider" />
        <line x1={oos1X} y1={pad} x2={oos1X} y2={height - pad} className="divider" />
        <path d={path} className="equity-path" />
        <text x={pad + 6} y={pad + 14} className="region-label">IS</text>
        <text x={isX + 6} y={pad + 14} className="region-label">OOS1</text>
        <text x={oos1X + 6} y={pad + 14} className="region-label">OOS2</text>
      </svg>
      <div className="partition-kpis">
        <Kpi label="IS expectancy (R)" value={formatNumber(view.isExpectancy / 1000, 2)} note={`${formatNumber(view.isTrades)} trades · ${formatNumber(view.isReturnPercent, 2)}%`} />
        <Kpi label="OOS1 expectancy (R)" value={formatNumber(view.oos1Expectancy / 1000, 2)} note={view.oos1ExpectancyRatio === null ? "no ratio" : `${view.oos1ExpectancyRatio.toFixed(2)}× IS`} />
        <Kpi label="OOS2 expectancy (R)" value={formatNumber(view.oos2Expectancy / 1000, 2)} note="display only · not a gate" />
        <Kpi label="Bars" value={`${formatNumber(view.isBars)} / ${formatNumber(view.oos1Bars)} / ${formatNumber(view.oos2Bars)}`} note="IS / OOS1 / OOS2" />
      </div>
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return <div><span>{label}</span><strong>{value}</strong></div>;
}

function finiteRatio(returnPercent: number, drawdownPercent: number): number | null {
  if (drawdownPercent <= 1e-12) return null;
  const value = returnPercent / drawdownPercent;
  return Number.isFinite(value) ? value : null;
}

function formatReturnDrawdown(
  value: Pick<EliteRow, "returnPercent" | "drawdownPercent" | "returnDrawdown">,
): string {
  if (value.returnDrawdown !== null) return value.returnDrawdown.toFixed(2);
  return value.drawdownPercent <= 1e-12 && value.returnPercent > 0 ? "∞" : "—";
}

function effectiveDetailSharpe(detail: EliteDetail): number | null {
  const persisted = detail.metrics.sharpe_ratio;
  if (persisted !== null && persisted !== undefined && Number.isFinite(Number(persisted))) {
    return Number(persisted);
  }
  const values = detail.equitySignature;
  if (values.length < 2) return null;
  const mean = values.reduce((total, value) => total + value, 0) / values.length;
  const variance = values.reduce(
    (total, value) => total + (value - mean) ** 2,
    0,
  ) / (values.length - 1);
  const deviation = Math.sqrt(variance);
  return deviation > 1e-12 ? mean / deviation * Math.sqrt(values.length) : null;
}

function equityCurveValues(
  values: number[],
  initialBalance: number,
  returnPercent: number,
) {
  const targetProfit = initialBalance * returnPercent / 100;
  const signatureTotal = values.reduce((total, value) => total + value, 0);
  const scale = Math.abs(signatureTotal) > 1e-9 ? targetProfit / signatureTotal : 0;
  let balance = initialBalance;
  return [
    balance,
    ...values.map((value, index) => {
      balance += scale === 0
        ? targetProfit / Math.max(values.length, 1)
        : value * scale;
      if (index === values.length - 1) balance = initialBalance + targetProfit;
      return balance;
    }),
  ];
}

function Sparkline({
  initialBalance,
  returnPercent,
  values,
}: {
  initialBalance: number;
  returnPercent: number;
  values: number[];
}) {
  if (values.length < 2) return null;
  const curve = equityCurveValues(values, initialBalance, returnPercent);
  const minimum = Math.min(...curve);
  const maximum = Math.max(...curve);
  const range = Math.max(maximum - minimum, 1e-9);
  const points = curve
    .map((value, index) => `${(index / (curve.length - 1)) * 300},${70 - ((value - minimum) / range) * 60}`)
    .join(" ");
  return (
    <section className="sparkline-wrap">
      <div><p className="eyebrow">Equity curve</p><small>64-point M1 archive downsample</small></div>
      <svg viewBox="0 0 300 80" role="img" aria-label="Downsampled cumulative equity curve">
        <polyline points={points} fill="none" />
      </svg>
    </section>
  );
}

export default App;
