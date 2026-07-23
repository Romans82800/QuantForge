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
  getDiscoverJob,
  inspectData,
  inspectVault,
  loadDatabankPath,
  loadElite,
  pauseDiscover,
  recordIncubation,
  resumeDiscover,
  runChallenge,
  runM1Judge,
  runSealedFinal,
  startIncubation,
  finalizeIncubation,
  startDiscover,
  stopDiscover,
} from "./api";
import type {
  AssembleEvidenceRequest,
  ChallengeRequest,
  ChallengeView,
  DatabankWorkspace,
  DataLabView,
  DeployRequest,
  DeployView,
  DiscoverJobView,
  DiscoverRequest,
  EliteDetail,
  EliteRow,
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
  PortfolioRequest,
  PortfolioView,
  SealedRequest,
  SealedView,
  VaultView,
  WorkspaceName,
} from "./types";
import {
  bindDiscoverTimezone,
  discoverProgress,
  filterAndSortElites,
  formatNumber,
  selectionBiasTone,
  visibleEliteFingerprint,
} from "./view";

const workspaces: { name: WorkspaceName; enabled: boolean }[] = [
  { name: "Home", enabled: true },
  { name: "Databank", enabled: true },
  { name: "Discover", enabled: true },
  { name: "Challenge", enabled: true },
  { name: "Vault", enabled: true },
  { name: "Data Lab", enabled: true },
  { name: "Parity Lab", enabled: true },
  { name: "Portfolio", enabled: true },
  { name: "Deploy", enabled: true },
];

const workspaceTitles: Record<WorkspaceName, string> = {
  Home: "Research cockpit",
  Databank: "Quality-diversity archive",
  Discover: "Deterministic strategy discovery",
  Challenge: "Robustness challenge",
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
        ) : activeWorkspace === "Challenge" ? (
          <ChallengeWorkspace onError={setError} workspace={workspace} />
        ) : activeWorkspace === "Parity Lab" ? (
          <ParityWorkspace onError={setError} />
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
                note={`${formatNumber(workspace.rejections.total)} total rejections`}
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
                        <option value="family">Family A–Z</option>
                        <option value="grade">Grade A–Z</option>
                      </select>
                    </div>
                  </div>
                  <EliteTable
                    rows={filtered}
                    selected={detail?.fingerprint ?? null}
                    onSelect={selectElite}
                  />
                </section>
              </div>

              <EliteInspector
                detail={detail}
                loading={detailLoading}
                onError={setError}
                tab={detailTab}
                onTab={setDetailTab}
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
          description="Explore behavioral coverage, selection-bias counts, evidence, novelty and the exact strategy IR."
          eyebrow="03 / Interrogate"
          onClick={hasDatabank ? () => onNavigate("Databank") : onOpen}
          title="Databank"
        />
        <WorkflowCard
          action="Run Challenge"
          description="Freeze chronological development, validation and sealed partitions, then run folds, cost shocks, Monte Carlo and parameter-neighborhood tests."
          eyebrow="04 / Challenge"
          onClick={() => onNavigate("Challenge")}
          title="Challenge"
        />
        <WorkflowCard
          action="Open Parity Lab"
          description="Replay decisions on M1, generate the guarded MT5 expert, and compare trade-level and equity-path output with Strategy Tester."
          eyebrow="05 / Reconcile"
          onClick={() => onNavigate("Parity Lab")}
          title="Parity Lab"
        />
        <WorkflowCard
          action="Open Vault"
          description="Admit only complete evidence chains, inspect immutable Certified entries, and hand a verified candidate to deployment packaging."
          eyebrow="06 / Certify"
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
          <PathField label="OHLC data" path={dataPath} choose={chooseDataFile} onChange={setDataPath} required />
          <PathField label="Exporter metadata" path={metadataPath} choose={chooseMetadataFile} onChange={setMetadataPath} />
          {!metadataPath && (
            <label className="field-row">
              <span>Source timezone <small>required without metadata</small></span>
              <input value={sourceTimezone} onChange={(event) => setSourceTimezone(event.target.value)} placeholder="Etc/UTC" />
            </label>
          )}
          <PathField label="Broker profile" path={brokerPath} choose={chooseBrokerFile} onChange={setBrokerPath} />
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
    brokerPath: "",
    databankPath: "",
    generations: 5,
    initialCandidates: 500,
    batchSize: 200,
    correlationThreshold: 0.88,
    noveltyWeight: 10,
    seed: 42,
    minimumTrades: 20,
    maximumDrawdownPercent: 30,
    minimumReturnPercent: 0,
    minimumProfitFactor: 1,
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
      const sourceBoundForm = bindDiscoverTimezone(form);
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

  const progress = discoverProgress(job?.completedGenerations ?? 0, job?.requestedGenerations ?? 0);
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
            <PathField label="OHLC data" path={form.dataPath} choose={chooseDataFile} onChange={(value) => update("dataPath", value)} required />
            <PathField label="Exporter metadata" path={form.metadataPath ?? ""} choose={chooseMetadataFile} onChange={(value) => setForm((current) => ({ ...current, metadataPath: value || null, sourceTimezone: value ? null : current.sourceTimezone ?? "Etc/UTC" }))} />
            {!form.metadataPath && <label className="field-row"><span>Source timezone</span><input value={form.sourceTimezone ?? ""} onChange={(event) => update("sourceTimezone", event.target.value || null)} /></label>}
            <PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required />
            <PathField label={form.mode === "new" ? "New databank" : "Existing databank"} path={form.databankPath} choose={chooseOutput} onChange={(value) => update("databankPath", value)} required />
          </div>
          <div className="numeric-grid core-numbers">
            <NumberField label="Generations" value={form.generations} onChange={(value) => update("generations", value ?? 1)} min={1} />
            {form.mode === "new" && <>
              <NumberField label="Initial candidates" value={form.initialCandidates} onChange={(value) => update("initialCandidates", value)} min={1} />
              <NumberField label="Batch / generation" value={form.batchSize} onChange={(value) => update("batchSize", value)} min={1} />
              <NumberField label="Seed" value={form.seed} onChange={(value) => update("seed", value)} min={0} />
              <NumberField label="Validation reserve" value={form.validationFraction} onChange={(value) => update("validationFraction", value)} step={0.05} />
              <NumberField label="Sealed reserve" value={form.sealedFraction} onChange={(value) => update("sealedFraction", value)} step={0.05} />
            </>}
          </div>
          {form.mode === "new" ? (
            <details className="advanced-settings">
              <summary>Gates, diversity and cost model</summary>
              <div className="numeric-grid">
                <NumberField label="Correlation ceiling" value={form.correlationThreshold} onChange={(value) => update("correlationThreshold", value)} step={0.01} />
                <NumberField label="Novelty weight" value={form.noveltyWeight} onChange={(value) => update("noveltyWeight", value)} step={0.1} />
                <NumberField label="Minimum trades" value={form.minimumTrades} onChange={(value) => update("minimumTrades", value)} min={0} />
                <NumberField label="Maximum drawdown %" value={form.maximumDrawdownPercent} onChange={(value) => update("maximumDrawdownPercent", value)} step={0.1} />
                <NumberField label="Minimum return %" value={form.minimumReturnPercent} onChange={(value) => update("minimumReturnPercent", value)} step={0.1} />
                <NumberField label="Minimum profit factor" value={form.minimumProfitFactor} onChange={(value) => update("minimumProfitFactor", value)} step={0.01} />
                <NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value)} step={0.01} />
                <NumberField label="Slippage points / side" value={form.slippagePointsPerSide} onChange={(value) => update("slippagePointsPerSide", value)} step={0.1} />
                <NumberField label="Fallback spread points" value={form.fallbackSpreadPoints} onChange={(value) => update("fallbackSpreadPoints", value)} step={0.1} optional />
                <NumberField label="Maximum spread points" value={form.maxSpreadPoints} onChange={(value) => update("maxSpreadPoints", value)} step={0.1} optional />
                <NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value)} min={1} />
              </div>
            </details>
          ) : (
            <p className="immutable-note">Continuation loads the grammar, seed, gates and cost model from the verified databank. Only the generation count can change.</p>
          )}
          {form.mode === "new" && <label className="check-field discover-split"><input type="checkbox" checked={form.promotionSplit ?? false} onChange={(event) => update("promotionSplit", event.target.checked)} /><span>Certification-grade search: evaluate development partition only</span></label>}
        </fieldset>
        <div className="form-footer">
          <p>New jobs refuse to overwrite a file. Continued jobs create a versioned checkpoint and preserve the original.</p>
          <button className="primary" disabled={active || busy || !form.dataPath || !form.brokerPath || !form.databankPath} onClick={start}>{busy ? "Starting…" : form.mode === "new" ? "Start discovery" : "Continue discovery"}</button>
        </div>
      </section>

      <section className={`panel job-panel job-${job?.status ?? "idle"}`}>
        <div className="panel-heading"><div><p className="eyebrow">Live worker</p><h2>{job?.phase ?? "No job loaded"}</h2></div><span className="job-status">{job?.status ?? "idle"}</span></div>
        <div className="job-body">
          <p className="job-message">{job?.message ?? "Configure a search to start the Rust discovery worker."}</p>
          <div className="progress-track"><i style={{ width: `${progress}%` }} /></div>
          <div className="progress-label"><span>{job?.completedGenerations ?? 0} / {job?.requestedGenerations ?? 0} generations this run</span><strong>{progress.toFixed(0)}%</strong></div>
          <div className="job-kpis">
            <Kpi label="Evaluations" value={formatNumber(job?.evaluationCount ?? 0)} note="Persistent selection-bias count" />
            <Kpi label="Coverage" value={formatNumber(job?.coverage ?? 0)} note="Occupied behavioral niches" />
            <Kpi label="QD score" value={formatNumber(job?.qdScore ?? 0, 2)} note="Positive evidence mass" />
            <Kpi label="Rejected" value={formatNumber(job?.rejectedTotal ?? 0)} note={`${formatNumber((job?.rejectedClone ?? 0) + (job?.rejectedCorrelated ?? 0))} clone + correlation`} />
          </div>
          {active && <div className="job-controls">
            {job?.status === "running" ? <button className="secondary" onClick={() => void control(pauseDiscover)}>Pause</button> : <button className="secondary" onClick={() => void control(resumeDiscover)}>Resume</button>}
            <button className="danger" onClick={() => void control(stopDiscover)}>Stop & checkpoint</button>
            <small>Pause and stop take effect at a generation boundary.</small>
          </div>}
          {job?.status === "completed" && job.outputPath && <div className="completion-card"><div><span>Checkpoint</span><code>{job.outputPath}</code></div><button className="primary" onClick={() => onOpenDatabank(job.outputPath!)}>Open in Databank</button></div>}
          {job?.status === "failed" && <div className="job-error">{job.message}</div>}
        </div>
      </section>
    </div>
  );
}

function ChallengeWorkspace({
  onError,
  workspace,
}: {
  onError: (message: string | null) => void;
  workspace: DatabankWorkspace | null;
}) {
  const [form, setForm] = useState<ChallengeRequest>({
    dataPath: "",
    metadataPath: null,
    sourceTimezone: "Etc/UTC",
    strategyPath: "",
    brokerPath: "",
    outputDirectory: "",
    validationFraction: 0.2,
    sealedFraction: 0.2,
    evaluationsTouched: workspace?.selectionBias.evaluationCount ?? 1,
    commissionPerLotRoundTurn: 7,
    slippagePointsPerSide: 0,
    fallbackSpreadPoints: null,
    maxSpreadPoints: null,
    initialBalance: 100000,
    folds: 5,
    monteCarloTrials: 1000,
    neighborhoodSamples: 20,
    seed: 42,
  });
  const [result, setResult] = useState<ChallengeView | null>(null);
  const [busy, setBusy] = useState(false);
  function update<K extends keyof ChallengeRequest>(key: K, value: ChallengeRequest[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }
  async function run() {
    setBusy(true); setResult(null); onError(null);
    try { setResult(await runChallenge(form)); }
    catch (reason) { onError(String(reason)); }
    finally { setBusy(false); }
  }
  return <div className="tool-content promotion-layout">
    <section className="panel setup-panel">
      <div className="panel-heading"><div><p className="eyebrow">Promotion gate</p><h2>Run the robustness battery</h2></div><span className="read-only-badge">No-clobber</span></div>
      <div className="form-stack compact">
        <PathField label="Decision OHLC" path={form.dataPath} choose={chooseDataFile} onChange={(value) => update("dataPath", value)} required />
        <PathField label="Exporter metadata" path={form.metadataPath ?? ""} choose={chooseMetadataFile} onChange={(value) => setForm((current) => ({ ...current, metadataPath: value || null, sourceTimezone: value ? null : current.sourceTimezone ?? "Etc/UTC" }))} />
        {!form.metadataPath && <label className="field-row"><span>Source timezone</span><input value={form.sourceTimezone ?? ""} onChange={(event) => update("sourceTimezone", event.target.value || null)} /></label>}
        <PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required />
        <PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required />
        <PathField label="New Challenge directory" path={form.outputDirectory} choose={() => chooseNewDirectory("Create Challenge directory", "quantforge-challenge")} onChange={(value) => update("outputDirectory", value)} required />
      </div>
      <div className="numeric-grid">
        <NumberField label="Validation fraction" value={form.validationFraction} onChange={(value) => update("validationFraction", value ?? .2)} step={.05} />
        <NumberField label="Sealed fraction" value={form.sealedFraction} onChange={(value) => update("sealedFraction", value ?? .2)} step={.05} />
        <NumberField label="Evaluations touched" value={form.evaluationsTouched} onChange={(value) => update("evaluationsTouched", value ?? 1)} min={1} />
        <NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value ?? 0)} step={.01} />
        <NumberField label="Folds" value={form.folds} onChange={(value) => update("folds", value ?? 5)} min={2} />
        <NumberField label="Monte Carlo trials" value={form.monteCarloTrials} onChange={(value) => update("monteCarloTrials", value ?? 1000)} min={1} />
        <NumberField label="Neighborhood samples" value={form.neighborhoodSamples} onChange={(value) => update("neighborhoodSamples", value ?? 20)} min={1} />
        <NumberField label="Seed" value={form.seed} onChange={(value) => update("seed", value ?? 42)} min={0} />
        <NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value ?? 100000)} min={1} />
      </div>
      <div className="form-footer"><p>The app freezes a 60/20/20 split by default. Failed Challenge runs are still written for audit but remain Illuminated.</p><button className="primary" disabled={busy || !form.dataPath || !form.strategyPath || !form.brokerPath || !form.outputDirectory} onClick={run}>{busy ? "Stress testing…" : "Run Challenge"}</button></div>
    </section>
    {result ? <ChallengeResult result={result} /> : <WorkspacePrimer title="No candidate challenged yet" copy="Choose one exact strategy IR and its broker-bound data. QuantForge will run purged folds, cost shocks, block-bootstrap Monte Carlo and local parameter perturbations." />}
    <PromotionLedgerPanels onError={onError} />
  </div>;
}

function ChallengeResult({ result }: { result: ChallengeView }) {
  return <section className={`panel result-panel result-${result.passed ? "pass" : "fail"}`}>
    <div className="result-hero"><div><p className="eyebrow">Challenge result</p><h2>{result.passed ? "Robustness gate passed" : "Candidate remains Illuminated"}</h2></div><span className="grade-pill">{result.grade}</span></div>
    <div className="job-kpis">
      <Kpi label="Validation trades" value={formatNumber(result.validationTrades)} note={`${formatNumber(result.returnPercent, 2)}% return`} />
      <Kpi label="Max drawdown" value={`${formatNumber(result.maximumDrawdownPercent, 2)}%`} note={`PF ${result.profitFactor === null ? "∞" : formatNumber(result.profitFactor, 2)}`} />
      <Kpi label="Purged folds" value={`${result.passingFolds} / ${result.totalFolds}`} note="Passing folds" />
      <Kpi label="Cost shocks" value={`${result.passingCostShocks} / ${result.totalCostShocks}`} note="Survived assumptions" />
    </div>
    <div className="partition-strip"><span>Development <strong>{formatNumber(result.developmentBars)}</strong></span><span>Validation <strong>{formatNumber(result.validationBars)}</strong></span><span>Sealed <strong>{formatNumber(result.sealedBars)}</strong></span></div>
    <ArtifactPath label="Challenge" value={result.challengePath} /><ArtifactPath label="Split plan" value={result.splitPlanPath} />
    {result.blockers.length > 0 && <div className="blocker-list">{result.blockers.map((blocker) => <span key={blocker}>{blocker}</span>)}</div>}
  </section>;
}

function PromotionLedgerPanels({ onError }: { onError: (message: string | null) => void }) {
  const [tab, setTab] = useState<"sealed" | "incubation">("sealed");
  return <section className="promotion-ledger">
    <div className="workspace-tabs"><button className={tab === "sealed" ? "active" : ""} onClick={() => setTab("sealed")}>2 · One-shot sealed final</button><button className={tab === "incubation" ? "active" : ""} onClick={() => setTab("incubation")}>3 · Paper incubation</button></div>
    {tab === "sealed" ? <SealedPanel onError={onError} /> : <IncubationPanel onError={onError} />}
  </section>;
}

function SealedPanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<SealedRequest>({ dataPath: "", metadataPath: null, sourceTimezone: "Etc/UTC", strategyPath: "", brokerPath: "", splitPlanPath: "", challengePath: "", sealedRoot: "", minimumTrades: 20, minimumReturnPercent: 1, minimumProfitFactor: 1.1, maximumDrawdownPercent: 20 });
  const [result, setResult] = useState<SealedView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof SealedRequest>(key: K, value: SealedRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await runSealedFinal(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content nested-tool"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">One attempt only</p><h2>Open the sealed partition</h2></div><span className="read-only-badge">Irreversible</span></div><div className="form-stack compact"><PathField label="Full OHLC source" path={form.dataPath} choose={chooseDataFile} onChange={(value) => update("dataPath", value)} required /><PathField label="Exporter metadata" path={form.metadataPath ?? ""} choose={chooseMetadataFile} onChange={(value) => setForm((current) => ({ ...current, metadataPath: value || null, sourceTimezone: value ? null : "Etc/UTC" }))} /><PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required /><PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required /><PathField label="Split plan" path={form.splitPlanPath} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => update("splitPlanPath", value)} required /><PathField label="Passing Challenge" path={form.challengePath} choose={() => chooseJsonFile("Choose Challenge artifact")} onChange={(value) => update("challengePath", value)} required /><PathField label="Sealed ledger root" path={form.sealedRoot} choose={() => chooseDirectory("Choose sealed ledger root")} onChange={(value) => update("sealedRoot", value)} required /></div><div className="numeric-grid"><NumberField label="Minimum trades" value={form.minimumTrades} onChange={(value) => update("minimumTrades", value ?? 20)} min={1} /><NumberField label="Minimum return %" value={form.minimumReturnPercent} onChange={(value) => update("minimumReturnPercent", value ?? 1)} step={.1} /><NumberField label="Minimum PF" value={form.minimumProfitFactor} onChange={(value) => update("minimumProfitFactor", value ?? 1.1)} step={.05} /><NumberField label="Maximum drawdown %" value={form.maximumDrawdownPercent} onChange={(value) => update("maximumDrawdownPercent", value ?? 20)} step={.1} /></div><div className="form-footer"><p>Access is durably claimed before market bars load. A crash or failed result still consumes this candidate/split attempt.</p><button className="danger" disabled={busy || !form.dataPath || !form.strategyPath || !form.brokerPath || !form.splitPlanPath || !form.challengePath || !form.sealedRoot} onClick={run}>{busy ? "Opening once…" : "Run sealed final once"}</button></div></section>{result ? <section className={`panel result-panel result-${result.passed ? "pass" : "fail"}`}><div className="result-hero"><div><p className="eyebrow">Sealed final</p><h2>{result.passed ? "Final holdout passed" : "Candidate demoted"}</h2></div><span className="grade-pill">{result.grade}</span></div><div className="job-kpis"><Kpi label="Trades" value={formatNumber(result.trades)} note={`${formatNumber(result.returnPercent, 2)}% return`} /><Kpi label="Drawdown" value={`${formatNumber(result.maximumDrawdownPercent, 2)}%`} note={`PF ${result.profitFactor === null ? "∞" : formatNumber(result.profitFactor, 2)}`} /></div><ArtifactPath label="Sealed artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="Sealed data remains closed" copy="Only run this after the candidate is shortlisted by a passing Challenge. The attempt is intentionally non-repeatable." />}</div>;
}

function IncubationPanel({ onError }: { onError: (message: string | null) => void }) {
  const today = new Date().toISOString().slice(0, 10);
  const [start, setStart] = useState<IncubationStartRequest>({ strategyPath: "", brokerPath: "", splitPlanPath: "", rootDirectory: "", startDate: today, initialBalance: "100000", maximumDailyLossPercent: "2", maximumTotalDrawdownPercent: "10", minimumObservationDays: 30, minimumTotalTrades: 20, maximumConsecutiveZeroTradeDays: 5 });
  const [startPath, setStartPath] = useState(""); const [record, setRecord] = useState({ date: today, endingBalance: 100000, maximumDrawdownPercent: 0, tradeCount: 0, note: "" }); const [view, setView] = useState<IncubationView | null>(null); const [busy, setBusy] = useState(false);
  async function open() { setBusy(true); onError(null); try { const next = await startIncubation(start); setView(next); setStartPath(next.startPath); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  async function append() { setBusy(true); onError(null); try { setView(await recordIncubation({ startPath, date: record.date, endingBalance: record.endingBalance, maximumDrawdownPercent: record.maximumDrawdownPercent, tradeCount: record.tradeCount, note: record.note || null })); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  async function finalize() { setBusy(true); onError(null); try { setView(await finalizeIncubation(startPath)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="incubation-grid"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Append-only ledger</p><h2>Open paper incubation</h2></div></div><div className="form-stack compact"><PathField label="Strategy IR" path={start.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => setStart((current) => ({ ...current, strategyPath: value }))} required /><PathField label="Broker profile" path={start.brokerPath} choose={chooseBrokerFile} onChange={(value) => setStart((current) => ({ ...current, brokerPath: value }))} required /><PathField label="Split plan" path={start.splitPlanPath} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => setStart((current) => ({ ...current, splitPlanPath: value }))} required /><PathField label="Incubation root" path={start.rootDirectory} choose={() => chooseDirectory("Choose incubation root")} onChange={(value) => setStart((current) => ({ ...current, rootDirectory: value }))} required /><label className="field-row"><span>Start date</span><input type="date" value={start.startDate} onChange={(event) => setStart((current) => ({ ...current, startDate: event.target.value }))} /></label></div><div className="numeric-grid"><NumberField label="Initial balance" value={Number(start.initialBalance)} onChange={(value) => setStart((current) => ({ ...current, initialBalance: String(value ?? 100000) }))} min={1} /><NumberField label="Daily loss limit %" value={Number(start.maximumDailyLossPercent)} onChange={(value) => setStart((current) => ({ ...current, maximumDailyLossPercent: String(value ?? 2) }))} step={.1} /><NumberField label="Total DD limit %" value={Number(start.maximumTotalDrawdownPercent)} onChange={(value) => setStart((current) => ({ ...current, maximumTotalDrawdownPercent: String(value ?? 10) }))} step={.1} /><NumberField label="Minimum days" value={start.minimumObservationDays} onChange={(value) => setStart((current) => ({ ...current, minimumObservationDays: value ?? 30 }))} min={1} /><NumberField label="Minimum trades" value={start.minimumTotalTrades} onChange={(value) => setStart((current) => ({ ...current, minimumTotalTrades: value ?? 20 }))} min={1} /></div><div className="form-footer"><p>Rules are frozen into the immutable start artifact.</p><button className="primary" disabled={busy || !start.strategyPath || !start.brokerPath || !start.splitPlanPath || !start.rootDirectory} onClick={open}>Open ledger</button></div></section><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Daily observation</p><h2>Record or finalize</h2></div><span className="job-status">{view?.status ?? "not open"}</span></div><div className="form-stack compact"><PathField label="Incubation start" path={startPath} choose={() => chooseJsonFile("Choose incubation-start.json")} onChange={setStartPath} required /><label className="field-row"><span>Observation date</span><input type="date" value={record.date} onChange={(event) => setRecord((current) => ({ ...current, date: event.target.value }))} /></label><label className="field-row"><span>Note</span><input value={record.note} onChange={(event) => setRecord((current) => ({ ...current, note: event.target.value }))} /></label></div><div className="numeric-grid"><NumberField label="Ending balance" value={record.endingBalance} onChange={(value) => setRecord((current) => ({ ...current, endingBalance: value ?? 100000 }))} min={1} /><NumberField label="Maximum DD %" value={record.maximumDrawdownPercent} onChange={(value) => setRecord((current) => ({ ...current, maximumDrawdownPercent: value ?? 0 }))} step={.1} /><NumberField label="Trades" value={record.tradeCount} onChange={(value) => setRecord((current) => ({ ...current, tradeCount: value ?? 0 }))} min={0} /></div><div className="form-footer"><button className="secondary" disabled={busy || !startPath || view?.status === "finalized"} onClick={append}>Append observation</button><button className="primary" disabled={busy || !startPath || view?.status === "finalized"} onClick={finalize}>Finalize ledger</button></div>{view && <div className="job-kpis"><Kpi label="Days" value={formatNumber(view.observationDays)} note="Recorded" /><Kpi label="Trades" value={formatNumber(view.totalTrades)} note="Cumulative" /><Kpi label="Return" value={view.returnPercent === null ? "open" : `${formatNumber(view.returnPercent, 2)}%`} note="Paper result" /><Kpi label="Status" value={view.passed === null ? "open" : view.passed ? "passed" : "failed"} note="Incubation gate" /></div>}{view?.finalPath && <ArtifactPath label="Incubation final" value={view.finalPath} />}</section></div>;
}

function ParityWorkspace({ onError }: { onError: (message: string | null) => void }) {
  const [tab, setTab] = useState<"judge" | "export" | "compare" | "indicators">("judge");
  return <div className="wide-tool-content">
    <div className="workspace-tabs"><button className={tab === "judge" ? "active" : ""} onClick={() => setTab("judge")}>1 · M1 Judge</button><button className={tab === "export" ? "active" : ""} onClick={() => setTab("export")}>2 · EA Export</button><button className={tab === "compare" ? "active" : ""} onClick={() => setTab("compare")}>3 · MT5 Compare</button><button className={tab === "indicators" ? "active" : ""} onClick={() => setTab("indicators")}>4 · Indicators</button></div>
    {tab === "judge" ? <JudgePanel onError={onError} /> : tab === "export" ? <ExportPanel onError={onError} /> : tab === "compare" ? <ParityComparePanel onError={onError} /> : <IndicatorParityPanel onError={onError} />}
  </div>;
}

function JudgePanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<JudgeRequest>({ decisionDataPath: "", decisionMetadataPath: null, decisionSourceTimezone: "Etc/UTC", m1DataPath: "", m1MetadataPath: null, m1SourceTimezone: "Etc/UTC", splitPlanPath: null, strategyPath: "", brokerPath: "", outputPath: "", commissionPerLotRoundTurn: 7, slippagePointsPerSide: 0, fallbackSpreadPoints: null, maxSpreadPoints: null, initialBalance: 100000 });
  const [result, setResult] = useState<JudgeView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof JudgeRequest>(key: K, value: JudgeRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await runM1Judge(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content">
    <section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Internal acceptance</p><h2>Replay decision signals through M1 chronology</h2></div></div>
      <div className="form-stack compact">
        <PathField label="Decision-timeframe OHLC" path={form.decisionDataPath} choose={chooseDataFile} onChange={(value) => update("decisionDataPath", value)} required />
        <PathField label="Decision metadata" path={form.decisionMetadataPath ?? ""} choose={chooseMetadataFile} onChange={(value) => setForm((current) => ({ ...current, decisionMetadataPath: value || null, decisionSourceTimezone: value ? null : "Etc/UTC" }))} />
        <PathField label="M1 OHLC" path={form.m1DataPath} choose={chooseDataFile} onChange={(value) => update("m1DataPath", value)} required />
        <PathField label="M1 metadata" path={form.m1MetadataPath ?? ""} choose={chooseMetadataFile} onChange={(value) => setForm((current) => ({ ...current, m1MetadataPath: value || null, m1SourceTimezone: value ? null : "Etc/UTC" }))} />
        <PathField label="Split plan (uses validation only)" path={form.splitPlanPath ?? ""} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => update("splitPlanPath", value || null)} />
        <PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required />
        <PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required />
        <PathField label="New Judge artifact" path={form.outputPath} choose={() => chooseOutputJson("Save Judge artifact", "judge.json")} onChange={(value) => update("outputPath", value)} required />
      </div>
      <div className="numeric-grid"><NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value ?? 0)} step={.01} /><NumberField label="Slippage / side" value={form.slippagePointsPerSide} onChange={(value) => update("slippagePointsPerSide", value ?? 0)} step={.1} /><NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value ?? 100000)} min={1} /></div>
      <div className="form-footer"><p>M1 files are OHLC bars, not ticks. Missing in-session minutes remain a hard failure.</p><button className="primary" disabled={busy || !form.decisionDataPath || !form.m1DataPath || !form.strategyPath || !form.brokerPath || !form.outputPath} onClick={run}>{busy ? "Replaying…" : "Run M1 Judge"}</button></div>
    </section>
    {result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">M1 Judge</p><h2>Execution chronology replayed</h2></div><span className="grade-pill">{result.grade}</span></div><div className="job-kpis"><Kpi label="Trades" value={formatNumber(result.trades)} note={`${formatNumber(result.returnPercent, 2)}% return`} /><Kpi label="Drawdown" value={`${formatNumber(result.maximumDrawdownPercent, 2)}%`} note={`PF ${result.profitFactor === null ? "∞" : formatNumber(result.profitFactor, 2)}`} /><Kpi label="Decision / M1 bars" value={`${formatNumber(result.decisionBars)} / ${formatNumber(result.m1Bars)}`} note="Chronology replayed" /><Kpi label="Managed exits" value={formatNumber(result.partialExits + result.endOfDayFlattens)} note={`${result.breakEvenMoves} BE · ${result.trailingMoves} trail`} /></div><ArtifactPath label="Judge artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No M1 replay yet" copy="Use matching decision-timeframe and M1 exports with their metadata files. The Judge enforces pending-order chronology and every active trade-management gene." />}
  </div>;
}

function ExportPanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<ExportRequest>({ strategyPath: "", brokerPath: "", outputDirectory: "", expertName: "QuantForgeStrategy", expertDirectory: "QuantForge", timeframe: "H1", magic: 42424242, deviationPoints: 10, maxSpreadPoints: null, slippagePointsPerSide: 0, commissionPerLotRoundTurn: 7, deposit: 100000, currency: "USD", leverage: 100, testerModel: 1 });
  const [result, setResult] = useState<ExportView | null>(null); const [busy, setBusy] = useState(false);
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

function EliteTable({
  rows,
  selected,
  onSelect,
}: {
  rows: EliteRow[];
  selected: string | null;
  onSelect: (fingerprint: string) => void;
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
        <span>Strategy</span><span>Family</span><span>Evidence</span><span>Novelty</span>
        <span>Trades</span><span>Return</span><span>DD</span><span>Parity</span>
      </div>
      <div
        className="elite-viewport"
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        style={{ height: viewportHeight }}
      >
        <div className="elite-virtual-body" style={{ height: rows.length * rowHeight }}>
          {rows.slice(start, end).map((row, offset) => (
            <button
              className={`elite-row ${selected === row.fingerprint ? "selected" : ""}`}
              key={row.fingerprint}
              onClick={() => onSelect(row.fingerprint)}
              style={{ height: rowHeight, transform: `translateY(${(start + offset) * rowHeight}px)` }}
            >
              <span className="strategy-cell"><strong>{row.strategyId}</strong><small>{row.fingerprint.slice(0, 10)}</small></span>
              <span>{row.family}</span>
              <span>{row.evidence.toFixed(2)}</span>
              <span>{row.novelty.toFixed(3)}</span>
              <span>{formatNumber(row.trades)}</span>
              <span className={row.returnPercent >= 0 ? "positive" : "negative"}>{row.returnPercent.toFixed(2)}%</span>
              <span>{row.drawdownPercent.toFixed(2)}%</span>
              <span className="parity-unknown">unknown</span>
            </button>
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
  tab,
  onTab,
}: {
  detail: EliteDetail | null;
  loading: boolean;
  onError: (message: string | null) => void;
  tab: "overview" | "ir";
  onTab: (tab: "overview" | "ir") => void;
}) {
  return (
    <aside className="panel inspector">
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
          </div>
          {tab === "overview" ? (
            <div className="inspector-body">
              <section>
                <p className="eyebrow">Thesis</p>
                <p>{detail.thesis}</p>
              </section>
              <Sparkline values={detail.equitySignature} />
              <section className="metric-list">
                <Metric label="Evidence" value={Number(detail.evidence.total).toFixed(2)} />
                <Metric label="Return" value={`${Number(detail.metrics.return_percent).toFixed(2)}%`} />
                <Metric label="Max drawdown" value={`${Number(detail.metrics.max_drawdown_percent).toFixed(2)}%`} />
                <Metric label="Trades" value={formatNumber(Number(detail.metrics.trade_count))} />
                <Metric label="Win rate" value={`${Number(detail.metrics.win_rate).toFixed(1)}%`} />
                <Metric label="Profit factor" value={detail.metrics.profit_factor === null ? "—" : Number(detail.metrics.profit_factor).toFixed(2)} />
              </section>
              <section>
                <p className="eyebrow">Behavioral niche</p>
                <code className="niche-code">{detail.niche}</code>
              </section>
              <div className="gate-stack">
                <span className="gate-pass">Discovery gates passed</span>
                <span className="gate-pending">Challenge not attached</span>
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

function Metric({ label, value }: { label: string; value: string }) {
  return <div><span>{label}</span><strong>{value}</strong></div>;
}

function Sparkline({ values }: { values: number[] }) {
  if (values.length < 2) return null;
  const minimum = Math.min(...values);
  const maximum = Math.max(...values);
  const range = Math.max(maximum - minimum, 1e-9);
  const points = values
    .map((value, index) => `${(index / (values.length - 1)) * 300},${70 - ((value - minimum) / range) * 60}`)
    .join(" ");
  return (
    <section className="sparkline-wrap">
      <div><p className="eyebrow">Equity signature</p><small>64-point archive preview</small></div>
      <svg viewBox="0 0 300 80" role="img" aria-label="Downsampled equity signature">
        <polyline points={points} fill="none" />
      </svg>
    </section>
  );
}

export default App;
