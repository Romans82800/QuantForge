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
  exportEliteEas,
  exportEliteTradeCsvs,
  getDiscoverJob,
  getDiscoverLiveDatabank,
  getElitePartitionEquity,
  getEliteMql5Source,
  inspectData,
  importMarketFolder,
  inspectVault,
  listSymbols,
  listDiscoverProfiles,
  listSearchRangeProfiles,
  saveDiscoverProfile,
  saveSearchRangeProfile,
  deleteDiscoverProfile,
  deleteSearchRangeProfile,
  loadDatabankPath,
  startHoldingBatteryJob,
  startProductionLaneJob,
  getHoldingBatteryJob,
  stopHoldingBattery,
  shrinkHoldingByDailyCorr,
  loadElite,
  pauseDiscover,
  recordIncubation,
  resumeDiscover,
  runConditionBakeoff,
  runTimeframeBakeoff,
  promoteEliteToVault,
  promoteHoldingWithoutRobustness,
  runFidelityDemo,
  runEliteRobustness,
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
  MarketFolderImportView,
  DeployRequest,
  DeployView,
  DiscoverJobView,
  DiscoverRequest,
  DiscoverRunModeId,
  BatteryJobView,
  EliteDetail,
  EliteMql5SourceView,
  EliteRow,
  FidelityDemoView,
  EliteSort,
  ExportRequest,
  ExportView,
  EvidenceView,
  ConditionBakeoffReport,
  TimeframeBakeoffReport,
  ConditionCoverage,
  JudgeRequest,
  JudgeView,
  IncubationStartRequest,
  IncubationView,
  IndicatorParityRequest,
  IndicatorParityView,
  ParityRequest,
  ParityView,
  PartitionEquityView,
  RobustnessEvidence,
  ResultsRobustnessMode,
  ResultsRobustnessView,
  TradeRowView,
  PortfolioRequest,
  PortfolioView,
  SearchRange,
  SearchRangeProfile,
  SavedDiscoverProfile,
  SavedSearchRangeProfile,
  SealedRequest,
  SealedView,
  SymbolPack,
  VaultView,
  WorkspaceName,
} from "./types";

async function writeClipboardText(value: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
    return;
  }
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.select();
  const copied = document.execCommand("copy");
  textarea.remove();
  if (!copied) throw new Error("clipboard access is unavailable");
}
import {
  bindDiscoverTimezone,
  cagrPercent,
  defaultCommissionPerLotRoundTurn,
  describeStrategyConditions,
  discoverDefaultsForSymbol,
  discoverProgress,
  discoverProgressLabel,
  filterAndSortElites,
  formatDateRange,
  conditionLabel,
  formatNumber,
  splitCaption,
  entryOrderError,
  entryOrderSummary,
  entryWindowError,
  entryWindowSummary,
  perturbationError,
  perturbationPercent,
  selectionBiasTone,
  symbolFromDataPath,
  timeframeFromDataPath,
  visibleEliteFingerprint,
} from "./view";

type NavIcon =
  | "overview"
  | "discover"
  | "databank"
  | "retester"
  | "data"
  | "portfolio"
  | "settings";

/**
 * Visible navigation rail. Vault and Deploy are intentionally absent: Vault is
 * now the Databank "Certified" tab and Deploy stays code-only, so no backend
 * command or workspace component is removed.
 */
const navigation: { name: WorkspaceName; label: string; icon: NavIcon }[] = [
  { name: "Home", label: "Overview", icon: "overview" },
  { name: "Discover", label: "Discover", icon: "discover" },
  { name: "Databank", label: "Databank", icon: "databank" },
  { name: "Parity Lab", label: "Retester", icon: "retester" },
  { name: "Data Lab", label: "Data Manager", icon: "data" },
  { name: "Portfolio", label: "Portfolio", icon: "portfolio" },
  { name: "Search Settings", label: "Settings", icon: "settings" },
];

const H1_COMPACT_SEARCH_RANGES: SearchRangeProfile = {
  indicatorPeriod: { minimum: 10, maximum: 20, step: 1 }, atrPeriod: { minimum: 10, maximum: 20, step: 1 },
  atrStopMultiple: { minimum: 1, maximum: 4, step: .25 }, atrTargetMultiple: { minimum: 1, maximum: 6, step: .5 },
  riskTargetMultiple: { minimum: .75, maximum: 4.5, step: .25 }, pendingDistanceAtr: { minimum: .25, maximum: 2, step: .25 },
  pendingExpiryBars: { minimum: 2, maximum: 8, step: 1 }, timeStopBars: { minimum: 4, maximum: 16, step: 1 },
  rsiUpper: { minimum: 52, maximum: 65, step: 1 }, rsiLower: { minimum: 20, maximum: 40, step: 1 }, adxThreshold: { minimum: 20, maximum: 35, step: 1 },
  rocThreshold: { minimum: .1, maximum: 2.5, step: .1 }, percentileLow: { minimum: 5, maximum: 25, step: 1 },
  zscoreThreshold: { minimum: 1, maximum: 2.5, step: .1 }, impulseBodyRatio: { minimum: .55, maximum: .75, step: .05 },
  impulseCloseLocation: { minimum: .7, maximum: .9, step: .05 }, atrPercentileMax: { minimum: 15, maximum: 35, step: 1 },
  atrPercentileLookback: { minimum: 20, maximum: 60, step: 20 },
  sessionStartHour: { minimum: 7, maximum: 14, step: 1 }, sessionRangeBars: { minimum: 2, maximum: 4, step: 1 },
  swingBars: { minimum: 2, maximum: 4, step: 1 }, baseBars: { minimum: 2, maximum: 4, step: 1 }, liquiditySweepThreshold: { minimum: 0, maximum: .5, step: .5 },
};

/** SQX-style random periods / parameters within reason. Wider than H1 compact; not forced. */
const SQX_RANDOM_SEARCH_RANGES: SearchRangeProfile = {
  indicatorPeriod: { minimum: 10, maximum: 50, step: 1 }, atrPeriod: { minimum: 7, maximum: 28, step: 1 },
  atrStopMultiple: { minimum: 1, maximum: 5, step: .25 }, atrTargetMultiple: { minimum: 1.5, maximum: 8, step: .5 },
  riskTargetMultiple: { minimum: 1, maximum: 5, step: .25 }, pendingDistanceAtr: { minimum: .25, maximum: 3, step: .25 },
  pendingExpiryBars: { minimum: 1, maximum: 12, step: 1 }, timeStopBars: { minimum: 2, maximum: 48, step: 1 },
  rsiUpper: { minimum: 55, maximum: 80, step: 1 }, rsiLower: { minimum: 20, maximum: 45, step: 1 }, adxThreshold: { minimum: 15, maximum: 40, step: 1 },
  rocThreshold: { minimum: .05, maximum: 5, step: .05 }, percentileLow: { minimum: 5, maximum: 30, step: 1 },
  zscoreThreshold: { minimum: .5, maximum: 3, step: .1 }, impulseBodyRatio: { minimum: .5, maximum: .85, step: .05 },
  impulseCloseLocation: { minimum: .6, maximum: .95, step: .05 }, atrPercentileMax: { minimum: 10, maximum: 50, step: 1 },
  atrPercentileLookback: { minimum: 20, maximum: 100, step: 10 },
  sessionStartHour: { minimum: 0, maximum: 20, step: 1 }, sessionRangeBars: { minimum: 1, maximum: 6, step: 1 },
  swingBars: { minimum: 2, maximum: 8, step: 1 }, baseBars: { minimum: 2, maximum: 8, step: 1 }, liquiditySweepThreshold: { minimum: 0, maximum: 1, step: .25 },
};

const DEFAULT_SEARCH_RANGES = H1_COMPACT_SEARCH_RANGES;

type BuiltInSearchPresetId = "h1_compact" | "sqx_random" | "custom";

const BUILT_IN_SEARCH_PRESETS: Array<{
  id: Exclude<BuiltInSearchPresetId, "custom">;
  label: string;
  note: string;
  ranges: SearchRangeProfile;
  maximumShift: number;
}> = [
  {
    id: "h1_compact",
    label: "H1 compact",
    note: "Tight periods 10–20 · ATR14 fixed · current QuantForge default",
    ranges: H1_COMPACT_SEARCH_RANGES,
    maximumShift: 3,
  },
  {
    id: "sqx_random",
    label: "SQX random",
    note: "Periods 10–50 · free ATR 7–28 · wider stops/targets within reason",
    ranges: SQX_RANDOM_SEARCH_RANGES,
    maximumShift: 5,
  },
];

function rangesMatch(a: SearchRangeProfile, b: SearchRangeProfile): boolean {
  return (Object.keys(a) as Array<keyof SearchRangeProfile>).every((key) => {
    const left = a[key];
    const right = b[key];
    return left.minimum === right.minimum && left.maximum === right.maximum && left.step === right.step;
  });
}

function detectBuiltInSearchPreset(ranges: SearchRangeProfile): BuiltInSearchPresetId {
  for (const preset of BUILT_IN_SEARCH_PRESETS) {
    if (rangesMatch(ranges, preset.ranges)) return preset.id;
  }
  return "custom";
}

// Search profiles are stored by Rust in snake_case while the TypeScript UI uses
// camelCase. Accept both forms so profiles saved by any installed version stay
// loadable instead of allowing a missing range row to crash the settings wall.
function normalizeSearchRanges(value: SearchRangeProfile | Record<string, unknown>): SearchRangeProfile {
  const source = value as Record<string, unknown>;
  const keys = Object.keys(DEFAULT_SEARCH_RANGES) as Array<keyof SearchRangeProfile>;
  const normalized = Object.fromEntries(keys.map((key) => {
    const snake = key.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
    const candidate = source[key] ?? source[snake];
    const fallback = DEFAULT_SEARCH_RANGES[key];
    const range = candidate && typeof candidate === "object" ? candidate as Partial<SearchRange> : fallback;
    const valid = [range.minimum, range.maximum, range.step].every((item) => typeof item === "number" && Number.isFinite(item));
    return [key, valid ? { minimum: range.minimum!, maximum: range.maximum!, step: range.step! } : { ...fallback }];
  }));
  return normalized as unknown as SearchRangeProfile;
}

const workspaceTitles: Record<WorkspaceName, string> = {
  Home: "Overview",
  Databank: "Databank",
  Discover: "Discover",
  "Search Settings": "Settings",
  Vault: "Certified",
  "Data Lab": "Data Manager",
  "Parity Lab": "Retester",
  Portfolio: "Portfolio",
  Deploy: "Deployment",
};

function navLabel(name: WorkspaceName): string {
  return navigation.find((item) => item.name === name)?.label ?? workspaceTitles[name];
}

const NAV_GLYPHS: Record<NavIcon, string> = {
  overview: "M4 11.5 12 4.5l8 7M6 10.5V19h12v-8.5",
  discover: "M10.5 4.5a6 6 0 1 0 0 12 6 6 0 0 0 0-12ZM15 15l4.5 4.5",
  databank: "M4.5 6.5h15M4.5 12h15M4.5 17.5h15M9 4v16",
  retester: "M5 19V9m4.7 10V5m4.6 14v-7m4.7 7V8",
  data: "M12 4c4 0 7 1.2 7 2.7S16 9.4 12 9.4 5 8.2 5 6.7 8 4 12 4Zm7 5v8.3c0 1.5-3 2.7-7 2.7s-7-1.2-7-2.7V9",
  portfolio: "M4.5 8.5h15v11h-15zM9 8.5V6a3 3 0 0 1 6 0v2.5",
  settings:
    "M12 9.2a2.8 2.8 0 1 0 0 5.6 2.8 2.8 0 0 0 0-5.6ZM12 3.5v2.2M12 18.3v2.2M5.5 12H3.3m17.4 0h-2.2M7.4 7.4 5.9 5.9m12.2 12.2-1.5-1.5M16.6 7.4l1.5-1.5M5.9 18.1l1.5-1.5",
};

function NavGlyph({ icon }: { icon: NavIcon }) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" className="nav-glyph">
      <path d={NAV_GLYPHS[icon]} />
    </svg>
  );
}

const DEFAULT_UNIVERSAL_GRAMMAR = {
  minimumEntryConditions: 2,
  maximumEntryConditions: 2,
  minimumExitConditions: 1,
  maximumExitConditions: 3,
  minimumShift: 1,
  maximumShift: 3,
};

function App() {
  const [activeWorkspace, setActiveWorkspace] = useState<WorkspaceName>("Home");
  const [workspace, setWorkspace] = useState<DatabankWorkspace | null>(null);
  const [detail, setDetail] = useState<EliteDetail | null>(null);
  const [loading, setLoading] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [entryFilter, setEntryFilter] = useState("all");
  const [sort, setSort] = useState<EliteSort>("evidence");
  const [detailTab, setDetailTab] = useState<"overview" | "ir" | "mq5">("overview");
  const [discoverPreset, setDiscoverPreset] = useState<Partial<DiscoverRequest>>({});
  const [judgePreset, setJudgePreset] = useState<Partial<JudgeRequest>>({});
  const [exportPreset, setExportPreset] = useState<Partial<ExportRequest>>({});
  const [batchSelection, setBatchSelection] = useState<Set<string>>(new Set());
  const [batchSize, setBatchSize] = useState(10);
  const [batchMessage, setBatchMessage] = useState<string | null>(null);
  const [batchBusy, setBatchBusy] = useState(false);
  const [batchEaTimeframe, setBatchEaTimeframe] = useState("H1");
  const [batchEaMagic, setBatchEaMagic] = useState(42_424_242);
  const [eliteModalOpen, setEliteModalOpen] = useState(false);
  const [resultsDetailFp, setResultsDetailFp] = useState<string | null>(null);
  const [resultsOrigin, setResultsOrigin] = useState<WorkspaceName>("Databank");
  const [databankTab, setDatabankTab] = useState<"holding" | "databank" | "certified">("holding");
  const [holdingControlsOpen, setHoldingControlsOpen] = useState(false);
  const [batteryJob, setBatteryJob] = useState<BatteryJobView | null>(null);
  const [batteryBusy, setBatteryBusy] = useState(false);
  const [holdingCorrCap, setHoldingCorrCap] = useState(0.5);
  const [holdingShrinkBusy, setHoldingShrinkBusy] = useState(false);
  const [factoryQueueLimit, setFactoryQueueLimit] = useState(0);
  const [factoryTargetDatabank, setFactoryTargetDatabank] = useState(0);
  const lastBatteryRevision = useRef(0);
  const [discoverResultsOpen, setDiscoverResultsOpen] = useState(false);
  const [stripMessage, setStripMessage] = useState<string | null>(null);
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

  async function openEliteModal(fingerprint: string) {
    setDetailTab("overview");
    setEliteModalOpen(true);
    await selectElite(fingerprint);
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
      selectedSymbol: report.symbol ?? symbolFromDataPath(report.sourcePath),
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

  function selectAllElites() {
    setBatchMessage(null);
    setBatchSelection(new Set(filtered.map((row) => row.fingerprint)));
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

  async function exportSelectedEas() {
    setBatchBusy(true);
    setBatchMessage(null);
    setError(null);
    try {
      const result = await exportEliteEas(
        [...batchSelection],
        batchEaTimeframe,
        batchEaMagic,
      );
      if (result) {
        setBatchMessage(
          `Exported ${result.expertPaths.length} MQ5 experts to ${result.directory}. Live trading is disabled in every EA.`,
        );
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatchBusy(false);
    }
  }

  async function exportSelectedTradeCsvs() {
    setBatchBusy(true);
    setBatchMessage(null);
    setError(null);
    try {
      const result = await exportEliteTradeCsvs([...batchSelection]);
      if (result) {
        setBatchMessage(
          `Exported ${result.csvPaths.length} SQX-style M1 trade CSV files and a strategy index to ${result.directory}`,
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


  async function exportIr(fingerprint: string) {
    setStripMessage(null);
    setError(null);
    try {
      const path = await exportEliteStrategy(fingerprint);
      if (path) setStripMessage(`Exported IR → ${path}`);
    } catch (reason) {
      setError(String(reason));
    }
  }

  async function exportMq5(fingerprint: string) {
    setStripMessage(null);
    setError(null);
    setBatchBusy(true);
    try {
      const result = await exportEliteEas([fingerprint], batchEaTimeframe, batchEaMagic);
      if (result) {
        setStripMessage(
          `Exported ${result.expertPaths.length} MQ5 expert(s) → ${result.directory}`,
        );
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatchBusy(false);
    }
  }

  async function stageForCertification(fingerprint: string) {
    setStripMessage(null);
    setError(null);
    try {
      const path = await promoteEliteToVault(fingerprint);
      if (path) setStripMessage(`Staged for certification → ${path}`);
    } catch (reason) {
      setError(String(reason));
    }
  }

  /**
   * Opens the Results detail page without moving the user to another workspace,
   * so Back returns to wherever the row was clicked (Databank, Discover, …).
   */
  async function openResultsDetail(fingerprint: string, origin?: WorkspaceName) {
    setResultsOrigin(origin ?? activeWorkspace);
    setResultsDetailFp(fingerprint);
    setError(null);
    await selectElite(fingerprint);
  }

  const archiveRows = useMemo(() => {
    if (!workspace) return [];
    if (databankTab === "holding") return workspace.holding ?? [];
    return workspace.elites;
  }, [workspace, databankTab]);

  const batteryActive =
    batteryJob?.status === "running"
    && (batteryJob.total === 0
      || batteryJob.completed < batteryJob.total
      || batteryJob.running > 0);

  // Long Holding batteries can be thousands of rows — keep a short live feed.
  const batteryActivity = useMemo(() => {
    const items = batteryJob?.items ?? [];
    const running = items.filter((item) => item.status === "running");
    const finished = items.filter(
      (item) => item.status === "passed"
        || item.status === "rejected"
        || item.status === "selected"
        || item.status === "eligible",
    );
    const limit = 12;
    const recentFinished = finished.slice(-limit).reverse();
    const queuedCount = items.filter((item) => item.status === "queued").length;
    return {
      rows: [...running, ...recentFinished],
      queuedCount,
      finishedCount: finished.length,
      hiddenFinished: Math.max(0, finished.length - recentFinished.length),
    };
  }, [batteryJob?.items, batteryJob?.revision]);

  useEffect(() => {
    const pull = () => {
      void getHoldingBatteryJob()
        .then(setBatteryJob)
        .catch(() => undefined);
    };
    pull();
    const timer = window.setInterval(pull, 2000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!batteryActive && batteryJob?.status !== "running") return;
    const timer = window.setInterval(() => {
      void getHoldingBatteryJob()
        .then((view) => {
          const done =
            view.status === "completed"
            || view.status === "stopped"
            || view.status === "failed"
            || (view.total > 0 && view.completed >= view.total && view.running === 0);
          setBatteryJob(
            done && view.status === "running"
              ? {
                  ...view,
                  status: "completed",
                  phase: "Battery complete",
                  running: 0,
                  queued: 0,
                  etaSeconds: 0,
                }
              : view,
          );
          // A battery checkpoint includes a fresh workspace after every pass,
          // so Databank updates strategy-by-strategy rather than only when the
          // complete queue finishes.
          if (view.workspace && view.revision !== lastBatteryRevision.current) {
            setWorkspace(view.workspace);
            lastBatteryRevision.current = view.revision;
            if (done && view.jobKind === "production_lane") {
              setDatabankTab("databank");
              setQuery("");
              setEntryFilter("all");
              setBatchSelection(new Set());
              setDetail(null);
            }
          }
          const shouldReloadArchive =
            !!view.databankPath
            && !view.workspace
            && view.revision !== lastBatteryRevision.current
            && done;
          if (shouldReloadArchive && view.databankPath) {
            lastBatteryRevision.current = view.revision;
            void loadDatabankPath(view.databankPath)
              .then(setWorkspace)
              .catch((reason) => setError(String(reason)));
          } else if (view.revision !== lastBatteryRevision.current) {
            lastBatteryRevision.current = view.revision;
          }
        })
        .catch((reason) => setError(String(reason)));
    }, 350);
    return () => window.clearInterval(timer);
  }, [batteryActive, batteryJob?.status]);

  useEffect(() => {
    if (!batteryJob) return;
    if (batteryJob.status !== "completed" && batteryJob.status !== "stopped") return;
    if (batteryJob.workspace) {
      setWorkspace(batteryJob.workspace);
      if (batteryJob.jobKind === "production_lane") {
        switchDatabankTab("databank");
        setDetail(null);
      }
      return;
    }
    if (!batteryJob.databankPath) return;
    void loadDatabankPath(batteryJob.databankPath)
      .then(setWorkspace)
      .catch((reason) => setError(String(reason)));
  }, [batteryJob?.status, batteryJob?.databankPath, batteryJob?.workspace]);

  async function startBatteryOnSelection() {
    const fingerprints = [...batchSelection];
    if (fingerprints.length === 0) return;
    setBatteryBusy(true);
    setError(null);
    try {
      const started = await startHoldingBatteryJob(fingerprints);
      lastBatteryRevision.current = started.revision;
      setBatteryJob(started);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatteryBusy(false);
    }
  }

  async function startHoldingFactory() {
    if ((workspace?.holding?.length ?? 0) === 0) return;
    setBatteryBusy(true);
    setError(null);
    try {
      const started = await startHoldingBatteryJob([], {
        ranked: true,
        // Full battery works from the visible Holding cohort. Correlation
        // shrinking is a separate, deliberate action; it must not silently
        // remove names when the user starts the battery.
        shrinkFirst: false,
        queueLimit: factoryQueueLimit,
        targetDatabank: factoryTargetDatabank,
      });
      lastBatteryRevision.current = started.revision;
      setBatteryJob(started);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatteryBusy(false);
    }
  }

  async function runProductionLane() {
    const holdingCount = workspace?.holding?.length ?? 0;
    if (holdingCount === 0) return;
    const confirmed = window.confirm(
      `Run Production Lane v1 on all ${holdingCount} frozen Holding strategies?\n\nOnly H4 Development data will be used for 3/6/12-month stability and expectancy × √trades ranking. The sealed final partition will stay unopened. Selected strategies will move to Databank; the rest remain in Holding.`,
    );
    if (!confirmed) return;
    setBatteryBusy(true);
    setError(null);
    try {
      const started = await startProductionLaneJob();
      lastBatteryRevision.current = started.revision;
      setBatteryJob(started);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatteryBusy(false);
    }
  }

  async function graduateAllHoldingWithoutRobustness() {
    const holdingCount = workspace?.holding?.length ?? 0;
    if (holdingCount === 0) return;
    const confirmed = window.confirm(
      `Graduate all ${holdingCount} Holding strategies into Databank without running calendar folds, walk-forward/CPCV, Monte Carlo or parameter-neighborhood tests?\n\nThey keep the basic and M1-fidelity checks already completed during Holding admission. This changes the archive and cannot be described as robustness-passed.`,
    );
    if (!confirmed) return;
    setBatteryBusy(true);
    setError(null);
    try {
      const result = await promoteHoldingWithoutRobustness();
      setWorkspace(result.workspace);
      setBatchSelection(new Set());
      setBatchMessage(
        `Graduated ${result.promoted + result.replaced} Holding strategies without the robustness battery. ${result.promoted} added and ${result.replaced} replaced matching Databank entries.`,
      );
      setDatabankTab("databank");
      setQuery("");
      setEntryFilter("all");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatteryBusy(false);
    }
  }

  function switchDatabankTab(tab: "holding" | "databank" | "certified") {
    setDatabankTab(tab);
    setQuery("");
    setEntryFilter("all");
    setBatchSelection(new Set());
  }

  async function stopBatteryJob() {
    setBatteryBusy(true);
    try {
      setBatteryJob(await stopHoldingBattery());
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBatteryBusy(false);
    }
  }

  async function shrinkHoldingByCorr() {
    const holdingCount = workspace?.holding?.length ?? 0;
    if (holdingCount === 0) return;
    const confirmed = window.confirm(
      `Replay Holding on Development H1, keep the stronger name of each pair whose daily P/L correlation is above ${holdingCorrCap}, and write the archive?\n\n${holdingCount} names in Holding now. This is not a Discover setting and does not run the battery.`,
    );
    if (!confirmed) return;
    setHoldingShrinkBusy(true);
    setError(null);
    try {
      const result = await shrinkHoldingByDailyCorr(holdingCorrCap);
      setWorkspace(result.workspace);
      setBatchSelection(new Set());
      setBatchMessage(
        `Holding shrink at max daily P/L corr ${holdingCorrCap}: kept ${result.kept}, dropped ${result.dropped} (${result.replayed} H1 replays).`,
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setHoldingShrinkBusy(false);
    }
  }

  const filtered = useMemo(
    () => filterAndSortElites(archiveRows, query, entryFilter, sort),
    [archiveRows, query, entryFilter, sort],
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
      <aside className="rail" aria-label="QuantForge">
        <div className="rail-brand" title="QuantForge">
          <span className="rail-monogram" aria-hidden="true">QF</span>
          <span className="rail-brand-label">QuantForge</span>
        </div>
        <nav className="rail-nav" aria-label="Workspaces">
          {navigation.map(({ name, label, icon }) => (
            <button
              aria-current={name === activeWorkspace ? "page" : undefined}
              className={name === activeWorkspace ? "rail-item active" : "rail-item"}
              key={name}
              onClick={() => {
                setError(null);
                setResultsDetailFp(null);
                setActiveWorkspace(name);
              }}
              title={label}
              type="button"
            >
              <NavGlyph icon={icon} />
              <span>{label}</span>
            </button>
          ))}
        </nav>
        <div className="rail-foot" title="Rust engine runs locally">
          <span className="status-light" />
          <span>Local</span>
        </div>
      </aside>

      <div className="app-main-column">
        <div className="app-main-scroll">
      <main>
        <header className="topbar">
          <nav className="crumbs" aria-label="Breadcrumb">
            <span>Strategies</span>
            <i aria-hidden="true" />
            {resultsDetailFp ? (
              <>
                <button type="button" className="crumb-link" onClick={() => setResultsDetailFp(null)}>
                  {navLabel(resultsOrigin)}
                </button>
                <i aria-hidden="true" />
                <strong>Results detail</strong>
              </>
            ) : (
              <strong>{workspaceTitles[activeWorkspace]}</strong>
            )}
          </nav>
          <div className="topbar-actions">
            {workspace && (
              <span className="topbar-chip" title={workspace.sourcePath}>
                {formatNumber((workspace.holding?.length ?? 0) + workspace.elites.length)} strategies · {workspace.sourcePath.split("/").at(-1)}
              </span>
            )}
            {activeWorkspace === "Databank" && (
              <button className="primary" onClick={openDatabank} disabled={loading}>
                {loading ? "Validating…" : workspace ? "Open another" : "Open databank"}
              </button>
            )}
            <div className="topbar-utilities" aria-label="Application status">
              <span className="topbar-icon" title="QuantForge notifications">
                <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M7.5 17h9l-1.2-1.8V11a3.3 3.3 0 0 0-6.6 0v4.2L7.5 17Zm3 2h3" /></svg>
              </span>
              <span className="topbar-icon" title="Research methodology">
                <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="7.5" /><path d="M10.4 9.6a1.8 1.8 0 0 1 3.4.8c0 1.6-1.8 1.7-1.8 3.1m0 2.6v.1" /></svg>
              </span>
              <span className="topbar-icon" title="Dark workspace">
                <svg aria-hidden="true" viewBox="0 0 24 24"><circle cx="12" cy="12" r="3.2" /><path d="M12 3.3v2M12 18.7v2M3.3 12h2M18.7 12h2M5.9 5.9l1.4 1.4m9.4 9.4 1.4 1.4m0-12.2-1.4 1.4M7.3 16.7l-1.4 1.4" /></svg>
              </span>
              <span className="topbar-divider" />
              <span className="user-avatar">DA</span>
              <span className="user-copy">
                <strong>Daniel</strong>
                <small>Local</small>
              </span>
              <span className="user-badge">Pro</span>
            </div>
          </div>
        </header>

        {error && (
          <div className="error-banner" role="alert">
            <strong>Operation failed</strong>
            <span>{error}</span>
          </div>
        )}
        {stripMessage && !error && (
          <div className="success-banner" role="status">
            <span>{stripMessage}</span>
            <button type="button" onClick={() => setStripMessage(null)} aria-label="Dismiss notification">×</button>
          </div>
        )}
        {resultsDetailFp ? (
          <ResultsDetailPage
            fingerprint={resultsDetailFp}
            workspace={workspace}
            onError={setError}
            onExportIr={(target) => void exportIr(target)}
            onExportMq5={(target) => void exportMq5(target)}
            onStage={(target) => void stageForCertification(target)}
            onOpenResult={(target) => void openResultsDetail(target, resultsOrigin)}
          />
        ) : (
          <>
        {activeWorkspace === "Home" ? (
          <HomeWorkspace
            workspace={workspace}
            onNavigate={setActiveWorkspace}
            onOpen={openDatabank}
            onOpenPath={openDatabankPath}
            onOpenResult={(fingerprint) => void openResultsDetail(fingerprint, "Home")}
            onError={setError}
          />
        ) : activeWorkspace === "Data Lab" ? (
          <DataLabWorkspace onError={setError} onUse={useDataInDiscover} />
        ) : activeWorkspace === "Discover" ? (
          <DiscoverWorkspace
            onError={setError}
            onOpenDatabank={openDatabankPath}
            onLiveWorkspace={(loaded) => {
              setWorkspace(loaded);
              setBatchSelection(new Set());
            }}
            onResultsView={setDiscoverResultsOpen}
            preset={discoverPreset}
          />
        ) : activeWorkspace === "Search Settings" ? (
          <SearchSettingsWorkspace onError={setError} />
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
        ) : activeWorkspace === "Databank" ? (
          <div className="databank-workspace">
            <div className="workspace-tabs databank-tabs" role="tablist" aria-label="Databank sections">
              <button
                type="button"
                role="tab"
                aria-selected={databankTab === "holding"}
                className={databankTab === "holding" ? "active" : ""}
                onClick={() => switchDatabankTab("holding")}
              >
                Holding{workspace ? ` (${workspace.holding?.length ?? 0})` : ""}
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={databankTab === "databank"}
                className={databankTab === "databank" ? "active" : ""}
                onClick={() => switchDatabankTab("databank")}
              >
                Databank{workspace ? ` (${workspace.elites.length})` : ""}
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={databankTab === "certified"}
                className={databankTab === "certified" ? "active" : ""}
                onClick={() => switchDatabankTab("certified")}
              >
                Certified
              </button>
            </div>
            {databankTab === "certified" ? (
              <VaultWorkspace onError={setError} />
            ) : !workspace ? (
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
                  <dt>Split</dt>
                  <dd>
                    {splitCaption(workspace.validationFraction, workspace.sealedFraction)}
                  </dd>
                </div>
                <div>
                  <dt>Grade</dt>
                  <dd>{workspace.researchGrade ? "research (Selected-TF scout)" : workspace.m1FidelityVerified ? "M1 verified" : "promotion"}</dd>
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
                    {workspace.conditionGroups.map((item) => (
                      <CoverageGrid
                        group={item}
                        key={item.entryConditions}
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
                  rows={archiveRows}
                  selected={detail?.fingerprint ?? null}
                  onSelect={selectElite}
                />

                <EquityCurvePanel
                  rows={archiveRows.filter((row) =>
                    batchSelection.size > 0
                      ? batchSelection.has(row.fingerprint)
                      : row.fingerprint === detail?.fingerprint,
                  )}
                  initialBalance={workspace.initialBalance}
                  m1FidelityVerified={workspace.m1FidelityVerified}
                />

                {(batteryJob && batteryJob.status !== "idle") && (
                  <section className="panel" aria-label="Holding job progress">
                    <div className="panel-heading">
                      <div>
                        <p className="eyebrow">
                          {batteryJob.jobKind === "production_lane"
                            ? "H4 Production Lane v1"
                            : "Holding battery"}
                        </p>
                        <h2>{batteryJob.phase}</h2>
                      </div>
                      <span className="read-only-badge">{batteryJob.status}</span>
                    </div>
                    <p>{batteryJob.message}</p>
                    {(batteryJob.holdingBeforeShrink ?? 0) > 0 && batteryJob.jobKind !== "production_lane" && (
                      <p className="muted">
                        Funnel {formatNumber(batteryJob.holdingBeforeShrink ?? 0)} Holding
                        → {formatNumber(batteryJob.holdingAfterShrink ?? batteryJob.holdingBeforeShrink ?? 0)} ready
                        → {formatNumber(batteryJob.total)} queued
                        → {formatNumber(batteryJob.passed)} Databank
                        {batteryJob.targetDatabank
                          ? ` (stop at ${formatNumber(batteryJob.targetDatabank)})`
                          : ""}
                      </p>
                    )}
                    <div className="kpi-grid" aria-label="Battery counters">
                      <Kpi
                        label="Progress"
                        value={`${formatNumber(batteryJob.completed)}/${formatNumber(batteryJob.total)}`}
                        note={`${formatNumber(batteryJob.queued)} queued · ${formatNumber(batteryJob.running)} running`}
                      />
                      <Kpi
                        label={batteryJob.jobKind === "production_lane" ? "Selected → Databank" : "Passed → Databank"}
                        value={formatNumber(batteryJob.passed)}
                        note={`${formatNumber(batteryJob.databankElites)} databank elites`}
                      />
                      <Kpi
                        label="Rejected"
                        value={formatNumber(batteryJob.rejected)}
                        note={`${formatNumber(batteryJob.holdingRemaining)} still in Holding`}
                      />
                      <Kpi
                        label={batteryJob.jobKind === "production_lane" ? "Replays / hour" : "Batteries / hour"}
                        value={formatNumber(batteryJob.batteriesPerHour, 1)}
                        note={
                          batteryJob.etaSeconds != null && batteryJob.status === "running"
                            ? `ETA ~${Math.max(1, Math.round(batteryJob.etaSeconds / 60))} min`
                            : `${formatNumber(batteryJob.elapsedSeconds, 0)}s elapsed`
                        }
                      />
                    </div>
                    {batteryJob.jobKind !== "production_lane" && batteryJob.killMix && batteryJob.rejected > 0 && (
                      <div className="kpi-grid" aria-label="Battery kill mix">
                        <Kpi label="Neighborhood / RetDD" value={formatNumber(batteryJob.killMix.neighborhood)} note="0.85–1.25 of median" />
                        <Kpi label="Monte Carlo" value={formatNumber(batteryJob.killMix.monteCarlo)} note="P80 profit retention" />
                        <Kpi label="Folds" value={formatNumber(batteryJob.killMix.folds)} note="CPCV / walk-forward / years" />
                        <Kpi
                          label="Other kills"
                          value={formatNumber(
                            batteryJob.killMix.m1
                              + batteryJob.killMix.deposit
                              + batteryJob.killMix.expectancy
                              + batteryJob.killMix.oos1
                              + batteryJob.killMix.other,
                          )}
                          note={`M1 ${batteryJob.killMix.m1} · deposit ${batteryJob.killMix.deposit}`}
                        />
                      </div>
                    )}
                    {batteryActive && (
                      <div className="form-footer">
                        <button
                          type="button"
                          className="secondary"
                          disabled={batteryBusy || batteryJob?.stopRequested}
                          onClick={() => void stopBatteryJob()}
                        >
                          {batteryJob?.stopRequested ? "Stopping…" : "Stop safely"}
                        </button>
                      </div>
                    )}
                    {batteryJob.reportPath && (
                      <p className="muted" title={batteryJob.reportPath}>
                        Immutable report: {batteryJob.reportPath}
                      </p>
                    )}
                    {batteryActivity.rows.length > 0 && (
                      <div className="family-tester-results">
                        <p className="muted" style={{ marginBottom: "0.5rem" }}>
                          Latest activity
                          {batteryActivity.hiddenFinished > 0
                            ? ` · ${formatNumber(batteryActivity.hiddenFinished)} earlier finishes hidden`
                            : ""}
                          {batteryActivity.queuedCount > 0
                            ? ` · ${formatNumber(batteryActivity.queuedCount)} queued`
                            : ""}
                        </p>
                        <table>
                          <thead>
                            <tr>
                              <th>Status</th>
                              <th>Strategy</th>
                              <th>Evidence</th>
                              <th>Trades</th>
                              <th>Reason</th>
                            </tr>
                          </thead>
                          <tbody>
                            {batteryActivity.rows.map((item) => (
                              <tr key={item.fingerprint}>
                                <td>{item.status}</td>
                                <td title={item.fingerprint}>{item.strategyId}</td>
                                <td>{formatNumber(item.evidence, 1)}</td>
                                <td>{formatNumber(item.trades)}</td>
                                <td>{item.reason ?? "—"}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </section>
                )}

                <section className="panel elites-panel">
                  <div className="panel-heading table-heading">
                    <div>
                      <p className="eyebrow">
                        {databankTab === "holding"
                          ? "Untested M1 survivors"
                          : "Selected research elites"}
                      </p>
                      <h2>
                        {filtered.length === archiveRows.length
                          ? formatNumber(archiveRows.length)
                          : `${formatNumber(filtered.length)} of ${formatNumber(archiveRows.length)}`}{" "}
                        {databankTab === "holding" ? "holding" : "elites"}
                      </h2>
                    </div>
                    <div className="table-controls">
                      {databankTab === "holding" && (
                        <>
                          {workspace?.decisionTimeframe === "H4" && <button
                            type="button"
                            className="primary"
                            disabled={
                              loading
                              || batteryBusy
                              || batteryActive
                              || holdingShrinkBusy
                              || (workspace?.holding?.length ?? 0) === 0
                            }
                            onClick={() => void runProductionLane()}
                            title="Fixed H4 Development-only recipe: require 6/12-month stability, rank by expectancy × √trades, and promote the top 20% under diversity limits. Sealed final stays unopened."
                          >
                            {batteryActive && batteryJob?.jobKind === "production_lane"
                              ? "Production Lane running…"
                              : `Run Production Lane v1 (${workspace?.holding?.length ?? 0})`}
                          </button>}
                          <button
                            type="button"
                            className="secondary"
                            disabled={
                              loading
                              || batteryBusy
                              || batteryActive
                              || holdingShrinkBusy
                              || (workspace?.holding?.length ?? 0) === 0
                            }
                            onClick={() => void startHoldingFactory()}
                            title="Keep the current Holding cohort, rank by trades × R-expectancy, then run the full battery. Each pass moves to Databank immediately; failed strategies remain in Holding."
                          >
                            {batteryActive && batteryJob?.jobKind !== "production_lane"
                              ? "Factory running…"
                              : "Run full robustness battery"}
                          </button>
                          <button
                            type="button"
                            className="secondary"
                            disabled={
                              loading
                              || batteryBusy
                              || batteryActive
                              || holdingShrinkBusy
                              || workspace?.legacyReadOnly
                              || (workspace?.holding?.length ?? 0) === 0
                            }
                            onClick={() => void graduateAllHoldingWithoutRobustness()}
                            title="Move every current Holding strategy to Databank without the deferred robustness battery."
                          >
                            {batteryBusy
                              ? "Working…"
                              : `Graduate all · skip robustness (${workspace?.holding?.length ?? 0})`}
                          </button>
                          <button
                            type="button"
                            className={holdingControlsOpen ? "secondary active" : "secondary"}
                            onClick={() => setHoldingControlsOpen((current) => !current)}
                          >
                            {holdingControlsOpen ? "Hide advanced controls" : "Advanced controls"}
                          </button>
                          {holdingControlsOpen && (
                            <div className="holding-advanced-controls">
                              <label>
                                Max daily P/L corr
                                <select
                                  aria-label="Maximum daily P/L correlation"
                                  disabled={loading || holdingShrinkBusy || batteryBusy || batteryActive}
                                  onChange={(event) => setHoldingCorrCap(Number(event.target.value))}
                                  value={holdingCorrCap}
                                >
                                  <option value={0.3}>0.3</option>
                                  <option value={0.4}>0.4</option>
                                  <option value={0.5}>0.5</option>
                                  <option value={0.6}>0.6</option>
                                </select>
                              </label>
                              <button
                                type="button"
                                className="secondary"
                                disabled={
                                  loading
                                  || holdingShrinkBusy
                                  || batteryBusy
                                  || batteryActive
                                  || (workspace?.holding?.length ?? 0) === 0
                                }
                                onClick={() => void shrinkHoldingByCorr()}
                                title="Drop correlated Holding clones using Development H1 daily P/L. Not a Discover start setting."
                              >
                                {holdingShrinkBusy ? "Shrinking…" : "Shrink Holding"}
                              </button>
                              <label>
                                Queue
                                <input
                                  aria-label="Factory queue limit"
                                  min={0}
                                  max={10000}
                                  onChange={(event) => setFactoryQueueLimit(Math.max(0, Number(event.target.value) || 0))}
                                  type="number"
                                  value={factoryQueueLimit}
                                />
                              </label>
                              <label>
                                Databank target
                                <input
                                  aria-label="Factory Databank target"
                                  min={0}
                                  max={10000}
                                  onChange={(event) => setFactoryTargetDatabank(Math.max(0, Number(event.target.value) || 0))}
                                  type="number"
                                  value={factoryTargetDatabank}
                                />
                              </label>
                              <button
                                type="button"
                                className="secondary"
                                disabled={
                                  loading
                                  || batteryBusy
                                  || batteryActive
                                  || holdingShrinkBusy
                                  || batchSelection.size === 0
                                }
                                onClick={() => void startBatteryOnSelection()}
                              >
                                {batteryActive
                                  ? "Battery running…"
                                  : batteryBusy
                                    ? "Starting…"
                                    : `Run battery (${batchSelection.size})`}
                              </button>
                            </div>
                          )}
                        </>
                      )}
                      <input
                        aria-label="Search elites"
                        onChange={(event) => setQuery(event.target.value)}
                        placeholder="Search ID or fingerprint"
                        value={query}
                      />
                      <select
                        aria-label="Filter entry conditions"
                        onChange={(event) => setEntryFilter(event.target.value)}
                        value={entryFilter}
                      >
                        <option value="all">All entry counts</option>
                        {workspace.conditionGroups.map((item) => (
                          <option key={item.entryConditions} value={String(item.entryConditions)}>
                            {item.label}
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
                        <option value="recoveryFactor">Recovery factor ↓</option>
                        <option value="sharpe">Sharpe ↓</option>
                        <option value="entryConditions">Entry conditions</option>
                        <option value="grade">Grade A–Z</option>
                      </select>
                      {filtered.length !== archiveRows.length && (
                        <button
                          type="button"
                          className="text-action"
                          onClick={() => {
                            setQuery("");
                            setEntryFilter("all");
                          }}
                        >
                          Clear filters
                        </button>
                      )}
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
                        disabled={filtered.length === 0}
                        onClick={selectAllElites}
                      >
                        Select all
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
                        {batchBusy ? "Exporting…" : `Export ${batchSelection.size} IRs`}
                      </button>
                      <label>
                        EA TF
                        <select
                          aria-label="Batch EA timeframe"
                          onChange={(event) => setBatchEaTimeframe(event.target.value)}
                          value={batchEaTimeframe}
                        >
                          <option value="M15">M15</option>
                          <option value="H1">H1</option>
                        </select>
                      </label>
                      <label>
                        Base magic
                        <input
                          aria-label="Batch EA base magic number"
                          min={1}
                          onChange={(event) =>
                            setBatchEaMagic(Math.max(1, Number(event.target.value) || 1))
                          }
                          type="number"
                          value={batchEaMagic}
                        />
                      </label>
                      <button
                        className="primary"
                        disabled={batchBusy || batchSelection.size === 0}
                        onClick={() => void exportSelectedEas()}
                      >
                        {batchBusy ? "Exporting…" : `Export ${batchSelection.size} EAs`}
                      </button>
                      <button
                        className="secondary"
                        disabled={batchBusy || batchSelection.size === 0}
                        onClick={() => void exportSelectedTradeCsvs()}
                        title="Replay each strategy on M1 and export one SQX-compatible trade ledger per strategy"
                      >
                        {batchBusy ? "Exporting…" : `Export ${batchSelection.size} CSVs`}
                      </button>
                    </div>
                  </div>
                  <p className="batch-message">
                    EA batch writes one <code>.mq5</code> per selected strategy (no .set, tester.ini,
                    evidence, or IR). Folder may already have other files; existing matching
                    <code>.mq5</code> names fail instead of overwriting. Experts are named
                    <code>SYMBOL_ENTRYCONDITIONS_UNIQUE_NUMBER</code>; that final number is also the
                    magic. Live trading is off. Choose the strategy’s decision timeframe.
                    Trade CSV export replays each selected strategy through the M1 judge and writes
                    SQX-compatible ledgers plus a summary index with IS, OOS1 and OOS2 labels.
                  </p>
                  {batchMessage && <p className="batch-message">{batchMessage}</p>}
                  <EliteTable
                    rows={filtered}
                    selected={detail?.fingerprint ?? null}
                    batchSelection={batchSelection}
                    onDoubleClick={(fingerprint) => void openResultsDetail(fingerprint, "Databank")}
                    onSelect={selectElite}
                    onToggle={toggleBatchElite}
                    showActions
                    onExportIr={(fingerprint) => void exportIr(fingerprint)}
                    onExportMq5={(fingerprint) => void exportMq5(fingerprint)}
                    onStage={(fingerprint) => void stageForCertification(fingerprint)}
                  />
                </section>
              </div>

              <EliteInspector
                detail={detail}
                loading={detailLoading}
                researchGrade={workspace.researchGrade}
                m1FidelityVerified={workspace.m1FidelityVerified}
                onError={setError}
                tab={detailTab}
                onTab={setDetailTab}
                onParity={continueToParity}
                onOpenResearch={(fingerprint) => void openResultsDetail(fingerprint, "Databank")}
              />
            </div>
          </div>
            )}
          </div>
        ) : null}

        {eliteModalOpen && workspace && (
          <EliteDetailModal
            detail={detail}
            loading={detailLoading}
            m1FidelityVerified={workspace.m1FidelityVerified}
            onClose={() => setEliteModalOpen(false)}
            onError={setError}
            onParity={continueToParity}
            onTab={setDetailTab}
            researchGrade={workspace.researchGrade}
            tab={detailTab}
          />
        )}
          </>
        )}
      </main>
        </div>

      </div>
    </div>
  );
}

function DatabankStrip({
  rows,
  selected,
  selection,
  message,
  busy,
  collapsed = false,
  onToggle,
  onOpen,
  onExportIr,
  onExportMq5,
  onStage,
  onExportSelectedIr,
  onExportSelectedMq5,
  onClearSelection,
  onOpenDatabank,
  hasDatabank,
}: {
  rows: EliteRow[];
  selected: string | null;
  selection: Set<string>;
  message: string | null;
  busy: boolean;
  /** Results detail already shows the full databank table; collapse to the summary bar. */
  collapsed?: boolean;
  onToggle: (fingerprint: string) => void;
  onOpen: (fingerprint: string) => void;
  onExportIr: (fingerprint: string) => void;
  onExportMq5: (fingerprint: string) => void;
  onStage: (fingerprint: string) => void;
  onExportSelectedIr: () => void;
  onExportSelectedMq5: () => void;
  onClearSelection: () => void;
  onOpenDatabank: () => void;
  hasDatabank: boolean;
}) {
  const top = [...rows].sort((a, b) => b.evidence - a.evidence).slice(0, 40);
  return (
    <footer
      className={collapsed ? "databank-strip collapsed" : "databank-strip"}
      aria-label="Databank elites"
    >
      <div className="databank-strip-head">
        <div>
          <p className="eyebrow">Databank</p>
          <strong>
            {rows.length
              ? `${formatNumber(rows.length)} elites${collapsed ? " · full table above" : ""}`
              : "No databank loaded"}
          </strong>
        </div>
        <div className="databank-strip-actions">
          {!hasDatabank ? (
            <button type="button" className="secondary" onClick={onOpenDatabank}>Open databank</button>
          ) : (
            <>
              <button type="button" className="secondary" disabled={busy || selection.size === 0} onClick={onExportSelectedIr}>
                Export {selection.size || ""} IR
              </button>
              <button type="button" className="secondary" disabled={busy || selection.size === 0} onClick={onExportSelectedMq5}>
                Export MQ5
              </button>
              <button type="button" className="text-action" disabled={selection.size === 0} onClick={onClearSelection}>Clear</button>
            </>
          )}
        </div>
      </div>
      {message && <p className="databank-strip-message">{message}</p>}
      {collapsed ? null : top.length === 0 ? (
        <p className="databank-strip-empty">Load or run Discover to populate the always-on elite strip.</p>
      ) : (
        <div className="databank-strip-table">
          <div className="strip-row strip-header">
            <span />
            <span>Strategy</span>
            <span>E/X</span>
            <span>Ev</span>
            <span>Ret</span>
            <span>DD</span>
            <span />
          </div>
          <div className="strip-body">
            {top.map((row) => (
              <div
                className={`strip-row ${selected === row.fingerprint ? "selected" : ""}`}
                key={row.fingerprint}
              >
                <span className="batch-check">
                  <input
                    aria-label={`Select ${row.strategyId}`}
                    checked={selection.has(row.fingerprint)}
                    onChange={() => onToggle(row.fingerprint)}
                    type="checkbox"
                  />
                </span>
                <button type="button" className="strip-open" onClick={() => onOpen(row.fingerprint)} title={row.fingerprint}>
                  <strong>{row.strategyId}</strong>
                </button>
                <span>{conditionLabel(row.entryConditions, row.exitConditions)}</span>
                <span>{row.evidence.toFixed(1)}</span>
                <span className={row.returnPercent >= 0 ? "positive" : "negative"}>{row.returnPercent.toFixed(1)}%</span>
                <span>{row.drawdownPercent.toFixed(1)}%</span>
                <span className="row-actions">
                  <button type="button" className="text-action" onClick={() => onOpen(row.fingerprint)}>Open</button>
                  <button type="button" className="text-action" onClick={() => onExportIr(row.fingerprint)}>IR</button>
                  <button type="button" className="text-action" onClick={() => onExportMq5(row.fingerprint)}>MQ5</button>
                  <button type="button" className="text-action" onClick={() => onStage(row.fingerprint)} title="Stage for certification">Stage</button>
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </footer>
  );
}

function ResultsDetailPage({
  fingerprint,
  workspace,
  backLabel,
  onBack,
  onError,
  onExportIr,
  onExportMq5,
  onStage,
  onOpenResult,
}: {
  fingerprint: string;
  workspace: DatabankWorkspace | null;
  backLabel?: string;
  onBack?: () => void;
  onError: (message: string | null) => void;
  onExportIr: (fingerprint: string) => void;
  onExportMq5: (fingerprint: string) => void;
  onStage: (fingerprint: string) => void;
  onOpenResult: (fingerprint: string) => void;
}) {
  const [detail, setDetail] = useState<EliteDetail | null>(null);
  const [loading, setLoading] = useState(true);
  const [partition, setPartition] = useState<PartitionEquityView | null>(null);
  const [partitionBusy, setPartitionBusy] = useState(true);
  const [robustnessMode, setRobustnessMode] = useState<ResultsRobustnessMode>("standard");
  const [robustnessRun, setRobustnessRun] = useState<ResultsRobustnessView | null>(null);
  const [robustnessBusy, setRobustnessBusy] = useState(false);
  const [tableQuery, setTableQuery] = useState("");
  const [copyMq5Busy, setCopyMq5Busy] = useState(false);
  const [copyMq5Done, setCopyMq5Done] = useState(false);
  const [analysisTab, setAnalysisTab] = useState<"overview" | "equity" | "walkForward" | "monteCarlo" | "parameters" | "trades" | "ir">("overview");
  const row = workspace?.elites.find((elite) => elite.fingerprint === fingerprint) ?? null;

  useEffect(() => {
    let cancelled = false;
    setRobustnessRun(null);
    setRobustnessBusy(false);
    setLoading(true);
    void loadElite(fingerprint)
      .then((loaded) => {
        if (!cancelled) setDetail(loaded);
      })
      .catch((reason) => {
        if (!cancelled) onError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fingerprint, onError]);

  const runRobustness = async () => {
    setRobustnessBusy(true);
    onError(null);
    try {
      const result = await runEliteRobustness({ fingerprint, mode: robustnessMode });
      setRobustnessRun(result);
      if (result.evidence) {
        setDetail((current) => current
          ? {
              ...current,
              robustness: {
                ...(current.robustness ?? {}),
                evidence: result.evidence,
              },
            }
          : current);
      }
    } catch (reason) {
      onError(String(reason));
    } finally {
      setRobustnessBusy(false);
    }
  };

  const copyMq5ToClipboard = async () => {
    setCopyMq5Busy(true);
    setCopyMq5Done(false);
    onError(null);
    try {
      const source = await getEliteMql5Source(fingerprint, timeframeFromDataPath(workspace?.dataPath) ?? "H1");
      await writeClipboardText(source.source);
      setCopyMq5Done(true);
      window.setTimeout(() => setCopyMq5Done(false), 1_800);
    } catch (reason) {
      onError(`Could not copy MQL5 source: ${String(reason)}`);
    } finally {
      setCopyMq5Busy(false);
    }
  };

  useEffect(() => {
    if (!detail || detail.fingerprint !== fingerprint || detail.sealedProtected) {
      setPartition(null);
      setPartitionBusy(false);
      return;
    }
    let cancelled = false;
    setPartition(null);
    setPartitionBusy(true);
    void getElitePartitionEquity(fingerprint)
      .then((next) => {
        if (!cancelled) setPartition(next);
      })
      .catch(() => {
        // A missing replay is not an error here: the stored signature is the fallback.
      })
      .finally(() => {
        if (!cancelled) setPartitionBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [fingerprint, detail?.sealedProtected]);

  const initialBalance = partition?.initialBalance ?? workspace?.initialBalance ?? 100_000;
  const storedMetrics = detail?.metrics ?? {};
  const usesReplay = partition !== null;
  const returnPercent = usesReplay
    ? partition.fullRunReturnPercent
    : row?.returnPercent ?? Number(storedMetrics.return_percent ?? 0);
  const drawdownPercent = usesReplay
    ? partition.fullRunMaxDrawdownPercent
    : row?.drawdownPercent ?? Number(storedMetrics.max_drawdown_percent ?? 0);
  const trades = usesReplay
    ? partition.fullRunTrades
    : row?.trades ?? Number(storedMetrics.trade_count ?? 0);
  const winRate = usesReplay ? partition.fullRunWinRate : Number(storedMetrics.win_rate ?? 0);
  const profitFactor = usesReplay
    ? partition.fullRunProfitFactor
    : storedMetrics.profit_factor ?? null;
  const sharpe = usesReplay
    ? partition.fullRunSharpeRatio
    : row?.sharpeRatio ?? (detail ? effectiveDetailSharpe(detail) : null);
  const netProfit = usesReplay
    ? partition.fullRunNetProfit
    : (initialBalance * returnPercent) / 100;
  const maxDrawdownAbs = usesReplay
    ? partition.fullRunMaxDrawdown
    : Number(storedMetrics.max_drawdown ?? 0);
  const startMs = partition?.points[0]?.timestampMs ?? null;
  const endMs = partition?.points.at(-1)?.timestampMs ?? null;
  const cagr = startMs !== null && endMs !== null ? cagrPercent(returnPercent, startMs, endMs) : null;
  const calmar = cagr !== null && drawdownPercent > 1e-9 ? cagr / drawdownPercent : null;
  const recoveryFactor = usesReplay
    ? partition.fullRunRecoveryFactor
    : maxDrawdownAbs > 1e-9
      ? netProfit / maxDrawdownAbs
      : row?.recoveryFactor ?? null;
  const averageTrade = trades > 0 ? netProfit / trades : null;
  const expectancy = storedMetrics.expectancy ?? null;
  const sortino =
    typeof storedMetrics.sortino_ratio === "number"
      ? storedMetrics.sortino_ratio
      : null;
  const conditions = useMemo(
    () => describeStrategyConditions(detail?.strategyIr ?? null),
    [detail?.strategyIr],
  );
  const evidence = robustnessRun?.evidence ?? detail?.robustness?.evidence ?? null;
  const tableRows = useMemo(() => {
    const rows = workspace?.elites ?? [];
    return filterAndSortElites(rows, tableQuery, "all", "evidence");
  }, [workspace, tableQuery]);

  const universe = symbolFromDataPath(workspace?.dataPath) ?? "—";
  const timeframe = timeframeFromDataPath(workspace?.dataPath) ?? "—";

  return (
    <div className="results-detail-page">
      <section className="results-strategy-card">
        <header className="results-detail-head">
          <div className="results-detail-title">
            {onBack && (
              <button type="button" className="results-context-back" onClick={onBack} title={`Back to ${backLabel ?? "results"}`}>
                <svg aria-hidden="true" viewBox="0 0 24 24"><path d="M14.5 6 9 12l5.5 6" /></svg>
              </button>
            )}
            <span className="result-favorite" title="Featured strategy" aria-label="Featured strategy">
              <svg aria-hidden="true" viewBox="0 0 24 24"><path d="m12 3 2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.8-5.2 2.8 1-5.8-4.3-4.1 5.9-.9L12 3Z" /></svg>
            </span>
            <div>
              <h2>
                {detail?.strategyId ?? row?.strategyId ?? "Strategy"}
                <span className="results-badge">{usesReplay ? "M1 replay" : "Stored backtest"}</span>
                {workspace?.legacyReadOnly && (
                  <span className="results-badge legacy">v5 · retest only</span>
                )}
                {detail && <span className="grade-pill">{detail.grade}</span>}
              </h2>
              <code title={fingerprint}>{fingerprint}</code>
            </div>
          </div>
          <div className="results-detail-actions">
            <span className="result-more" title="Export and certification actions" aria-hidden="true">•••</span>
            <button type="button" className="secondary" onClick={() => onExportIr(fingerprint)}>Export IR</button>
            <button type="button" className="secondary" onClick={() => onExportMq5(fingerprint)}>Export MQ5</button>
            <button type="button" className="secondary" disabled={copyMq5Busy} onClick={() => void copyMq5ToClipboard()}>
              {copyMq5Busy ? "Generating…" : copyMq5Done ? "Copied" : "Copy MQ5"}
            </button>
            <button
              type="button"
              className="primary"
              disabled={workspace?.legacyReadOnly}
              title={workspace?.legacyReadOnly ? "Run a fresh v6 Discover search before staging for certification." : undefined}
              onClick={() => onStage(fingerprint)}
            >
              Stage
            </button>
          </div>
        </header>

        {loading && <div className="results-inline-loading">Loading elite…</div>}

        <section className="results-meta" aria-label="Strategy metadata">
          <div className="results-meta-facts">
            <MetaFact label="Universe" value={universe} />
            <MetaFact label="Timeframe" value={timeframe} />
            <MetaFact label="Date range" value={formatDateRange(startMs, endMs)} />
            <MetaFact label="Capital" value={`$${formatNumber(initialBalance)}`} />
            <MetaFact
              label="Fees"
              value={workspace ? `$${formatNumber(workspace.commissionPerLotRoundTurn, 2)}/lot RT` : "—"}
            />
            <MetaFact
              label="Slippage"
              value={workspace ? `${formatNumber(workspace.slippagePointsPerSide, 2)} pts/side` : "—"}
            />
          </div>
          <div className="results-conditions">
            <ConditionList
              title={`Entry conditions (${detail?.entryConditions ?? row?.entryConditions ?? 0})`}
              items={conditions.entry}
              note={conditions.side ? `${conditions.side} · ${conditions.order ?? "Market"} entry` : null}
            />
            <ConditionList
              title={`Exit conditions (${detail?.exitConditions ?? row?.exitConditions ?? 0})`}
              items={conditions.exit}
              note={
                conditions.stopLoss || conditions.takeProfit
                  ? `Stop ${conditions.stopLoss ?? "—"} · Target ${conditions.takeProfit ?? "—"}`
                  : null
              }
            />
          </div>
          <div className="results-trades-card">
            <span>Total trades</span>
            <strong>{formatNumber(trades)}</strong>
            <small>{usesReplay ? "100% replayed" : "stored IS gate"}</small>
          </div>
        </section>
      </section>

      <ResultsAnalysisTabs active={analysisTab} onChange={setAnalysisTab} />

      {analysisTab === "overview" && <>
      <section className="results-kpi-strip" aria-label="Performance">
        <KpiCell label="Net profit" value={`$${formatNumber(netProfit)}`} note={`${returnPercent.toFixed(2)}% return`} tone={netProfit >= 0 ? "positive" : "negative"} />
        <KpiCell label="CAGR" value={cagr === null ? "—" : `${cagr.toFixed(2)}%`} note={cagr === null ? "window too short" : "annualized"} tone={cagr !== null && cagr >= 0 ? "positive" : undefined} />
        <KpiCell label="Win rate" value={`${winRate.toFixed(1)}%`} note={`${formatNumber(trades)} trades`} />
        <KpiCell label="Profit factor" value={profitFactor === null || profitFactor === undefined ? "—" : Number(profitFactor).toFixed(2)} note="gross win / gross loss" />
        <KpiCell label="Max drawdown" value={`-${drawdownPercent.toFixed(2)}%`} note={maxDrawdownAbs > 0 ? `$${formatNumber(maxDrawdownAbs)} equity peak to valley` : "equity peak to valley"} tone="negative" />
        <KpiCell label="Recovery factor" value={recoveryFactor === null || recoveryFactor === undefined ? "—" : Number(recoveryFactor).toFixed(2)} note="net profit / equity DD (MT5)" />
        <KpiCell label="Sharpe" value={sharpe === null || sharpe === undefined ? "—" : Number(sharpe).toFixed(2)} note="per-bar equity, not MT5 scale" />
        <KpiCell label="Sortino" value={sortino === null ? "—" : sortino.toFixed(2)} note={sortino === null ? "not recorded" : "downside deviation"} />
        <KpiCell label="Calmar" value={calmar === null ? "—" : calmar.toFixed(2)} note="CAGR / max DD" />
        <KpiCell label="Avg trade" value={averageTrade === null ? "—" : `$${formatNumber(averageTrade, 2)}`} note="net per closed trade" />
        <KpiCell label="Expectancy" value={expectancy === null ? "—" : `$${formatNumber(Number(expectancy), 2)}`} note={expectancy === null ? "not recorded" : `${(Number(expectancy) / 1000).toFixed(2)}R at $1,000 risk`} />
      </section>

      <section className={`panel results-robustness-runner ${robustnessRun ? (robustnessRun.passed ? "pass" : "fail") : ""}`}>
        <div className="results-robustness-copy">
          <p className="eyebrow">Robustness battery</p>
          <h2>Re-run the exact M1 promotion checks</h2>
          <p>
            Uses this databank’s bound IS, M1 chronology, broker costs, thresholds and frozen seed.
            OOS1 validation runs after the Development battery; sealed OOS2 remains untouched.
          </p>
        </div>
        <div className="results-robustness-controls">
          <div className="segmented robustness-depth" aria-label="Robustness depth">
            <button
              type="button"
              className={robustnessMode === "standard" ? "active" : ""}
              disabled={robustnessBusy}
              onClick={() => setRobustnessMode("standard")}
              title="Repeat the battery strength sealed into this databank"
            >
              Standard
            </button>
            <button
              type="button"
              className={robustnessMode === "deep" ? "active" : ""}
              disabled={robustnessBusy}
              onClick={() => setRobustnessMode("deep")}
              title="More folds, neighbours and Monte Carlo resampling; gates remain fixed"
            >
              Deep
            </button>
          </div>
          <button
            type="button"
            className="primary robustness-run-button"
            disabled={robustnessBusy || loading || !detail}
            onClick={() => void runRobustness()}
          >
            {robustnessBusy ? "Running battery…" : robustnessRun ? "Run again" : "Run robustness battery"}
          </button>
        </div>
        <div className="results-robustness-note">
          {robustnessRun ? (
            <>
              <span className={`robustness-run-status ${robustnessRun.passed ? "pass" : "fail"}`}>
                {robustnessRun.passed ? "Passed" : `Blocked · ${robustnessRun.blocker ?? "robustness gate"}`}
              </span>
              <span>
                {robustnessRun.folds} folds · {formatNumber(robustnessRun.monteCarloTrials)} MC ·{" "}
                {formatNumber(robustnessRun.neighborhoodSamples)} neighbours
              </span>
              <code title={robustnessRun.artifactPath}>{robustnessRun.artifactPath}</code>
            </>
          ) : (
            <span>
              {robustnessMode === "standard"
                ? "Standard repeats the promotion battery already sealed into the databank."
                : "Deep raises compute depth without changing thresholds or searching for a better seed."}
            </span>
          )}
        </div>
      </section>

      <div className="results-robustness-grid">
        <MonteCarloPanel evidence={evidence?.monte_carlo ?? null} fallback={detail?.robustness?.monteCarlo ?? detail?.robustness?.summary ?? null} />
        <ParameterNeighborhoodPanel
          evidence={evidence?.parameter_neighborhood ?? null}
          retention={evidence?.m1_retention ?? null}
          fallback={detail?.robustness?.paramPermutation ?? null}
        />
        <WalkForwardPanel evidence={evidence?.walk_forward ?? null} fallback={detail?.robustness?.walkForward ?? null} title="Development CPCV analysis" />
        <WalkForwardPanel evidence={evidence?.sequential_walk_forward ?? null} fallback={null} title="Sequential walk-forward analysis" />
      </div>

      {detail?.thesis && (
        <section className="panel">
          <div className="panel-heading"><div><p className="eyebrow">Thesis</p><h2>Strategy brief</h2></div></div>
          <p className="results-thesis">{detail.thesis}</p>
        </section>
      )}
      </>}

      {analysisTab === "equity" && (
        <section className="panel results-equity-panel results-analysis-panel">
          <div className="panel-heading">
            <div><p className="eyebrow">Equity chart</p><h2>IS with distinct OOS evidence overlays</h2></div>
            <span className="read-only-badge">M1 chronology · full history</span>
          </div>
          {usesReplay || partitionBusy ? (
            <PartitionEquityChart
              view={partition}
              busy={partitionBusy}
              researchGrade={workspace?.researchGrade ?? false}
              m1FidelityVerified={workspace?.m1FidelityVerified ?? false}
              large
            />
          ) : (
            <StoredSignatureChart
              initialBalance={initialBalance}
              returnPercent={returnPercent}
              values={detail?.equitySignature ?? row?.equitySignature ?? []}
            />
          )}
        </section>
      )}

      {analysisTab === "walkForward" && (
        <section className="results-analysis-single">
          <WalkForwardPanel evidence={evidence?.walk_forward ?? null} fallback={detail?.robustness?.walkForward ?? null} title="Development CPCV analysis" expanded />
          <WalkForwardPanel evidence={evidence?.sequential_walk_forward ?? null} fallback={null} title="Sequential walk-forward analysis" expanded />
        </section>
      )}

      {analysisTab === "monteCarlo" && (
        <section className="results-analysis-single">
          <MonteCarloPanel evidence={evidence?.monte_carlo ?? null} fallback={detail?.robustness?.monteCarlo ?? detail?.robustness?.summary ?? null} expanded />
        </section>
      )}

      {analysisTab === "parameters" && (
        <section className="results-analysis-single">
          <ParameterNeighborhoodPanel
            evidence={evidence?.parameter_neighborhood ?? null}
            retention={evidence?.m1_retention ?? null}
            fallback={detail?.robustness?.paramPermutation ?? null}
            expanded
          />
        </section>
      )}

      {analysisTab === "trades" && (
        <section className="panel results-trades-panel">
          <div className="panel-heading"><div><p className="eyebrow">List of trades</p><h2>M1 replay ledger</h2></div><span className="read-only-badge">{partition ? `${formatNumber(partition.trades.length)} rows` : "loading"}</span></div>
          <TradeListPanel busy={partitionBusy} trades={partition?.trades ?? []} />
        </section>
      )}

      {analysisTab === "ir" && (
        <section className="panel results-ir-panel">
          <div className="panel-heading"><div><p className="eyebrow">Strategy configuration</p><h2>Canonical QuantForge IR</h2></div></div>
          <pre className="ir-tree results-ir-tree">{detail ? JSON.stringify(detail.strategyIr, null, 2) : "Loading strategy…"}</pre>
        </section>
      )}

      <section className="panel results-databank-panel">
        <div className="panel-heading results-databank-head">
          <div className="results-databank-title">
            <p className="eyebrow">Databank</p>
            <span className="results-databank-tab">All results</span>
          </div>
          <input
            aria-label="Search databank"
            className="results-databank-search"
            onChange={(event) => setTableQuery(event.target.value)}
            placeholder="Search databank…"
            value={tableQuery}
          />
        </div>
        {tableRows.length === 0 ? (
          <p className="cluster-empty">
            {workspace
              ? "No databank rows match this search."
              : "Open a databank to list every result alongside this strategy."}
          </p>
        ) : (
          <>
            <EliteTable
              batchSelection={new Set()}
              onDoubleClick={onOpenResult}
              onSelect={onOpenResult}
              onToggle={() => {}}
              rows={tableRows}
              selected={fingerprint}
              showBatch={false}
              showActions
              onExportIr={onExportIr}
              onExportMq5={onExportMq5}
              onStage={onStage}
              rowHeightOverride={44}
              viewportHeightOverride={70}
            />
            <p className="results-databank-foot">
              Showing {formatNumber(tableRows.length)} of {formatNumber(workspace?.elites.length ?? 0)} results
            </p>
          </>
        )}
      </section>
    </div>
  );
}

function MetaFact({ label, value, wide = false }: { label: string; value: string; wide?: boolean }) {
  return (
    <div className={wide ? "meta-fact wide" : "meta-fact"}>
      <span>{label}</span>
      <strong title={value}>{value}</strong>
    </div>
  );
}

function ResultsAnalysisTabs({
  active,
  onChange,
}: {
  active: "overview" | "equity" | "walkForward" | "monteCarlo" | "parameters" | "trades" | "ir";
  onChange: (tab: "overview" | "equity" | "walkForward" | "monteCarlo" | "parameters" | "trades" | "ir") => void;
}) {
  const tabs: Array<[typeof active, string]> = [
    ["overview", "Overview"],
    ["equity", "Equity"],
    ["walkForward", "Development CPCV"],
    ["monteCarlo", "Monte Carlo"],
    ["parameters", "Parameters"],
    ["trades", "Trades"],
    ["ir", "Strategy IR"],
  ];
  return (
    <nav className="results-analysis-tabs" aria-label="Strategy research views" role="tablist">
      {tabs.map(([value, label]) => (
        <button key={value} type="button" role="tab" aria-selected={active === value} className={active === value ? "active" : ""} onClick={() => onChange(value)}>{label}</button>
      ))}
    </nav>
  );
}

function ConditionList({
  title,
  items,
  note,
}: {
  title: string;
  items: string[];
  note: string | null;
}) {
  return (
    <article className="condition-list">
      <p className="eyebrow">{title}</p>
      {items.length === 0 ? (
        <p className="condition-empty">Condition tree unavailable for this stored IR.</p>
      ) : (
        <ol>
          {items.map((item, index) => (
            <li key={`${index}-${item}`}><span>{index + 1}.</span><code>{item}</code></li>
          ))}
        </ol>
      )}
      {note && <small>{note}</small>}
    </article>
  );
}

function KpiCell({
  label,
  value,
  note,
  tone,
}: {
  label: string;
  value: string;
  note: string;
  tone?: "positive" | "negative";
}) {
  return (
    <article className="kpi-cell">
      <span>{label}</span>
      <strong className={tone ?? undefined}>{value}</strong>
      <small>{note}</small>
    </article>
  );
}

function StoredSignatureChart({
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
        <polyline fill="none" points={points} />
      </svg>
    </div>
  );
}

function RobustnessShell({
  title,
  status,
  children,
}: {
  title: string;
  status: { label: string; tone: "pass" | "fail" | "unknown" };
  children: React.ReactNode;
}) {
  return (
    <article className="panel robustness-panel">
      <div className="robustness-head">
        <p className="eyebrow">{title}</p>
        <span className={`robustness-status ${status.tone}`}>{status.label}</span>
      </div>
      {children}
    </article>
  );
}

function RobustnessMissing({ fallback }: { fallback: string | null }) {
  return (
    <div className="robustness-missing">
      {fallback && fallback.trim().length > 0 ? (
        <p>{fallback}</p>
      ) : (
        <p>
          No structured evidence was persisted for this elite. Use the Results robustness battery
          above to create a new immutable report.
        </p>
      )}
    </div>
  );
}

function StatRows({ rows }: { rows: Array<[string, string]> }) {
  return (
    <dl className="robustness-stats">
      {rows.map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function GaugeBar({
  fraction,
  required,
  passed,
}: {
  fraction: number;
  required?: number | null;
  passed: boolean;
}) {
  const clamped = Math.max(0, Math.min(1, fraction));
  return (
    <div className="gauge-bar" role="img" aria-label={`${(clamped * 100).toFixed(0)} percent`}>
      <i className={passed ? "pass" : "fail"} style={{ width: `${clamped * 100}%` }} />
      {required !== null && required !== undefined && (
        <span className="gauge-threshold" style={{ left: `${Math.max(0, Math.min(1, required)) * 100}%` }} />
      )}
    </div>
  );
}

function medianOf(values: number[]): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
}

function metricNumber(metrics: Record<string, number | null> | null | undefined, key: string): number | null {
  if (!metrics) return null;
  const value = metrics[key];
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function MonteCarloPanel({
  evidence,
  fallback,
  expanded = false,
}: {
  evidence: RobustnessEvidence["monte_carlo"] | null;
  fallback: string | null;
  expanded?: boolean;
}) {
  if (!evidence) {
    return (
      <RobustnessShell title="Monte Carlo analysis" status={{ label: "not recorded", tone: "unknown" }}>
        <RobustnessMissing fallback={fallback} />
      </RobustnessShell>
    );
  }
  const p80 = evidence.p80_net_profit ?? null;
  const baseline = evidence.baseline_net_profit ?? null;
  const retentionFloor = evidence.minimum_p80_profit_retention ?? 0.6;
  const scale = Math.max(
    Math.abs(evidence.median_net_profit),
    Math.abs(evidence.p05_net_profit),
    p80 != null ? Math.abs(p80) : 0,
    baseline != null ? Math.abs(baseline) : 0,
    1e-9,
  );
  const p80Retained =
    p80 != null &&
    baseline != null &&
    baseline > 0 &&
    p80 >= retentionFloor * baseline;
  const bars: Array<{ label: string; value: number; width: number; tone: string }> = [
    {
      label: "Median net profit",
      value: evidence.median_net_profit,
      width: Math.abs(evidence.median_net_profit) / scale,
      tone: evidence.median_net_profit >= 0 ? "pass" : "fail",
    },
    {
      label: "80th percentile",
      value: p80 ?? 0,
      width: Math.abs(p80 ?? 0) / scale,
      tone: p80Retained ? "pass" : "fail",
    },
    {
      label: "5th percentile",
      value: evidence.p05_net_profit,
      width: Math.abs(evidence.p05_net_profit) / scale,
      tone: evidence.p05_net_profit >= 0 ? "pass" : "fail",
    },
  ];
  return (
    <RobustnessShell
      title="Monte Carlo analysis"
      status={{ label: evidence.passed ? "passed" : "failed", tone: evidence.passed ? "pass" : "fail" }}
    >
      <div className="robustness-bars">
        {bars.map((bar) => (
          <div className="robustness-bar" key={bar.label}>
            <span>{bar.label}</span>
            <i><b className={bar.tone} style={{ width: `${Math.max(2, bar.width * 100)}%` }} /></i>
            <strong className={bar.value >= 0 ? "positive" : "negative"}>
              ${formatNumber(bar.value)}
            </strong>
          </div>
        ))}
      </div>
      <StatRows
        rows={[
          ["Trials", formatNumber(evidence.trials)],
          ["Method", `${evidence.method} · block ${formatNumber(evidence.block_length)}`],
          ["Trade removal", `${((evidence.skip_trade_probability ?? 0) * 100).toFixed(0)}% per simulation`],
          ["P05 floor", `P05 ≥ $${formatNumber(evidence.minimum_p05_net_profit ?? 0)}`],
          [
            "P80 retention",
            `P80 ≥ ${(retentionFloor * 100).toFixed(0)}% of baseline${
              baseline != null ? ` ($${formatNumber(retentionFloor * baseline)})` : ""
            }`,
          ],
          ["Baseline net profit", baseline != null ? `$${formatNumber(baseline)}` : "—"],
          [
            "Drawdown criterion",
            `P95 ≤ ${(evidence.maximum_p95_drawdown_percent ?? 0).toFixed(2)}% (${((evidence.maximum_drawdown_ratio ?? 1.75) * 100).toFixed(0)}% of baseline)`,
          ],
          ["P95 drawdown", `${evidence.p95_drawdown_percent.toFixed(2)}%`],
          ["Worst drawdown", `${evidence.worst_drawdown_percent.toFixed(2)}%`],
          ["Seed", String(evidence.seed)],
        ]}
      />
      <MonteCarloPathChart paths={evidence.sample_paths ?? []} tall={expanded} />
    </RobustnessShell>
  );
}

function MonteCarloPathChart({ paths, tall = false }: { paths: number[][]; tall?: boolean }) {
  if (paths.length === 0) {
    return (
      <p className="robustness-missing">
        Path samples were not recorded in this older evidence artifact. Re-run the battery to generate them.
      </p>
    );
  }
  const values = paths.flat();
  const rawMin = Math.min(...values);
  const rawMax = Math.max(...values);
  // Paths are balance levels that start at the opening deposit, so anchoring the
  // axis at zero would squash every path into a thin band at the top.
  const pad = Math.max((rawMax - rawMin) * 0.06, 1);
  const minimum = rawMin - pad;
  const maximum = rawMax + pad;
  const span = Math.max(maximum - minimum, 1e-9);
  const left = 68;
  const right = 984;
  const top = 20;
  const bottom = 402;
  const plotHeight = bottom - top;
  const plotWidth = right - left;
  const yAt = (value: number) => bottom - ((value - minimum) / span) * plotHeight;
  const tickCount = 7;
  const ticks = Array.from(
    { length: tickCount },
    (_, index) => maximum - ((maximum - minimum) * index) / (tickCount - 1),
  );
  const openingBalance = paths[0]?.[0];
  const breakEvenY =
    openingBalance !== undefined && openingBalance > minimum && openingBalance < maximum
      ? yAt(openingBalance)
      : null;
  return (
    <section className={`monte-carlo-paths${tall ? " tall" : ""}`}>
      <div>
        <p className="eyebrow">Resampled paths</p>
        <small>{paths.length} deterministic samples · resampling + trade removal</small>
      </div>
      <svg viewBox="0 0 1000 420" role="img" aria-label="Monte Carlo resampled equity paths" preserveAspectRatio="none">
        {ticks.map((tick) => (
          <g key={tick}>
            <line className="mc-grid" x1={left} x2={right} y1={yAt(tick)} y2={yAt(tick)} />
            <text className="mc-axis-label" x={left - 8} y={yAt(tick) + 3} textAnchor="end">
              {formatNumber(tick, 0)}
            </text>
          </g>
        ))}
        {breakEvenY !== null && (
          <>
            <line className="mc-zero" x1={left} x2={right} y1={breakEvenY} y2={breakEvenY} />
            <text className="mc-axis-label" x={right} y={breakEvenY - 6} textAnchor="end">
              break-even
            </text>
          </>
        )}
        {paths.map((path, pathIndex) => {
          const points = path
            .map((value, index) => {
              const x = left + (index / Math.max(path.length - 1, 1)) * plotWidth;
              const y = yAt(value);
              return `${x},${y}`;
            })
            .join(" ");
          return <polyline key={pathIndex} points={points} className={pathIndex === 0 ? "mc-path first" : "mc-path"} />;
        })}
      </svg>
    </section>
  );
}

function ParameterNeighborhoodPanel({
  evidence,
  retention,
  fallback,
  expanded = false,
}: {
  evidence: RobustnessEvidence["parameter_neighborhood"] | null;
  retention: RobustnessEvidence["m1_retention"] | null;
  fallback: string | null;
  expanded?: boolean;
}) {
  if (!evidence) {
    return (
      <RobustnessShell title="Parameter neighborhood" status={{ label: "not recorded", tone: "unknown" }}>
        <RobustnessMissing fallback={fallback} />
      </RobustnessShell>
    );
  }
  const ratio = (value: number | null | undefined) =>
    value === null || value === undefined ? "—" : `${(value * 100).toFixed(1)}%`;
  const samples = evidence.samples ?? [];
  const original = evidence.original_metrics ?? null;
  const recoveryValues = samples
    .map((sample) => {
      if (typeof sample.recovery_factor === "number" && Number.isFinite(sample.recovery_factor)) {
        return sample.recovery_factor;
      }
      if (typeof sample.max_drawdown === "number" && sample.max_drawdown > 1e-12) {
        return sample.net_profit / sample.max_drawdown;
      }
      return null;
    })
    .filter((value): value is number => value !== null);
  const originalRecovery = (() => {
    const direct = metricNumber(original, "recovery_factor");
    if (direct !== null) return direct;
    const profit = metricNumber(original, "net_profit");
    const dd = metricNumber(original, "max_drawdown");
    if (profit === null || dd === null || dd <= 1e-12) return null;
    return profit / dd;
  })();
  const recoveryBand =
    evidence.original_recovery_to_median === null || evidence.original_recovery_to_median === undefined
      ? null
      : evidence.original_recovery_to_median;
  const recoveryBandPassed = evidence.passed_recovery_median_band;
  const passed =
    evidence.survival_fraction >= evidence.required_survival_fraction &&
    recoveryBandPassed !== false;
  const distributionMetrics = ([
    {
      key: "recovery_factor",
      label: "Ret / DD",
      values: recoveryValues,
      original: originalRecovery,
      format: (value: number) => value.toFixed(2),
    },
    {
      key: "net_profit",
      label: "Net profit",
      values: samples.map((sample) => sample.net_profit),
      original: metricNumber(original, "net_profit"),
      format: (value: number) => `$${formatNumber(value)}`,
    },
    {
      key: "max_drawdown_percent",
      label: "Max DD %",
      values: samples.map((sample) => sample.max_drawdown_percent),
      original: metricNumber(original, "max_drawdown_percent"),
      format: (value: number) => `${value.toFixed(2)}%`,
    },
    {
      key: "return_percent",
      label: "Return %",
      values: samples.map((sample) => sample.return_percent),
      original: metricNumber(original, "return_percent"),
      format: (value: number) => `${value.toFixed(2)}%`,
    },
    {
      key: "sharpe_ratio",
      label: "Sharpe",
      values: samples
        .map((sample) => sample.sharpe_ratio)
        .filter((value): value is number => typeof value === "number" && Number.isFinite(value)),
      original: metricNumber(original, "sharpe_ratio"),
      format: (value: number) => value.toFixed(2),
    },
    {
      key: "profit_factor",
      label: "Profit factor",
      values: samples
        .map((sample) => sample.profit_factor)
        .filter((value): value is number => typeof value === "number" && Number.isFinite(value)),
      original: metricNumber(original, "profit_factor"),
      format: (value: number) => value.toFixed(2),
    },
    {
      key: "trade_count",
      label: "Trades",
      values: samples.map((sample) => sample.trade_count),
      original: metricNumber(original, "trade_count"),
      format: (value: number) => formatNumber(value),
    },
  ] as const).filter((metric) => metric.values.length > 0);

  return (
    <RobustnessShell
      title="System parameter permutation"
      status={{ label: passed ? "passed" : "failed", tone: passed ? "pass" : "fail" }}
    >
      <div className="robustness-gauge">
        <div className="robustness-gauge-head">
          <span>±{(evidence.perturbation_fraction * 100).toFixed(0)}% survival</span>
          <strong className={passed ? "positive" : "negative"}>
            {(evidence.survival_fraction * 100).toFixed(0)}%
          </strong>
        </div>
        <GaugeBar
          fraction={evidence.survival_fraction}
          required={evidence.required_survival_fraction}
          passed={passed}
        />
        <small>Marker is the required {(evidence.required_survival_fraction * 100).toFixed(0)}% floor.</small>
      </div>
      <StatRows
        rows={[
          ["Neighbors", `${formatNumber(evidence.surviving_samples)} / ${formatNumber(evidence.samples_evaluated)} survived`],
          ["Requested", formatNumber(evidence.samples_requested)],
          [
            "Ret/DD orig ÷ median",
            recoveryBand === null
              ? "—"
              : `${(recoveryBand * 100).toFixed(0)}% (keep 85–125%)`,
          ],
          [
            "ADX plateau",
            evidence.plateau_neighbors > 0
              ? `${formatNumber(evidence.plateau_surviving)} / ${formatNumber(evidence.plateau_neighbors)} (${ratio(evidence.plateau_survival_fraction)})`
              : "not applicable",
          ],
          ["M1 return retention", ratio(retention?.return_retention)],
          ["M1 trade retention", ratio(retention?.trade_retention)],
        ]}
      />
      {distributionMetrics.length > 0 && (
        <section className={`param-distribution${expanded ? " expanded" : ""}`}>
          <div className="param-summary-wrap">
            <p className="eyebrow">Median strategy properties</p>
            <table className="param-summary-table">
              <thead>
                <tr>
                  <th>Stat</th>
                  <th>Median</th>
                  <th>Orig.</th>
                  <th>% Orig / Median</th>
                </tr>
              </thead>
              <tbody>
                {distributionMetrics.map((metric) => {
                  const median = medianOf(metric.values);
                  const orig = metric.original;
                  const pct =
                    median !== null && orig !== null && Math.abs(median) > 1e-12
                      ? `${((orig / median) * 100).toFixed(2)}%`
                      : "—";
                  return (
                    <tr key={metric.key}>
                      <td>{metric.label}</td>
                      <td>{median === null ? "—" : metric.format(median)}</td>
                      <td>{orig === null ? "—" : metric.format(orig)}</td>
                      <td>{pct}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          <p className="param-hist-caption">
            {evidence.method === "h1_cached_axis_plus_seeded_joint"
              ? "Neighbours replay on the selected TF with a shared indicator cache, not M1 fills. "
              : evidence.method === "systematic_axis_plus_seeded_joint"
              ? "Each execution axis is tested at both sides of the band before joint perturbations. "
              : "One bar per group of neighbours, so a tall single bar means the plateau is flat and a wide spread means small parameter changes move the result. "}
            The dashed line is the original strategy. Ret/DD must sit between 85% and 125% of the neighbourhood median or the name is treated as a tail fit.
          </p>
          <div className="param-hist-grid">
            {distributionMetrics.map((metric) => (
              <MetricHistogram
                key={metric.key}
                label={`${metric.label} across ${metric.values.length} neighbours`}
                values={metric.values}
                original={metric.original}
                format={metric.format}
                zeroIsMeaningful={metric.key === "net_profit" || metric.key === "return_percent"}
              />
            ))}
          </div>
        </section>
      )}
      {expanded && samples.length === 0 && (
        <section className="parameter-explainer">
          <p className="eyebrow">Plateau test</p>
          <p>
            Each neighbour changes the strategy’s numeric genes by up to ±{(evidence.perturbation_fraction * 100).toFixed(0)}%.
            Re-run the robustness battery to persist per-neighbour metrics for distribution charts.
          </p>
        </section>
      )}
    </RobustnessShell>
  );
}

function MetricHistogram({
  label,
  values,
  original,
  format,
  zeroIsMeaningful = false,
}: {
  label: string;
  values: number[];
  original: number | null;
  format: (value: number) => string;
  /// Profit and return histograms read as "how much of the plateau is losing",
  /// so those charts get a break-even reference line and a shaded loss region.
  zeroIsMeaningful?: boolean;
}) {
  const median = medianOf(values);
  if (values.length === 0 || median === null) return null;
  // Keep the original and the break-even line inside the axis, otherwise the
  // most important reference on the chart is the one you cannot see.
  const anchors = [...values, ...(original === null ? [] : [original]), ...(zeroIsMeaningful ? [0] : [])];
  const min = Math.min(...anchors);
  const max = Math.max(...anchors);
  const span = Math.max(max - min, Math.max(Math.abs(max), 1e-9) * 1e-6);
  const bins = Math.min(24, Math.max(8, Math.round(Math.sqrt(values.length))));
  const counts = Array.from({ length: bins }, () => 0);
  for (const value of values) {
    const index = Math.min(bins - 1, Math.floor(((value - min) / span) * bins));
    counts[index] += 1;
  }
  const peak = Math.max(...counts, 1);
  const percent = (value: number) => ((value - min) / span) * 100;
  const binEdge = (index: number) => min + (span * index) / bins;
  const losing = zeroIsMeaningful ? values.filter((value) => value < 0).length : 0;
  return (
    <article className="param-hist">
      <p className="eyebrow">{label}</p>
      <div className="param-hist-body">
        <div className="param-hist-yaxis" aria-hidden="true">
          <span>{peak}</span>
          <span>0</span>
        </div>
        <div
          className="param-hist-plot"
          role="img"
          aria-label={`${label}: ${values.length} neighbours between ${format(min)} and ${format(max)}, median ${format(median)}`}
        >
          {zeroIsMeaningful && min < 0 && (
            <span className="param-hist-loss" style={{ width: `${percent(Math.min(0, max))}%` }} />
          )}
          {counts.map((count, index) => (
            <i
              key={index}
              style={{ height: `${(count / peak) * 100}%` }}
              title={`${count} of ${values.length} between ${format(binEdge(index))} and ${format(binEdge(index + 1))}`}
            />
          ))}
          {zeroIsMeaningful && min < 0 && max > 0 && (
            <span className="param-hist-zero" style={{ left: `${percent(0)}%` }} title="Break-even" />
          )}
          <span className="param-hist-median" style={{ left: `${percent(median)}%` }} title={`Median ${format(median)}`} />
          {original !== null && (
            <span className="param-hist-original" style={{ left: `${percent(original)}%` }} title={`Original ${format(original)}`} />
          )}
        </div>
      </div>
      <div className="param-hist-xaxis" aria-hidden="true">
        <span>{format(min)}</span>
        <span>{format(max)}</span>
      </div>
      <small className="param-hist-legend">
        <em className="median">Median {format(median)}</em>
        {original !== null && <em className="original">Original {format(original)}</em>}
        {zeroIsMeaningful && <em className="loss">{losing} of {values.length} losing</em>}
      </small>
    </article>
  );
}

function WalkForwardPanel({
  evidence,
  fallback,
  title = "Development CPCV analysis",
  expanded = false,
}: {
  evidence: RobustnessEvidence["walk_forward"] | null;
  fallback: string | null;
  title?: string;
  expanded?: boolean;
}) {
  if (!evidence) {
    return (
      <RobustnessShell title={title} status={{ label: "not recorded", tone: "unknown" }}>
        <RobustnessMissing fallback={fallback} />
      </RobustnessShell>
    );
  }
  const passed = evidence.passing_fraction >= evidence.required_passing_fraction;
  const folds = evidence.folds ?? [];
  const returns = folds.map((fold) => Number(fold.metrics.return_percent ?? 0));
  const scale = Math.max(...returns.map((value) => Math.abs(value)), 1e-9);
  const scorePercent = (evidence.passing_fraction * 100).toFixed(0);
  return (
    <RobustnessShell
      title={title}
      status={{ label: passed ? "passed" : "failed", tone: passed ? "pass" : "fail" }}
    >
      <div className="wf-score-card">
        <div>
          <span>Result</span>
          <strong className={passed ? "positive" : "negative"}>{passed ? "PASSED" : "FAILED"}</strong>
        </div>
        <div>
          <span>Robustness score</span>
          <strong>
            {scorePercent}% ({formatNumber(evidence.passing_folds)} from {formatNumber(evidence.total_folds)} folds)
          </strong>
        </div>
        <div>
          <span>Required</span>
          <strong>≥{(evidence.required_passing_fraction * 100).toFixed(0)}%</strong>
        </div>
        <div>
          <span>Scheme</span>
          <strong>{evidence.fold_scheme}</strong>
        </div>
      </div>
      {!expanded && folds.length > 0 && (
        <div className="fold-chart" role="img" aria-label="Return per walk-forward fold">
          {folds.map((fold, index) => {
            const value = returns[index];
            const height = Math.max(3, (Math.abs(value) / scale) * 46);
            return (
              <div className="fold-column" key={fold.fold} title={`Fold ${fold.fold}: ${value.toFixed(2)}% · ${formatNumber(fold.trades_in_fold)} trades`}>
                <div className="fold-plot">
                  <i
                    className={fold.passed ? "pass" : "fail"}
                    style={value >= 0
                      ? { height: `${height}%`, bottom: "50%" }
                      : { height: `${height}%`, top: "50%" }}
                  />
                </div>
                <span>{fold.fold}</span>
              </div>
            );
          })}
        </div>
      )}
      {expanded && folds.length > 0 && (
        <div className="wf-table-wrap">
          <table className="wf-results-table">
            <thead>
              <tr>
                <th>Fold</th>
                <th>Period</th>
                <th>Bars</th>
                <th>Trades</th>
                <th>Net profit</th>
                <th>Max DD %</th>
                <th>Return %</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {folds.map((fold) => {
                const net = metricNumber(fold.metrics, "net_profit");
                const dd = metricNumber(fold.metrics, "max_drawdown_percent");
                const ret = metricNumber(fold.metrics, "return_percent");
                return (
                  <tr key={fold.fold} className={fold.passed ? "pass" : "fail"}>
                    <td>{fold.fold}</td>
                    <td>{formatDateRange(fold.start_timestamp_ms, fold.end_timestamp_ms)}</td>
                    <td>{formatNumber(fold.decision_bars)}</td>
                    <td>{formatNumber(fold.trades_in_fold)}</td>
                    <td className={net !== null && net >= 0 ? "positive" : "negative"}>
                      {net === null ? "—" : `$${formatNumber(net)}`}
                    </td>
                    <td className="negative">{dd === null ? "—" : `${dd.toFixed(2)}%`}</td>
                    <td className={ret !== null && ret >= 0 ? "positive" : "negative"}>
                      {ret === null ? "—" : `${ret.toFixed(2)}%`}
                    </td>
                    <td>{fold.passed ? "PASSED" : "FAILED"}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {folds.length === 0 && (
        <div className="robustness-gauge">
          <div className="robustness-gauge-head">
            <span>Passing folds</span>
            <strong className={passed ? "positive" : "negative"}>
              {(evidence.passing_fraction * 100).toFixed(0)}%
            </strong>
          </div>
          <GaugeBar
            fraction={evidence.passing_fraction}
            required={evidence.required_passing_fraction}
            passed={passed}
          />
          <small>Per-fold detail was not persisted for this elite.</small>
        </div>
      )}
      {!expanded && (
        <StatRows
          rows={[
            ["Segments", `${formatNumber(evidence.passing_folds)} / ${formatNumber(evidence.total_folds)} passed`],
            ["Scheme", evidence.fold_scheme],
            ["Passing fraction", `${(evidence.passing_fraction * 100).toFixed(1)}%`],
            ["Required", `${(evidence.required_passing_fraction * 100).toFixed(0)}%`],
          ]}
        />
      )}
    </RobustnessShell>
  );
}

function HomeWorkspace({
  workspace,
  onNavigate,
  onOpen,
  onOpenPath,
  onOpenResult,
  onError,
}: {
  workspace: DatabankWorkspace | null;
  onNavigate: (workspace: WorkspaceName) => void;
  onOpen: () => void;
  onOpenPath: (path: string) => Promise<void>;
  onOpenResult: (fingerprint: string) => void;
  onError: (message: string | null) => void;
}) {
  const [job, setJob] = useState<DiscoverJobView | null>(null);
  const [rangeProfiles, setRangeProfiles] = useState<number | null>(null);
  const hasDatabank = workspace !== null;
  const active = job?.status === "running" || job?.status === "paused";

  useEffect(() => {
    void getDiscoverJob()
      .then(setJob)
      .catch((reason) => onError(String(reason)));
    void listSearchRangeProfiles()
      .then((profiles) => setRangeProfiles(profiles.length))
      .catch(() => setRangeProfiles(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => {
      void getDiscoverJob().then(setJob).catch(() => {});
    }, 1500);
    return () => window.clearTimeout(timer);
  }, [active]);

  const top = useMemo(
    () =>
      workspace
        ? [...workspace.elites].sort((left, right) => right.evidence - left.evidence).slice(0, 6)
        : [],
    [workspace],
  );
  const progress = discoverProgress(
    job?.completedGenerations ?? 0,
    job?.requestedGenerations ?? 0,
    job?.runUntilStopped ?? true,
  );
  const progressLabel = discoverProgressLabel(
    job?.completedGenerations ?? 0,
    job?.requestedGenerations ?? 0,
    job?.runUntilStopped ?? true,
  );
  const best = top[0] ?? null;

  return (
    <div className="overview-content">
      <section className={`panel overview-status job-${job?.status ?? "idle"}`}>
        <div className="panel-heading">
          <div>
            <p className="eyebrow">Discover</p>
            <h2>{job?.phase ?? "No search has run on this machine yet"}</h2>
          </div>
          <span className="job-status">{job?.status ?? "idle"}</span>
        </div>
        <div className="overview-status-body">
          <p className="overview-status-message">
            {job?.message ?? "Bind a symbol in Discover to start the local Rust worker."}
          </p>
          {(active || (job?.completedGenerations ?? 0) > 0) && (
            <>
              <div className="progress-track"><i style={{ width: `${progress}%` }} /></div>
              <div className="progress-label">
                <span>{progressLabel}</span>
                <strong>{job?.runUntilStopped && (job?.requestedGenerations ?? 0) <= 0 ? "∞" : `${progress.toFixed(0)}%`}</strong>
              </div>
            </>
          )}
          <div className="overview-status-kpis">
            <KpiCell label="Evaluated" value={formatNumber(job?.evaluationCount ?? 0)} note="candidates looked at" />
            <KpiCell label="Initial pot" value={formatNumber(job?.potElites ?? 0)} note={job?.breedingActive ? "breeding" : `fill → ${formatNumber(job?.mutateAfterElites ?? 0)}`} />
            <KpiCell label="Holding" value={formatNumber(job?.holdingElites ?? 0)} note="M1 80/130" />
            <KpiCell label="Databank" value={formatNumber(job?.databankElites ?? 0)} note="after Holding battery" />
            <KpiCell
              label="Evals / hour · rolling"
              value={formatNumber(job?.rollingEvaluationsPerHour ?? 0)}
              note={`${formatNumber(job?.lifetimeEvaluationsPerHour ?? job?.evaluationsPerHour ?? 0)} lifetime avg · ${job?.workerThreads ?? "—"} scout${job?.breedingActive ? ` · queue ${job?.promotionQueueDepth ?? 0}` : ""}`}
            />
          </div>
          <div className="overview-status-actions">
            <button className="primary" type="button" onClick={() => onNavigate("Discover")}>
              {active ? "Open live progress" : "Configure Discover"}
            </button>
            {job?.status === "completed" && job.outputPath && (
              <button className="secondary" type="button" onClick={() => void onOpenPath(job.outputPath!)}>
                Open checkpoint in Databank
              </button>
            )}
            <button className="secondary" type="button" onClick={() => onNavigate("Data Lab")}>Inspect data</button>
            <button className="secondary" type="button" onClick={onOpen}>
              {hasDatabank ? "Open another databank" : "Open databank"}
            </button>
          </div>
        </div>
      </section>

      <div className="overview-grid">
        {hasDatabank ? (
          <section className="panel overview-databank">
            <div className="panel-heading">
              <div>
                <p className="eyebrow">Loaded databank</p>
                <h2>{workspace.sourcePath.split("/").at(-1)}</h2>
              </div>
              <span className="read-only-badge">
                {workspace.researchGrade ? "research grade" : workspace.m1FidelityVerified ? "M1 verified" : "promotion grade"}
              </span>
            </div>
            <div className="overview-status-kpis">
              <KpiCell label="Elites" value={formatNumber(workspace.elites.length)} note={`${workspace.coverage} / ${workspace.totalNiches} niches`} />
              <KpiCell label="QD score" value={formatNumber(workspace.qdScore, 2)} note="positive evidence mass" />
              <KpiCell
                label="Best return"
                value={best ? `${best.returnPercent.toFixed(2)}%` : "—"}
                note={best ? `DD ${best.drawdownPercent.toFixed(2)}%` : "no elites"}
                tone={best && best.returnPercent >= 0 ? "positive" : undefined}
              />
              <KpiCell label="Data quality" value={`${workspace.qualityGrade} · ${workspace.qualityScore}`} note={`run ${workspace.runId.slice(0, 10)}`} />
            </div>
            <div className="overview-top-list">
              <div className="overview-top-head">
                <span>Top strategies by evidence</span>
                <button className="text-action" type="button" onClick={() => onNavigate("Databank")}>
                  Open databank →
                </button>
              </div>
              {top.map((elite) => (
                <button
                  className="overview-top-row"
                  key={elite.fingerprint}
                  onClick={() => onOpenResult(elite.fingerprint)}
                  title={elite.fingerprint}
                  type="button"
                >
                  <strong>{elite.strategyId}</strong>
                  <span>{conditionLabel(elite.entryConditions, elite.exitConditions)}</span>
                  <span>Ev {elite.evidence.toFixed(2)}</span>
                  <span className={elite.returnPercent >= 0 ? "positive" : "negative"}>
                    {elite.returnPercent.toFixed(2)}%
                  </span>
                  <span className="negative">-{elite.drawdownPercent.toFixed(2)}%</span>
                  <span>{formatNumber(elite.trades)} trades</span>
                </button>
              ))}
            </div>
          </section>
        ) : (
          <section className="panel overview-empty">
            <div className="panel-heading">
              <div>
                <p className="eyebrow">Databank</p>
                <h2>No archive loaded</h2>
              </div>
            </div>
            <p>
              Open a validated databank JSON to see KPIs, top strategies and full result detail here.
              QuantForge revalidates the run manifest, fingerprints and niche map before displaying anything.
            </p>
            <button className="primary" type="button" onClick={onOpen}>Open databank</button>
          </section>
        )}

        <section className="panel overview-checklist">
          <div className="panel-heading">
            <div>
              <p className="eyebrow">Getting started</p>
              <h2>Research path</h2>
            </div>
          </div>
          <ChecklistRow
            done={rangeProfiles !== null && rangeProfiles > 0}
            label="Save a search-range profile"
            note={rangeProfiles === null ? "profiles unavailable" : `${rangeProfiles} saved`}
            action="Settings"
            onAction={() => onNavigate("Search Settings")}
          />
          <ChecklistRow
            done={job?.jobId !== null && job?.jobId !== undefined}
            label="Run Discover on a bound symbol"
            note={job?.jobId ? `last job ${job.status}` : "no job yet"}
            action="Discover"
            onAction={() => onNavigate("Discover")}
          />
          <ChecklistRow
            done={hasDatabank}
            label="Open the resulting databank"
            note={hasDatabank ? `${formatNumber(workspace.elites.length)} elites` : "not loaded"}
            action="Databank"
            onAction={hasDatabank ? () => onNavigate("Databank") : onOpen}
          />
          <ChecklistRow
            done={hasDatabank && top.length > 0}
            label="Inspect a result and its robustness evidence"
            note={best ? `start with ${best.strategyId}` : "needs elites"}
            action="Open"
            onAction={best ? () => onOpenResult(best.fingerprint) : () => onNavigate("Databank")}
          />
          <ChecklistRow
            done={false}
            label="Retest on M1 / MT5, then stage for certification"
            note="Retester → Databank · Certified"
            action="Retester"
            onAction={() => onNavigate("Parity Lab")}
          />
        </section>
      </div>
    </div>
  );
}

function ChecklistRow({
  done,
  label,
  note,
  action,
  onAction,
}: {
  done: boolean;
  label: string;
  note: string;
  action: string;
  onAction: () => void;
}) {
  return (
    <div className={done ? "checklist-row done" : "checklist-row"}>
      <span className="checklist-mark" aria-hidden="true">
        {done ? (
          <svg viewBox="0 0 24 24" className="nav-glyph"><path d="M5 12.5 10 17.5 19 7" /></svg>
        ) : (
          <i />
        )}
      </span>
      <div>
        <strong>{label}</strong>
        <small>{note}</small>
      </div>
      <button className="text-action" type="button" onClick={onAction}>{action} →</button>
    </div>
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
  const [importDirectory, setImportDirectory] = useState("");
  const [importOutputDirectory, setImportOutputDirectory] = useState("");
  const [importTimezone, setImportTimezone] = useState("ICMarkets/EST+7");
  const [importReport, setImportReport] = useState<MarketFolderImportView | null>(null);
  const [importBusy, setImportBusy] = useState(false);
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

  async function importFolder() {
    if (!importDirectory) return;
    setImportBusy(true);
    setImportReport(null);
    onError(null);
    try {
      const imported = await importMarketFolder({
        sourceDirectory: importDirectory,
        outputDirectory: importOutputDirectory || null,
        sourceTimezone: importTimezone,
        aggregateTicksToBars: true,
      });
      setImportReport(imported);
    } catch (reason) {
      onError(String(reason));
    } finally {
      setImportBusy(false);
    }
  }

  function useImportedFile(file: MarketFolderImportView["files"][number]) {
    const data = file.m1Path ?? file.h1Path;
    const metadata = file.m1MetadataPath ?? file.h1MetadataPath;
    if (data) setDataPath(data);
    if (metadata) setMetadataPath(metadata);
    setReport(null);
    onError(null);
  }

  return (
    <div className="tool-content">
      <section className="panel setup-panel import-panel">
        <div className="panel-heading">
          <div><p className="eyebrow">IC Markets import</p><h2>Bring in broker CSVs</h2></div>
          <span className="read-only-badge">Safe copy</span>
        </div>
        <p className="immutable-note">Choose a Downloads folder. QuantForge scans it recursively, ignores trade lists and unrelated schemas, preserves bid/ask quotes in a canonical M1 sidecar, and derives H1 from the bid M1 stream.</p>
        <div className="form-stack compact">
          <PathField label="CSV folder" path={importDirectory} choose={() => chooseDirectory("Choose IC Markets CSV folder")} onChange={setImportDirectory} required />
          <PathField label="Output folder (optional)" path={importOutputDirectory} choose={() => chooseDirectory("Choose import output folder")} onChange={setImportOutputDirectory} />
          <label className="field-row"><span>Broker wall-clock</span><select value={importTimezone} onChange={(event) => setImportTimezone(event.target.value)}><option value="ICMarkets/EST+7">ICMarkets / EST+7 (recommended)</option><option value="Etc/UTC">UTC</option></select></label>
        </div>
        <div className="form-footer"><p>Existing files are never overwritten; each import receives a timestamped output directory when none is chosen.</p><button className="primary" disabled={importBusy || !importDirectory} onClick={importFolder}>{importBusy ? "Scanning and converting…" : "Import market CSVs"}</button></div>
      </section>
      {importReport && <section className="panel import-results">
        <div className="panel-heading"><div><p className="eyebrow">Import complete</p><h2>{formatNumber(importReport.importedCount)} datasets ready</h2></div><span className="read-only-badge">{formatNumber(importReport.skippedCount)} skipped</span></div>
        <p className="immutable-note">Output: <code>{importReport.outputDirectory}</code>. Tick imports produce a bid M1 CSV plus a matching <code>.quotes.csv</code> sidecar; select a row to load its canonical path into diagnostics.</p>
        <div className="import-file-list">{importReport.files.map((file) => <div className={`import-file-row import-${file.status}`} key={file.sourcePath}><div><strong>{file.symbol ?? "Unknown"}</strong><span>{file.kind} · {file.status}{file.bars ? ` · ${formatNumber(file.bars)} M1 bars` : ""}</span>{file.message && <small>{file.message}</small>}</div><div className="button-row">{file.status === "imported" && <button type="button" className="secondary" onClick={() => useImportedFile(file)}>Use</button>}</div></div>)}</div>
      </section>}
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
          <PathField label="Decision / imported OHLC" path={dataPath} choose={chooseDataFile} onChange={setDataPath} required />
          <PathField label="Metadata (optional if timezone is set)" path={metadataPath} choose={chooseMetadataFile} onChange={setMetadataPath} />
          <PathField label="Broker profile" path={brokerPath} choose={chooseBrokerFile} onChange={setBrokerPath} required />
          <details className="advanced-settings">
            <summary>Bound paths (auto-filled)</summary>
            <div className="form-stack compact">
              <label className="field-row"><span>Decision TF</span><code>{dataPath || "—"}</code></label>
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
          <h2>{report.discoverReady ? "Bound and ready for discovery" : report.feedMode === "diagnostic_midpoint" ? "Diagnostic-only midpoint feed" : q.grade === "fail" ? "Source is blocked" : "Broker binding still required"}</h2>
          <p>{formatNumber(report.bars)} normalized bars from {new Date(report.firstTimestampMs).toLocaleString()} to {new Date(report.lastTimestampMs).toLocaleString()}.</p>
          <p className="immutable-note">Feed: <code>{report.feedMode}</code>{report.quotePath ? <> · quote sidecar: <code>{report.quotePath}</code></> : " · no bid/ask sidecar bound"}</p>
        </div>
        <button className="primary" disabled={!report.discoverReady} onClick={onUse}>Use in Discover</button>
      </section>
      <section className="kpi-grid diagnostics-grid">
        <Kpi label="Rows read" value={formatNumber(report.sourceRows)} note={`${formatNumber(report.duplicateRowsRemoved)} exact duplicates removed`} />
        <Kpi label="Interval" value={q.expectedIntervalSeconds ? `${q.expectedIntervalSeconds}s` : "Unknown"} note={`Delimiter ${JSON.stringify(report.delimiter)}`} />
        <Kpi label="Symbol / timeframe" value={`${report.symbol ?? "—"} · ${report.timeframe ?? "—"}`} note={report.brokerProfile ?? "No broker profile bound"} />
        <Kpi label="Ordering" value={report.inputWasSorted ? "Sorted" : "Normalized"} note={report.sourceTimezone} />
        <Kpi label="Certification" value={report.certificationReady ? "Bid/ask ready" : "Diagnostic only"} note={report.feedMode} />
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

function SearchSettingsWorkspace({ onError }: { onError: (message: string | null) => void }) {
  const [profiles, setProfiles] = useState<SavedSearchRangeProfile[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [name, setName] = useState("H1-M15 default");
  const [ranges, setRanges] = useState<SearchRangeProfile>(DEFAULT_SEARCH_RANGES);
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState(false);
  const refresh = () => listSearchRangeProfiles().then(setProfiles).catch((reason) => onError(String(reason)));
  useEffect(() => { refresh(); }, []);
  function load(profile: SavedSearchRangeProfile) {
    setSelectedId(profile.id);
    setName(profile.name);
    setRanges(normalizeSearchRanges(profile.ranges));
    setDirty(false);
  }
  const rangesAreValid = Object.values(ranges).every((range) => Number.isFinite(range.minimum) && Number.isFinite(range.maximum) && Number.isFinite(range.step) && range.minimum <= range.maximum && range.step > 0);
  async function saveProfile() {
    setBusy(true); onError(null);
    try {
      const next = await saveSearchRangeProfile({ id: selectedId ?? "", name, ranges, updatedAt: "" });
      setProfiles(next);
      const saved = next.find((profile) => profile.name === name);
      if (saved) setSelectedId(saved.id);
      setDirty(false);
    } catch (reason) { onError(String(reason)); } finally { setBusy(false); }
  }
  useEffect(() => {
    if (!selectedId || !dirty || !rangesAreValid || busy) return;
    const timer = window.setTimeout(() => { void saveProfile(); }, 700);
    return () => window.clearTimeout(timer);
  }, [selectedId, name, ranges, dirty, rangesAreValid, busy]);
  async function removeProfile() {
    if (!selectedId) return;
    setBusy(true); onError(null);
    try {
      setProfiles(await deleteSearchRangeProfile(selectedId));
      setSelectedId(null); setName("H1-M15 default"); setRanges(DEFAULT_SEARCH_RANGES); setDirty(false);
    } catch (reason) { onError(String(reason)); } finally { setBusy(false); }
  }
  const state: { tone: "saved" | "dirty" | "invalid" | "new"; label: string } = !rangesAreValid
    ? { tone: "invalid", label: "Invalid ranges" }
    : busy
      ? { tone: "dirty", label: "Saving…" }
      : !selectedId
        ? { tone: "new", label: "Unsaved profile" }
        : dirty
          ? { tone: "dirty", label: "Unsaved edits" }
          : { tone: "saved", label: "Saved locally" };

  return <div className="settings-workspace">
    <section className="panel settings-profile-card">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Reusable research contract</p>
          <h2>Search settings</h2>
        </div>
        <span className={`settings-state ${state.tone}`}>{state.label}</span>
      </div>
      <div className="settings-profile-body">
        <label className="field-row">
          <span>Profile name</span>
          <input
            value={name}
            onChange={(event) => { setName(event.target.value); setDirty(true); }}
            placeholder="e.g. H1 conservative plateau"
          />
        </label>
        <BuiltInSearchPresetPicker
          value={ranges}
          onApply={(preset) => {
            setRanges({ ...preset.ranges });
            if (!selectedId) setName(preset.label);
            setDirty(true);
          }}
        />
        <div className="settings-profile-actions">
          <button className="secondary" disabled={busy} onClick={() => { setSelectedId(null); setName("New profile"); setRanges(DEFAULT_SEARCH_RANGES); setDirty(false); }}>New</button>
          <button className="primary" disabled={busy || !name.trim() || !rangesAreValid} onClick={() => void saveProfile()}>
            {busy ? "Saving…" : selectedId ? "Save now" : "Save profile"}
          </button>
          {selectedId && <button className="danger" disabled={busy} onClick={() => void removeProfile()}>Delete</button>}
        </div>
      </div>
      <p className="settings-help">
        {!rangesAreValid
          ? "Fix each range so minimum ≤ maximum and step is positive before saving."
          : selectedId
            ? dirty
              ? "Edits to this saved profile persist automatically after a short pause."
              : "This profile is stored locally; edits save automatically."
            : "Name the profile and save it once, then later edits save automatically."}{" "}
        The selected profile is sealed into every new databank, so an existing archive can never be re-scoped.
      </p>
    </section>

    <section className="panel settings-library-card">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Persistent library</p>
          <h2>Saved profiles</h2>
        </div>
        <span className="read-only-badge">{profiles.length} stored</span>
      </div>
      {profiles.length ? (
        <div className="profile-list">
          {profiles.map((profile) => (
            <button
              key={profile.id}
              className={profile.id === selectedId ? "profile-row active" : "profile-row"}
              onClick={() => load(profile)}
              type="button"
            >
              <strong>{profile.name}</strong>
              <small>Updated {new Date(profile.updatedAt).toLocaleString()}</small>
            </button>
          ))}
        </div>
      ) : (
        <p className="settings-help">No saved profiles yet. Set the ranges below, then save them with a name.</p>
      )}
    </section>

    <section className="panel settings-ranges-card">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Allowed numeric genes</p>
          <h2>Minimum / maximum / step</h2>
        </div>
        <span className="read-only-badge">sealed per databank</span>
      </div>
      <SearchRangesWall value={ranges} onChange={(next) => { setRanges(next); setDirty(true); }} />
    </section>
  </div>;
}

function BuiltInSearchPresetPicker({
  value,
  onApply,
}: {
  value: SearchRangeProfile;
  onApply: (preset: (typeof BUILT_IN_SEARCH_PRESETS)[number]) => void;
}) {
  const active = detectBuiltInSearchPreset(value);
  return (
    <div className="field-row">
      <span>Built-in range preset</span>
      <div className="mode-toggle">
        {BUILT_IN_SEARCH_PRESETS.map((preset) => (
          <button
            key={preset.id}
            type="button"
            className={active === preset.id ? "active" : ""}
            onClick={() => onApply(preset)}
            title={preset.note}
          >
            {preset.label}
          </button>
        ))}
      </div>
      <small>
        {active === "custom"
          ? "Custom / saved ranges — pick a built-in preset to replace them, or keep editing."
          : BUILT_IN_SEARCH_PRESETS.find((preset) => preset.id === active)?.note}
      </small>
    </div>
  );
}

function SavedRangeProfilePicker({ value, onChange, onError }: { value: SearchRangeProfile; onChange: (ranges: SearchRangeProfile) => void; onError: (message: string | null) => void }) {
  const [profiles, setProfiles] = useState<SavedSearchRangeProfile[]>([]);
  const [selectedId, setSelectedId] = useState("");
  useEffect(() => { void listSearchRangeProfiles().then(setProfiles).catch((reason) => onError(String(reason))); }, []);
  const selected = profiles.find((profile) => profile.id === selectedId);
  return <label className="field-row"><span>Saved profile</span><select value={selectedId} onChange={(event) => { const nextId = event.target.value; setSelectedId(nextId); const match = profiles.find((profile) => profile.id === nextId); if (match) onChange(normalizeSearchRanges(match.ranges)); }}><option value="">Current job profile</option>{profiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}</select><small>{selected ? `Applied to this new job: ${selected.name}. The job will use its sealed ranges.` : profiles.length ? "Choose a profile to replace the current ranges for this new job." : "Create a reusable profile in Settings first."}</small></label>;
}

function DiscoverWorkspace({
  onError,
  onOpenDatabank,
  onLiveWorkspace,
  onResultsView,
  preset,
}: {
  onError: (message: string | null) => void;
  onOpenDatabank: (path: string) => void;
  onLiveWorkspace?: (workspace: DatabankWorkspace) => void;
  onResultsView?: (open: boolean) => void;
  preset: Partial<DiscoverRequest>;
}) {
  const [form, setForm] = useState<DiscoverRequest>(() => ({
    mode: "new",
    selectedSymbol: null,
    dataPath: "",
    decisionTimeframe: "H1",
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
    universalGrammar: { ...DEFAULT_UNIVERSAL_GRAMMAR },
    runMode: "full_harvest" as DiscoverRunModeId,
    generalIslandCount: 0,
    refinementIslandCount: 0,
    explorationIslandCount: 0,
    migrationInterval: 10,
    migrationElites: 2,
    earlyStopPotElites: null,
    targetDatabankElites: null,
    searchRanges: DEFAULT_SEARCH_RANGES,
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
    minimumM1ReturnRetention: 0.80,
    minimumDevelopmentExpectancyR: 0.25,
    oos1ExpectancyRetention: 0.7,
    requireM1Precision: true,
    simpleExits: true,
    allowBreakEven: false,
    allowTrailingStops: false,
    allowPartialExits: false,
    allowMarketEntries: true,
    allowStopEntries: false,
    allowLimitEntries: false,
    flattenAt22: false,
    endOfDayHour: 23,
    entryWindowStartHour: 2,
    entryWindowEndHour: 19,
    maxOneEntryPerDay: true,
    mutateAfterElites: 300,
    randomFillFraction: 0.75,
    workerThreads: 0,
    promotionWorkerThreads: 0,
    promotionQueueCapacity: 64,
    maxMemoryMb: 8192,
    requireM1Robustness: true,
    buildToHolding: true,
    robustnessFolds: 3,
    robustnessMonteCarloTrials: 250,
    robustnessMonteCarloBlockLength: 5,
    robustnessMonteCarloSkipTradeProbability: 0.10,
    robustnessMonteCarloP80ProfitRetention: 0.60,
    robustnessMonteCarloMaxDrawdownRatio: 1.75,
    robustnessNeighborhoodSamples: 200,
    robustnessPerturbationFraction: 0.2,
    minimumNeighborhoodSurvivalFraction: 0.55,
    calendarYearFolds: false,
    minimumDeflatedTradeSharpe: null,
    multiSymbolMinimumPass: 0,
    packDataDir: null,
    commissionPerLotRoundTurn: 7,
    slippagePointsPerSide: 0,
    fallbackSpreadPoints: null,
    maxSpreadPoints: null,
    initialBalance: 100000,
    promotionSplit: true,
    validationFraction: 0,
    sealedFraction: 1 / 3,
    historyStartYear: 2016,
    factoryAfterDiscover: false,
    factoryQueueLimit: 0,
    factoryTargetDatabank: 0,
    factoryMaxCorrelation: 0.5,
    ...preset,
  }));
  const [discoverProfiles, setDiscoverProfiles] = useState<SavedDiscoverProfile[]>([]);
  const [selectedDiscoverProfileId, setSelectedDiscoverProfileId] = useState("");
  const [discoverProfileName, setDiscoverProfileName] = useState("My Discover profile");
  const [discoverProfileBusy, setDiscoverProfileBusy] = useState(false);
  const [job, setJob] = useState<DiscoverJobView | null>(null);
  const [busy, setBusy] = useState(false);
  const [testerCounts, setTesterCounts] = useState<number[]>([2, 3, 4]);
  const [testerGenerations, setTesterGenerations] = useState(3);
  const [testerCandidates, setTesterCandidates] = useState(60);
  const [testerSeed, setTesterSeed] = useState(42);
  const [testerBusy, setTesterBusy] = useState(false);
  const [testerReport, setTesterReport] = useState<ConditionBakeoffReport | null>(null);
  const [timeframeBakeoffDraws, setTimeframeBakeoffDraws] = useState(20);
  const [timeframeBakeoffBusy, setTimeframeBakeoffBusy] = useState(false);
  const [timeframeBakeoffReport, setTimeframeBakeoffReport] = useState<TimeframeBakeoffReport | null>(null);
  const [resultsViewFp, setResultsViewFp] = useState<string | null>(null);
  const [liveWorkspace, setLiveWorkspace] = useState<DatabankWorkspace | null>(null);
  const [liveSelected, setLiveSelected] = useState<string | null>(null);
  const [liveDetail, setLiveDetail] = useState<EliteDetail | null>(null);
  const [liveDetailOpen, setLiveDetailOpen] = useState(false);
  const [liveDetailLoading, setLiveDetailLoading] = useState(false);
  const [discoverTab, setDiscoverTab] = useState<"progress" | "settings" | "results">("settings");
  const [expertMode, setExpertMode] = useState(false);
  const [liveInspectorTab, setLiveInspectorTab] = useState<"overview" | "ir" | "mq5">("overview");
  const lastDatabankCountRef = useRef(0);
  const lastLiveRevisionRef = useRef(0);
  const liveReloadBusyRef = useRef(false);
  const active = job?.status === "running" || job?.status === "paused";

  useEffect(() => {
    void getDiscoverJob().then(setJob).catch((reason) => onError(String(reason)));
  }, []);

  useEffect(() => {
    void listDiscoverProfiles().then(setDiscoverProfiles).catch((reason) => onError(String(reason)));
  }, []);

  useEffect(() => {
    if (active) setDiscoverTab("progress");
  }, [active]);

  // The always-on strip collapses while this workspace shows its own results
  // page, so the screenshot-style elite table is never rendered twice.
  useEffect(() => {
    onResultsView?.(resultsViewFp !== null);
    return () => onResultsView?.(false);
  }, [resultsViewFp]);

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => {
      void getDiscoverJob().then(setJob).catch((reason) => onError(String(reason)));
    }, 500);
    return () => window.clearInterval(timer);
  }, [active]);

  useEffect(() => {
    if (!job?.outputPath) return;
    const eliteCount = (job.holdingElites ?? 0) + (job.databankElites ?? 0);
    const liveRevision = job.liveDatabankRevision ?? eliteCount;
    const shouldReload =
      job.status === "completed"
      || (active
        && eliteCount > 0
        && (liveRevision !== lastLiveRevisionRef.current
          || eliteCount !== lastDatabankCountRef.current));
    if (!shouldReload || liveReloadBusyRef.current) return;

    const timer = window.setTimeout(() => {
      liveReloadBusyRef.current = true;
      void (active ? getDiscoverLiveDatabank() : loadDatabankPath(job.outputPath!))
        .then((workspace) => {
          lastDatabankCountRef.current = eliteCount;
          lastLiveRevisionRef.current = liveRevision;
          setLiveWorkspace(workspace);
          onLiveWorkspace?.(workspace);
        })
        .catch((reason) => onError(`Live databank refresh failed: ${String(reason)}`))
        .finally(() => {
          liveReloadBusyRef.current = false;
        });
    }, active ? 1800 : 0);
    return () => window.clearInterval(timer);
  }, [active, job?.outputPath, job?.holdingElites, job?.databankElites, job?.liveDatabankRevision, job?.status]);

  const liveEliteRows = useMemo(() => {
    if (!liveWorkspace) return [];
    return [...(liveWorkspace.holding ?? []), ...liveWorkspace.elites].sort(
      (left, right) => (right.foldMedianR ?? right.evidence) - (left.foldMedianR ?? left.evidence),
    );
  }, [liveWorkspace]);

  async function inspectLiveElite(fingerprint: string) {
    if (!job?.outputPath) return;
    setLiveDetailLoading(true);
    setLiveDetailOpen(true);
    setLiveInspectorTab("overview");
    setLiveSelected(fingerprint);
    onError(null);
    try {
      if (active) await getDiscoverLiveDatabank();
      else await loadDatabankPath(job.outputPath);
      setLiveDetail(await loadElite(fingerprint));
    } catch (reason) {
      onError(String(reason));
      setLiveDetailOpen(false);
    } finally {
      setLiveDetailLoading(false);
    }
  }

  function update<K extends keyof DiscoverRequest>(key: K, value: DiscoverRequest[K]) {
    setForm((current) => ({ ...current, [key]: value }));
  }

  function discoverProfileSnapshot(): DiscoverRequest {
    return {
      ...form,
      mode: "new",
      // A profile must never point a later search at the previous immutable databank.
      databankPath: "",
      promotionSplit: true,
      searchRanges: normalizeSearchRanges(form.searchRanges ?? DEFAULT_SEARCH_RANGES),
    };
  }

  function applyDiscoverProfile(profile: SavedDiscoverProfile) {
    const saved = profile.settings;
    setForm((current) => ({
      ...current,
      ...saved,
      selectedSymbol:
        saved.selectedSymbol
        ?? symbolFromDataPath(saved.brokerPath)
        ?? symbolFromDataPath(saved.dataPath),
      mode: "new",
      databankPath: "",
      promotionSplit: true,
      searchRanges: normalizeSearchRanges(saved.searchRanges ?? DEFAULT_SEARCH_RANGES),
    }));
    setSelectedDiscoverProfileId(profile.id);
    setDiscoverProfileName(profile.name);
  }

  async function saveCurrentDiscoverProfile() {
    if (!discoverProfileName.trim()) {
      onError("Give the Discover profile a name before saving it.");
      return;
    }
    setDiscoverProfileBusy(true);
    onError(null);
    try {
      const profiles = await saveDiscoverProfile({
        id: selectedDiscoverProfileId,
        name: discoverProfileName,
        settings: discoverProfileSnapshot(),
        updatedAt: "",
      });
      setDiscoverProfiles(profiles);
      const saved = profiles.find((profile) => profile.id === selectedDiscoverProfileId)
        ?? profiles.filter((profile) => profile.name === discoverProfileName.trim()).at(-1);
      if (saved) setSelectedDiscoverProfileId(saved.id);
    } catch (reason) {
      onError(String(reason));
    } finally {
      setDiscoverProfileBusy(false);
    }
  }

  async function removeCurrentDiscoverProfile() {
    if (!selectedDiscoverProfileId) return;
    setDiscoverProfileBusy(true);
    onError(null);
    try {
      setDiscoverProfiles(await deleteDiscoverProfile(selectedDiscoverProfileId));
      setSelectedDiscoverProfileId("");
      setDiscoverProfileName("My Discover profile");
    } catch (reason) {
      onError(String(reason));
    } finally {
      setDiscoverProfileBusy(false);
    }
  }

  async function chooseOutput() {
    return form.mode === "new" ? chooseNewDatabank() : chooseDatabank();
  }

  async function start() {
    const blocker = entryWindowError(form) ?? entryOrderError(form) ?? perturbationError(form);
    if (blocker) {
      onError(blocker);
      return;
    }
    setBusy(true);
    onError(null);
    try {
      const sourceBoundForm = bindDiscoverTimezone({
        ...form,
        selectedSymbol:
          form.selectedSymbol
          ?? symbolFromDataPath(form.brokerPath)
          ?? symbolFromDataPath(form.dataPath),
        promotionSplit: true,
        // Generations 0 means SQX-style run-until-stopped, never a hard start block.
        runUntilStopped: (form.runUntilStopped ?? true) || form.generations < 1,
        // Open-ended Discover never inherits a factory cap. When you press
        // Stop, its battery examines every frozen Holding strategy.
        factoryTargetDatabank: (form.runUntilStopped ?? true) ? 0 : form.factoryTargetDatabank,
      });
      const request = form.mode === "continue"
          ? {
            ...sourceBoundForm,
            decisionTimeframe: null,
            initialCandidates: null,
            batchSize: null,
            correlationThreshold: null,
            noveltyWeight: null,
            seed: null,
            runMode: null,
            generalIslandCount: null,
            refinementIslandCount: null,
            explorationIslandCount: null,
            migrationInterval: null,
            migrationElites: null,
            earlyStopPotElites: null,
            targetDatabankElites: null,
            searchRanges: null,
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
            minimumDevelopmentExpectancyR: null,
            oos1ExpectancyRetention: null,
            requireM1Precision: null,
            simpleExits: null,
            allowBreakEven: null,
            allowTrailingStops: null,
            allowPartialExits: null,
            allowMarketEntries: null,
            allowStopEntries: null,
            allowLimitEntries: null,
            flattenAt22: null,
            entryWindowStartHour: null,
            entryWindowEndHour: null,
            maxOneEntryPerDay: null,
            mutateAfterElites: null,
            randomFillFraction: null,
            workerThreads: null,
            promotionWorkerThreads: null,
            promotionQueueCapacity: null,
            requireM1Robustness: null,
            buildToHolding: null,
            robustnessFolds: null,
            robustnessMonteCarloTrials: null,
            robustnessNeighborhoodSamples: null,
            robustnessPerturbationFraction: null,
            minimumNeighborhoodSurvivalFraction: null,
            calendarYearFolds: null,
            minimumDeflatedTradeSharpe: null,
            multiSymbolMinimumPass: null,
            packDataDir: null,
            commissionPerLotRoundTurn: null,
            slippagePointsPerSide: null,
            fallbackSpreadPoints: null,
            maxSpreadPoints: null,
            initialBalance: null,
            promotionSplit: null,
            validationFraction: null,
            sealedFraction: null,
            historyStartYear: null,
          }
        : sourceBoundForm;
      const started = await startDiscover(request);
      setJob(started);
      if (form.mode === "new" && started.outputPath) {
        setForm((current) => ({ ...current, databankPath: started.outputPath! }));
      }
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

  function toggleTesterCount(count: number) {
    setTesterCounts((current) =>
      current.includes(count)
        ? current.filter((value) => value !== count)
        : [...current, count].sort(),
    );
  }

  async function runTester() {
    if (testerCounts.length < 1) {
      onError("Select at least one entry-condition count for the condition bakeoff.");
      return;
    }
    if (!form.dataPath || !form.m1DataPath || !form.brokerPath) {
      onError("Choose a symbol (decision timeframe, M1, broker) before running the condition bakeoff.");
      return;
    }
    setTesterBusy(true);
    onError(null);
    try {
      const bound = bindDiscoverTimezone(form);
      const report = await runConditionBakeoff({
        dataPath: bound.dataPath,
        metadataPath: bound.metadataPath,
        sourceTimezone: bound.sourceTimezone,
        m1DataPath: bound.m1DataPath,
        m1MetadataPath: bound.m1MetadataPath,
        m1SourceTimezone: bound.m1SourceTimezone,
        brokerPath: bound.brokerPath,
        generations: Math.max(1, testerGenerations),
        initialCandidates: Math.max(20, testerCandidates),
        seed: testerSeed,
        commissionPerLotRoundTurn: form.commissionPerLotRoundTurn ?? 7,
        slippagePointsPerSide: form.slippagePointsPerSide ?? 0,
        fallbackSpreadPoints: form.fallbackSpreadPoints,
        validationFraction: form.validationFraction ?? 0,
        sealedFraction: form.sealedFraction ?? 1 / 3,
        entryConditionCounts: testerCounts,
        historyStartYear: form.historyStartYear ?? 2016,
      });
      setTesterReport(report);
    } catch (reason) {
      onError(String(reason));
    } finally {
      setTesterBusy(false);
    }
  }

  async function runTimeframeComparison() {
    if (!form.dataPath || !form.m1DataPath || !form.brokerPath) {
      onError("Choose a symbol (H1 source, M1, broker) before running the timeframe bakeoff.");
      return;
    }
    setTimeframeBakeoffBusy(true);
    onError(null);
    try {
      const bound = bindDiscoverTimezone(form);
      const grammar = form.universalGrammar ?? DEFAULT_UNIVERSAL_GRAMMAR;
      const report = await runTimeframeBakeoff({
        dataPath: bound.dataPath,
        metadataPath: bound.metadataPath,
        sourceTimezone: bound.sourceTimezone,
        m1DataPath: bound.m1DataPath,
        m1MetadataPath: bound.m1MetadataPath,
        m1SourceTimezone: bound.m1SourceTimezone,
        brokerPath: bound.brokerPath,
        drawsPerCell: Math.max(1, timeframeBakeoffDraws),
        seed: form.seed ?? 42,
        minimumTrades: form.minimumTrades ?? 10,
        minimumReturnPercent: form.minimumReturnPercent ?? 0,
        minimumProfitFactor: form.minimumProfitFactor ?? 1,
        maximumDrawdownPercent: form.maximumDrawdownPercent ?? 40,
        oos1Retention: form.oos1ExpectancyRetention ?? 0.7,
        commissionPerLotRoundTurn: form.commissionPerLotRoundTurn ?? 7,
        slippagePointsPerSide: form.slippagePointsPerSide ?? 0,
        fallbackSpreadPoints: form.fallbackSpreadPoints,
        validationFraction: (form.validationFraction ?? 0) > 0 ? form.validationFraction! : 0.2,
        sealedFraction: form.sealedFraction ?? 1 / 3,
        minimumEntryConditions: grammar.minimumEntryConditions,
        maximumEntryConditions: grammar.maximumEntryConditions,
        minimumExitConditions: grammar.minimumExitConditions,
        maximumExitConditions: grammar.maximumExitConditions,
        historyStartYear: form.historyStartYear ?? 2016,
      });
      setTimeframeBakeoffReport(report);
    } catch (reason) {
      onError(String(reason));
    } finally {
      setTimeframeBakeoffBusy(false);
    }
  }

  function applyRecommendedCount(count: number) {
    setForm((current) => ({
      ...current,
      mode: "new",
      universalGrammar: {
        ...(current.universalGrammar ?? DEFAULT_UNIVERSAL_GRAMMAR),
        minimumEntryConditions: count,
        maximumEntryConditions: count,
      },
    }));
  }

  function updateGrammar<K extends keyof typeof DEFAULT_UNIVERSAL_GRAMMAR>(
    key: K,
    value: number,
  ) {
    setForm((current) => ({
      ...current,
      universalGrammar: {
        ...(current.universalGrammar ?? DEFAULT_UNIVERSAL_GRAMMAR),
        [key]: value,
      },
    }));
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
  const rejected = job?.rejectedTotal ?? 0;
  const resolvedSymbol =
    form.selectedSymbol
    || symbolFromDataPath(form.brokerPath)
    || symbolFromDataPath(form.dataPath);
  const startBlocker = active
    ? "A Discover job is already running. Open Live progress to pause or stop it."
    : busy
      ? "Starting…"
      : !resolvedSymbol
        ? "Choose a symbol. If the list is empty, the IC Markets 2016 pack is not visible to the app."
        : !form.dataPath || !form.m1DataPath || !form.brokerPath
          ? "Symbol paths are incomplete (H1, M1, or broker missing)."
          : form.mode === "continue" && !form.databankPath
            ? "Continuation needs an existing databank path."
            : form.mode === "new" && !form.universalGrammar
              ? "Universal grammar is missing."
              : entryWindowError(form)
                ?? entryOrderError(form)
                ?? perturbationError(form);
  const canStart = !startBlocker;

  return (
    <div className="discover-layout">
      <div className="discover-dashboard">
        <div className="discover-tabs" role="tablist" aria-label="Discover dashboard">
          <button
            type="button"
            className={discoverTab === "progress" ? "active" : ""}
            onClick={() => setDiscoverTab("progress")}
            role="tab"
            aria-selected={discoverTab === "progress"}
          >
            Live progress
          </button>
          <button
            type="button"
            className={discoverTab === "settings" ? "active" : ""}
            onClick={() => setDiscoverTab("settings")}
            role="tab"
            aria-selected={discoverTab === "settings"}
          >
            Configure
          </button>
          <button
            type="button"
            className={discoverTab === "results" ? "active" : ""}
            onClick={() => setDiscoverTab("results")}
            role="tab"
            aria-selected={discoverTab === "results"}
          >
            Results
          </button>
        </div>

      {discoverTab === "settings" && (
      <div className="discover-main discover-settings-pane">
      <section className={`panel discover-form ${expertMode ? "expert-mode" : "simple-mode"}`}>
        <div className="panel-heading">
          <div><p className="eyebrow">Discover</p><h2>{expertMode ? "Configure deterministic evolution" : "Choose a research recipe"}</h2></div>
          <div className="mode-toggle">
            <button
              type="button"
              className={expertMode ? "active" : ""}
              disabled={active || busy}
              onClick={() => setExpertMode((current) => !current)}
              title="Show worker, gate, robustness and search-range controls"
            >
              {expertMode ? "Hide expert settings" : "Expert settings"}
            </button>
            <button type="button" className={form.mode === "new" ? "active" : ""} disabled={active} onClick={() => update("mode", "new")}>New</button>
            <button type="button" className={form.mode === "continue" ? "active" : ""} disabled={active} onClick={() => update("mode", "continue")}>Continue</button>
            <button
              type="button"
              className="primary"
              disabled={!canStart}
              title={startBlocker ?? "Start Discover"}
              onClick={() => void start()}
            >
              {busy ? "Starting…" : form.mode === "new" ? "Start Discover" : "Continue"}
            </button>
          </div>
        </div>
        {startBlocker && !busy && (
          <p className="field-error" style={{ margin: "0 16px 8px" }}>{startBlocker}</p>
        )}
        <fieldset disabled={active || busy}>
          {form.mode === "new" ? (
            <>
              <details className="advanced-settings" open>
                <summary>Discover profile — save this whole new-job setup</summary>
                <p className="immutable-note">Profiles are saved locally and restore the symbol paths, timeframe, gates, costs, robustness and search ranges. A new databank location is always required, so an old archive cannot be overwritten by accident.</p>
                <div className="form-stack compact">
                  <label className="field-row"><span>Saved Discover profile</span><select value={selectedDiscoverProfileId} onChange={(event) => { const profile = discoverProfiles.find((item) => item.id === event.target.value); if (profile) applyDiscoverProfile(profile); else setSelectedDiscoverProfileId(""); }}><option value="">Current unsaved form</option>{discoverProfiles.map((profile) => <option key={profile.id} value={profile.id}>{profile.name}</option>)}</select><small>{selectedDiscoverProfileId ? "Loaded profile is applied. Save to update it." : "Save this form once, then select it here next time."}</small></label>
                  <label className="field-row"><span>Profile name</span><input value={discoverProfileName} onChange={(event) => setDiscoverProfileName(event.target.value)} placeholder="e.g. AUDUSD H1 conservative" /></label>
                </div>
                <div className="form-footer"><p>{discoverProfileBusy ? "Saving profile…" : selectedDiscoverProfileId ? "This profile can be updated without affecting any existing databank." : "New profiles start from the form currently on screen."}</p><div className="button-row"><button type="button" className="secondary" disabled={discoverProfileBusy} onClick={() => { setSelectedDiscoverProfileId(""); setDiscoverProfileName("My Discover profile"); }}>Save as new</button><button type="button" className="primary" disabled={discoverProfileBusy || !discoverProfileName.trim()} onClick={() => void saveCurrentDiscoverProfile()}>{discoverProfileBusy ? "Saving…" : selectedDiscoverProfileId ? "Update profile" : "Save profile"}</button>{selectedDiscoverProfileId && <button type="button" className="danger" disabled={discoverProfileBusy} onClick={() => void removeCurrentDiscoverProfile()}>Delete</button>}</div></div>
              </details>
              <div className="hypothesis-strip">
                <div className="hypothesis-heading">
                  <p className="eyebrow">Hypothesis</p>
                  <h3>{expertMode ? "Universal grammar + run mode" : "Research recipe"}</h3>
                </div>
                <p className="recipe-summary">
                  {expertMode
                    ? "Family-free search: mirrored entry AND blocks and side-specific exit OR blocks with closed-bar shifts. Entry conditions 2 by default (3–4 remain available). Exit conditions 1–3."
                    : "Start with a bounded research path. QuantForge keeps the grammar, gates and validation firewall on safe defaults; choose how much of the pipeline to run below."}
                </p>
                {expertMode && <div className="form-grid universal-grammar-grid">
                  <NumberField label="Entry conditions min" value={form.universalGrammar?.minimumEntryConditions ?? 2} onChange={(value) => updateGrammar("minimumEntryConditions", value ?? 2)} min={2} max={4} />
                  <NumberField label="Entry conditions max" value={form.universalGrammar?.maximumEntryConditions ?? 2} onChange={(value) => updateGrammar("maximumEntryConditions", value ?? 2)} min={2} max={4} />
                  <NumberField label="Exit conditions min" value={form.universalGrammar?.minimumExitConditions ?? 1} onChange={(value) => updateGrammar("minimumExitConditions", value ?? 1)} min={1} max={3} />
                  <NumberField label="Exit conditions max" value={form.universalGrammar?.maximumExitConditions ?? 3} onChange={(value) => updateGrammar("maximumExitConditions", value ?? 3)} min={1} max={3} />
                  <NumberField label="Closed-bar shift min" value={form.universalGrammar?.minimumShift ?? 1} onChange={(value) => updateGrammar("minimumShift", value ?? 1)} min={1} />
                  <NumberField label="Closed-bar shift max" value={form.universalGrammar?.maximumShift ?? 3} onChange={(value) => updateGrammar("maximumShift", value ?? 3)} min={1} />
                </div>}
                <div className="mode-toggle run-mode-toggle">
                  <button
                    type="button"
                    className={form.runMode === "fast_scout" ? "active" : ""}
                    onClick={() =>
                      setForm((current) => ({
                        ...current,
                        runMode: "fast_scout",
                        mutateAfterElites: 20,
                        earlyStopPotElites: 8,
                        targetDatabankElites: null,
                      }))
                    }
                  >
                    Fast Scout
                  </button>
                  <button
                    type="button"
                    className={form.runMode === "full_harvest" ? "active" : ""}
                    onClick={() =>
                      setForm((current) => ({
                        ...current,
                        runMode: "full_harvest",
                        mutateAfterElites: 300,
                        earlyStopPotElites: null,
                        targetDatabankElites: null,
                        robustnessMonteCarloTrials: 1000,
                        robustnessNeighborhoodSamples: 200,
                        minimumNeighborhoodSurvivalFraction: 0.55,
                        requireM1Robustness: true,
                        buildToHolding: true,
                      }))
                    }
                  >
                    Full Harvest
                  </button>
                  <button
                    type="button"
                    className={form.runMode === "quota_harvest" ? "active" : ""}
                    onClick={() =>
                      setForm((current) => ({
                        ...current,
                        runMode: "quota_harvest",
                        mutateAfterElites: 25,
                        earlyStopPotElites: null,
                        targetDatabankElites: 400,
                        initialCandidates: Math.max(current.initialCandidates ?? 500, 1000),
                        batchSize: Math.max(current.batchSize ?? 200, 300),
                        randomFillFraction: Math.max(current.randomFillFraction ?? 0.75, 0.75),
                        robustnessMonteCarloTrials: 1000,
                        robustnessNeighborhoodSamples: 200,
                        minimumNeighborhoodSurvivalFraction: 0.55,
                        minimumDevelopmentExpectancyR: 0.25,
                        requireM1Robustness: true,
                        buildToHolding: true,
                        requireM1Precision: true,
                        simpleExits: true,
                        multiSymbolMinimumPass: 0,
                        factoryAfterDiscover: false,
                        factoryQueueLimit: current.factoryQueueLimit ?? 0,
                        factoryTargetDatabank: current.factoryTargetDatabank ?? 0,
                        factoryMaxCorrelation: current.factoryMaxCorrelation ?? 0.5,
                      }))
                    }
                  >
                    Quota (grow Holding overnight)
                  </button>
                  {expertMode && <button
                    type="button"
                    className={form.runMode === "high_performance_islands" ? "active" : ""}
                    onClick={() =>
                      setForm((current) => ({
                        ...current,
                        runMode: "high_performance_islands",
                        mutateAfterElites: 300,
                        earlyStopPotElites: null,
                        targetDatabankElites: null,
                        initialCandidates: Math.max(current.initialCandidates ?? 500, 2000),
                        batchSize: Math.max(current.batchSize ?? 200, 500),
                        requireM1Robustness: true,
                        requireM1Precision: true,
                        buildToHolding: true,
                        generalIslandCount: (current.generalIslandCount ?? 0) > 0 ? current.generalIslandCount : 4,
                        refinementIslandCount: 0,
                        explorationIslandCount: 0,
                        migrationInterval: current.migrationInterval ?? 10,
                        migrationElites: current.migrationElites ?? 2,
                      }))
                    }
                  >
                    High-Performance Islands
                  </button>}
                </div>
                {form.runMode === "quota_harvest" && (
                  <div className="form-stack compact">
                    <p className="recipe-summary">
                      Grows Holding overnight: H1 scout on the pot, then a cheap side queue (M1 80/130 only). Fold-R, plateau, CPCV, and Monte Carlo wait for the Holding tab. Stops at {formatNumber(form.targetDatabankElites ?? 400)} Holding names. Sealed holdout is unused to pick.
                    </p>
                    <label className="field-row">
                      <span>Overnight factory</span>
                      <input
                        type="checkbox"
                        checked={form.factoryAfterDiscover === true}
                        disabled={active || busy}
                        onChange={(event) => setForm((current) => ({ ...current, factoryAfterDiscover: event.target.checked }))}
                      />
                      <small>
                        {form.runUntilStopped ?? true
                          ? "Open-ended mode keeps Discover running until you press Stop. Then it ranks and tests every frozen Holding strategy; it has no Databank cap."
                          : `When Discover completes, rank the current Holding cohort by trades × R-expectancy, then run the battery ${(form.factoryQueueLimit ?? 0) > 0 ? `on the top ${formatNumber(form.factoryQueueLimit ?? 0)}` : "on everyone in Holding"}. Each pass moves to Databank immediately; failures stay in Holding. Correlation shrinking remains a separate manual action. ${(form.factoryTargetDatabank ?? 0) > 0 ? `Stop once Databank has ${formatNumber(form.factoryTargetDatabank ?? 0)} names.` : "Keep every passer — do not stop at a Databank count."}`}
                      </small>
                    </label>
                    <div className="form-grid universal-grammar-grid">
                      <NumberField label="Holding quota" value={form.targetDatabankElites ?? 400} onChange={(value) => setForm((current) => ({ ...current, targetDatabankElites: value ?? 400 }))} min={40} max={10000} />
                      <NumberField label="Factory queue (0 = all)" value={form.factoryQueueLimit ?? 0} onChange={(value) => setForm((current) => ({ ...current, factoryQueueLimit: value ?? 0 }))} min={0} max={10000} />
                      {!(form.runUntilStopped ?? true) && <NumberField label="Databank target (0 = all passers)" value={form.factoryTargetDatabank ?? 0} onChange={(value) => setForm((current) => ({ ...current, factoryTargetDatabank: value ?? 0 }))} min={0} max={10000} />}
                    </div>
                  </div>
                )}
                {form.runMode === "high_performance_islands" && (
                  <div className="form-stack compact">
                    <p className="recipe-summary">
                      Four islands, large batches, no quota stop. Scout breeds; side workers admit M1 80/130 into Holding. Run fold-R, plateau, CPCV, and Monte Carlo from the Holding tab when you want Databank names.
                    </p>
                    <div className="form-grid universal-grammar-grid">
                      <NumberField label="General islands" value={form.generalIslandCount ?? 4} onChange={(value) => setForm((current) => ({ ...current, generalIslandCount: value ?? 4 }))} min={1} max={32} />
                      <NumberField label="Migration interval" value={form.migrationInterval ?? 10} onChange={(value) => setForm((current) => ({ ...current, migrationInterval: value ?? 10 }))} min={1} />
                      <NumberField label="Migration elites" value={form.migrationElites ?? 2} onChange={(value) => setForm((current) => ({ ...current, migrationElites: value ?? 2 }))} min={0} max={16} />
                    </div>
                  </div>
                )}
              </div>
            </>
          ) : (
            <p className="immutable-note">
              Continuation loads search grammar, run mode, seed, gates and cost model from the
              verified databank. Only the generation count can change.
            </p>
          )}
          <div className="form-stack compact">
            <SymbolSelect
              disabled={active || busy}
              onError={onError}
              onSelect={(symbol) => {
                const defaults = discoverDefaultsForSymbol(symbol.symbol);
                setForm((current) => ({
                  ...current,
                  selectedSymbol: symbol.symbol,
                  dataPath: symbol.dataPath,
                  metadataPath: symbol.metadataPath,
                  sourceTimezone: null,
                  m1DataPath: symbol.m1DataPath,
                  m1MetadataPath: symbol.m1MetadataPath,
                  m1SourceTimezone: null,
                  brokerPath: symbol.brokerPath,
                  commissionPerLotRoundTurn: defaults.commissionPerLotRoundTurn,
                  multiSymbolMinimumPass: defaults.multiSymbolMinimumPass,
                  packDataDir:
                    defaults.packDataDir === undefined ? current.packDataDir : defaults.packDataDir,
                  // New runs automatically receive a unique immutable path at
                  // Start. Never carry the previous archive into a fresh run.
                  databankPath: current.mode === "new" ? "" : current.databankPath || symbol.defaultDatabankPath,
                }));
              }}
              selectedSymbol={form.selectedSymbol}
              selectedDataPath={form.dataPath}
            />
            <PathField
              label={form.mode === "new" ? "New databank (auto if blank)" : "Existing databank"}
              path={form.databankPath}
              choose={chooseOutput}
              onChange={(value) => update("databankPath", value)}
              required={form.mode !== "new"}
            />
            {form.mode === "new" && !form.databankPath && (
              <p className="immutable-note">QuantForge will create a unique archive under <code>Documents/QuantForge/runs/SYMBOL/Databank</code> when you start. Choose a path only if you want a specific location.</p>
            )}
            <div className="field-row">
              <span>Decision timeframe <small>required</small></span>
              <div className="mode-toggle">
                <button type="button" className={form.decisionTimeframe === "H1" ? "active" : ""} onClick={() => update("decisionTimeframe", "H1")}>H1</button>
                <button type="button" className={form.decisionTimeframe === "M15" ? "active" : ""} onClick={() => update("decisionTimeframe", "M15")}>M15</button>
                <button type="button" className={form.decisionTimeframe === "H4" ? "active" : ""} onClick={() => update("decisionTimeframe", "H4")}>H4</button>
              </div>
            </div>
            <div className="field-row">
              <span>History start <small>required</small></span>
              <div className="mode-toggle">
                <button
                  type="button"
                  disabled={active || busy || form.mode !== "new"}
                  className={(form.historyStartYear ?? 2016) === 2016 ? "active" : ""}
                  onClick={() => update("historyStartYear", 2016)}
                >
                  2016
                </button>
                <button
                  type="button"
                  disabled={active || busy || form.mode !== "new"}
                  className={form.historyStartYear === 2020 ? "active" : ""}
                  onClick={() => update("historyStartYear", 2020)}
                >
                  2020
                </button>
              </div>
            </div>
            <p className="immutable-note">
              {form.historyStartYear === 2020
                ? "Drops bars before 1 Jan 2020 broker time from the 2016 pack. IS and sealed OOS fractions are taken on the remaining history."
                : "Uses the full ICMarkets 2016–present pack. Continue runs keep the year sealed in the databank."}
            </p>
            <p className="immutable-note">M15 and H4 are built directly from the selected M1 export, keeping decision candles aligned with execution chronology. Orders are still checked against M1 for fidelity.</p>
            <details className="advanced-settings">
              <summary>Bound paths (auto-filled from symbol)</summary>
              <div className="form-stack compact">
                <label className="field-row"><span>Decision source ({form.decisionTimeframe})</span><code>{form.decisionTimeframe === "H1" ? form.dataPath || "—" : "Built from the M1 path below"}</code></label>
                <label className="field-row"><span>Decision metadata</span><code>{form.metadataPath || "—"}</code></label>
                <label className="field-row"><span>M1</span><code>{form.m1DataPath || "—"}</code></label>
                <label className="field-row"><span>M1 metadata</span><code>{form.m1MetadataPath || "—"}</code></label>
                <label className="field-row"><span>Quote sidecar</span><code>{form.m1DataPath ? (form.m1DataPath.replace(/\.(tsv|csv)$/i, "") + ".quotes.csv") : "—"}</code></label>
                <label className="field-row"><span>Broker</span><code>{form.brokerPath || "—"}</code></label>
              </div>
            </details>
          </div>
          <div className="numeric-grid core-numbers">
            <label className="check-field discover-split" style={{ gridColumn: "1 / -1" }}>
              <input
                type="checkbox"
                checked={form.runUntilStopped ?? true}
                onChange={(event) => setForm((current) => ({
                  ...current,
                  runUntilStopped: event.target.checked,
                  factoryTargetDatabank: event.target.checked ? 0 : current.factoryTargetDatabank,
                }))}
              />
              <span>Run until stopped (SQX-style continuous bank growth)</span>
            </label>
            <NumberField
              label={form.runUntilStopped ? "Soft generation budget (0 = none)" : "Generations"}
              value={form.generations}
              onChange={(value) => update("generations", value ?? 0)}
              min={0}
            />
            {expertMode && form.mode === "new" && <>
              <NumberField label="Initial candidates" value={form.initialCandidates} onChange={(value) => update("initialCandidates", value)} min={1} />
              <NumberField label="Batch / generation" value={form.batchSize} onChange={(value) => update("batchSize", value)} min={1} />
              <NumberField label="Seed" value={form.seed} onChange={(value) => update("seed", value)} min={0} />
              <NumberField label="Scout worker threads (0 = auto)" value={form.workerThreads} onChange={(value) => update("workerThreads", value)} min={0} />
              <NumberField label="Promotion workers (0 = auto 2–4)" value={form.promotionWorkerThreads} onChange={(value) => update("promotionWorkerThreads", value)} min={0} />
              <NumberField label="Promotion queue capacity" value={form.promotionQueueCapacity} onChange={(value) => update("promotionQueueCapacity", value)} min={1} />
              <NumberField label="Maximum RAM (MB)" value={form.maxMemoryMb} onChange={(value) => update("maxMemoryMb", value)} min={1024} step={1024} />
              <NumberField label="Breed after pot elites" value={form.mutateAfterElites} onChange={(value) => update("mutateAfterElites", value)} min={0} />
              <NumberField label="Random fill fraction" value={form.randomFillFraction} onChange={(value) => update("randomFillFraction", value)} step={0.05} min={0} />
              <NumberField label="OOS1 reserve (0 = off)" value={form.validationFraction} onChange={(value) => update("validationFraction", value)} step={0.05} min={0} />
              <NumberField label="Sealed holdout" value={form.sealedFraction} onChange={(value) => update("sealedFraction", value)} step={0.05} />
              <label className="field-row pack-directory-field">
                <span>FX pack directory (matching-timeframe screen)</span>
                <input
                  value={form.packDataDir ?? ""}
                  onChange={(event) => update("packDataDir", event.target.value || null)}
                  placeholder="ICMarkets_EST7_2020_present"
                />
              </label>
            </>}
          </div>
          {form.mode === "new" ? (
            <>
              <details className="advanced-settings" open>
                <summary>Scout gates — random search screen</summary>
                <p className="immutable-note">Cheap Selected-TF filter. Keep these loose so the databank pot can fill before breeding.</p>
                <div className="numeric-grid">
                  <NumberField label="Minimum trades" value={form.minimumTrades} onChange={(value) => update("minimumTrades", value)} min={0} />
                  <NumberField label="Maximum drawdown %" value={form.maximumDrawdownPercent} onChange={(value) => update("maximumDrawdownPercent", value)} step={0.1} />
                  <NumberField label="Minimum return %" value={form.minimumReturnPercent} onChange={(value) => update("minimumReturnPercent", value)} step={0.1} />
                  <NumberField label="Minimum profit factor" value={form.minimumProfitFactor} onChange={(value) => update("minimumProfitFactor", value)} step={0.01} />
                  <NumberField label="Minimum recovery factor" value={form.minimumReturnDrawdown} onChange={(value) => update("minimumReturnDrawdown", value)} min={0} step={0.05} />
                </div>
              </details>
              <details className="advanced-settings" open>
                <summary>Deposit gates — enter the initial pot</summary>
                <p className="immutable-note">Selected-TF metrics to join the breeding pot. Nothing enters the databank yet — that waits until breeding unlocks.</p>
                <div className="numeric-grid">
                  <NumberField label="Minimum trades" value={form.depositMinimumTrades} onChange={(value) => update("depositMinimumTrades", value)} min={0} />
                  <NumberField label="Maximum drawdown %" value={form.depositMaximumDrawdownPercent} onChange={(value) => update("depositMaximumDrawdownPercent", value)} step={0.1} />
                  <NumberField label="Minimum return %" value={form.depositMinimumReturnPercent} onChange={(value) => update("depositMinimumReturnPercent", value)} step={0.1} />
                  <NumberField label="Minimum profit factor" value={form.depositMinimumProfitFactor} onChange={(value) => update("depositMinimumProfitFactor", value)} step={0.01} />
                  <NumberField label="Minimum recovery factor" value={form.depositMinimumReturnDrawdown} onChange={(value) => update("depositMinimumReturnDrawdown", value)} min={0} step={0.05} />
                </div>
              </details>
              <details className="advanced-settings" open>
                <summary>Search profile</summary>
                <p className="immutable-note">Pick a built-in gene-space preset or a saved Settings profile. The chosen ranges are sealed into this new databank; continuation cannot change them.</p>
                <BuiltInSearchPresetPicker
                  value={form.searchRanges ?? DEFAULT_SEARCH_RANGES}
                  onApply={(preset) => {
                    setForm((current) => ({
                      ...current,
                      searchRanges: { ...preset.ranges },
                      universalGrammar: {
                        ...(current.universalGrammar ?? DEFAULT_UNIVERSAL_GRAMMAR),
                        maximumShift: preset.maximumShift,
                      },
                    }));
                  }}
                />
                <SavedRangeProfilePicker value={form.searchRanges ?? DEFAULT_SEARCH_RANGES} onChange={(searchRanges) => update("searchRanges", searchRanges)} onError={onError} />
              </details>
              <details className="advanced-settings" open>
                <summary>Holding admission — cheap path after breeding</summary>
                <p className="immutable-note">Discover deposits to Holding after H1 gates + M1 80/130 only. Fold-R, permutation (Ret/DD 0.85–1.25, 55% neighbours), CPCV, and Monte Carlo wait until you run the battery from the Holding tab.</p>
                <label className="check-field discover-split"><input type="checkbox" checked={form.buildToHolding ?? true} onChange={(event) => update("buildToHolding", event.target.checked)} /><span>Build to Holding (recommended). Uncheck only to send the full battery during Discover</span></label>
                <label className="check-field discover-split"><input type="checkbox" checked={form.requireM1Robustness ?? true} onChange={(event) => update("requireM1Robustness", event.target.checked)} /><span>Keep battery settings for later Holding → Databank (CPCV / Monte Carlo / param plateau)</span></label>
                <label className="check-field discover-split"><input type="checkbox" checked={form.requireM1Precision ?? true} onChange={(event) => update("requireM1Precision", event.target.checked)} /><span>M1 80% trades / 130% DD retention vs Selected-TF (required)</span></label>
                <label className="check-field discover-split"><input type="checkbox" checked={form.calendarYearFolds ?? false} onChange={(event) => update("calendarYearFolds", event.target.checked)} /><span>Strict calendar-year folds (every IS year must pass)</span></label>
                <div className="numeric-grid">
                  <NumberField label="Minimum Development expectancy (R)" value={form.minimumDevelopmentExpectancyR} onChange={(value) => update("minimumDevelopmentExpectancyR", value)} min={0} step={0.05} />
                  <NumberField label="Legacy fold setting" value={form.robustnessFolds} onChange={(value) => update("robustnessFolds", value)} min={2} />
                  <NumberField label="MC trials" value={form.robustnessMonteCarloTrials} onChange={(value) => update("robustnessMonteCarloTrials", value)} min={1} />
                  <NumberField label="MC block length" value={form.robustnessMonteCarloBlockLength} onChange={(value) => update("robustnessMonteCarloBlockLength", value)} min={1} />
                  <NumberField
                    label="MC P80 retention (0–1)"
                    value={form.robustnessMonteCarloP80ProfitRetention}
                    onChange={(value) => update("robustnessMonteCarloP80ProfitRetention", value)}
                    min={0}
                    max={1}
                    step={0.05}
                  />
                  <NumberField
                    label="MC skip-trade prob"
                    value={form.robustnessMonteCarloSkipTradeProbability}
                    onChange={(value) => update("robustnessMonteCarloSkipTradeProbability", value)}
                    min={0}
                    max={0.99}
                    step={0.01}
                  />
                  <NumberField
                    label="MC max DD ratio"
                    value={form.robustnessMonteCarloMaxDrawdownRatio}
                    onChange={(value) => update("robustnessMonteCarloMaxDrawdownRatio", value)}
                    min={1}
                    step={0.05}
                  />
                  <NumberField label="Param samples" value={form.robustnessNeighborhoodSamples} onChange={(value) => update("robustnessNeighborhoodSamples", value)} min={1} />
                  <NumberField
                    label="Param jitter ±% (SQX default 20)"
                    value={perturbationPercent(form)}
                    onChange={(value) => update("robustnessPerturbationFraction", value === null ? null : Math.min(100, Math.max(1, value)) / 100)}
                    min={1}
                    max={100}
                    step={1}
                  />
                  <NumberField label="Optional cross-symbol min pass (0=off)" value={form.multiSymbolMinimumPass} onChange={(value) => update("multiSymbolMinimumPass", value)} min={0} />
                </div>
                <p className="immutable-note">Monte Carlo settings seal into the databank with the rest of Discover config. Stop/limit search requires a bid/ask <code>.quotes.csv</code> sidecar beside M1.</p>
                {perturbationError(form) && <p className="field-error">{perturbationError(form)}</p>}
              </details>
              <details className="advanced-settings">
                <summary>Diversity and cost model</summary>
                <div className="numeric-grid">
                  <NumberField label="Correlation ceiling" value={form.correlationThreshold} onChange={(value) => update("correlationThreshold", value)} step={0.01} />
                  <NumberField label="Novelty weight" value={form.noveltyWeight} onChange={(value) => update("noveltyWeight", value)} step={0.1} />
                  <NumberField label="M1 fidelity min return retention" value={form.minimumM1ReturnRetention} onChange={(value) => update("minimumM1ReturnRetention", value)} step={0.01} />
                  <NumberField label="OOS1 certification retention" value={form.oos1ExpectancyRetention} onChange={(value) => update("oos1ExpectancyRetention", value)} step={0.05} />
                  <NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value)} step={0.01} />
                  <NumberField label="Slippage points / side" value={form.slippagePointsPerSide} onChange={(value) => update("slippagePointsPerSide", value)} step={0.1} />
                  <NumberField label="Fallback spread points" value={form.fallbackSpreadPoints} onChange={(value) => update("fallbackSpreadPoints", value)} step={0.1} optional />
                  <NumberField label="Maximum spread points" value={form.maxSpreadPoints} onChange={(value) => update("maxSpreadPoints", value)} step={0.1} optional />
                  <NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value)} min={1} />
                </div>
              </details>
            </>
          ) : null}
          {form.mode === "new" && <>
            <p className="immutable-note simple-mode-note">Development drives search and breeding. Holding receives the cheap M1 fidelity screen; the full robustness battery runs later from the Holding tab. Sealed holdout is never loaded by Discover. Turn on Expert settings to tune gates, robustness, execution modules and search ranges.</p>
            <details className="advanced-settings" open>
              <summary>Execution modules — search genes (pot only until breeding)</summary>
              <p className="immutable-note">Disabled is the high-parity baseline. Enabling a module widens the H1 search pot. Databank admission still requires the post-breed M1 pipeline.</p>
              <label className="check-field discover-split"><input type="checkbox" checked={form.allowBreakEven ?? false} onChange={(event) => setForm((current) => ({ ...current, allowBreakEven: event.target.checked, simpleExits: event.target.checked ? false : current.simpleExits }))} /><span>Break-even stop move</span></label>
              <label className="check-field discover-split"><input type="checkbox" checked={form.allowTrailingStops ?? false} onChange={(event) => setForm((current) => ({ ...current, allowTrailingStops: event.target.checked, simpleExits: event.target.checked ? false : current.simpleExits }))} /><span>Trailing stop</span></label>
              <label className="check-field discover-split"><input type="checkbox" checked={form.allowPartialExits ?? false} onChange={(event) => setForm((current) => ({ ...current, allowPartialExits: event.target.checked, simpleExits: event.target.checked ? false : current.simpleExits }))} /><span>Partial exit</span></label>
            </details>
            <details className="advanced-settings" open>
              <summary>Entry order types — sampled in equal shares</summary>
              <p className="immutable-note">{entryOrderSummary(form)}</p>
              <label className="check-field discover-split"><input type="checkbox" checked={form.allowMarketEntries ?? true} onChange={(event) => setForm((current) => ({ ...current, allowMarketEntries: event.target.checked, simpleExits: event.target.checked ? current.simpleExits : false }))} /><span>Market entry at bar open <small>(highest H1↔M1 agreement)</small></span></label>
              <label className="check-field discover-split"><input type="checkbox" checked={form.allowStopEntries ?? false} onChange={(event) => setForm((current) => ({ ...current, allowStopEntries: event.target.checked, simpleExits: event.target.checked ? false : current.simpleExits }))} /><span>Buy-stop / sell-stop entry <small>(requires bid/ask quote sidecar for Discover + Judge)</small></span></label>
              <label className="check-field discover-split"><input type="checkbox" checked={form.allowLimitEntries ?? false} onChange={(event) => setForm((current) => ({ ...current, allowLimitEntries: event.target.checked, simpleExits: event.target.checked ? false : current.simpleExits }))} /><span>Buy-limit / sell-limit entry <small>(requires bid/ask quote sidecar for Discover + Judge)</small></span></label>
              {entryOrderError(form) && <p className="field-error">{entryOrderError(form)}</p>}
            </details>
            <label className="check-field discover-split"><input type="checkbox" checked={form.maxOneEntryPerDay ?? true} onChange={(event) => update("maxOneEntryPerDay", event.target.checked)} /><span>Max one fill/day (first market or pending fill locks the day)</span></label>
            <div className="discover-split">
              <NumberField
                label="Entry window start hour (broker local, inclusive)"
                value={form.entryWindowStartHour ?? 2}
                onChange={(value) => update("entryWindowStartHour", Math.min(23, Math.max(0, value ?? 2)))}
                min={0}
              />
              <NumberField
                label="Entry window end hour (broker local, exclusive)"
                value={form.entryWindowEndHour ?? 19}
                onChange={(value) => update("entryWindowEndHour", Math.min(24, Math.max(1, value ?? 19)))}
                min={1}
              />
            </div>
            <p className="immutable-note">{entryWindowSummary(form)} Broker local time, so it follows the symbol profile's timezone rather than UTC. Widen the start hour when your broker only accepts orders a few minutes after midnight; the exported expert and every M1 replay use the same window.</p>
            <label className="check-field discover-split"><input type="checkbox" checked={form.flattenAt22 ?? false} onChange={(event) => update("flattenAt22", event.target.checked)} /><span>Exit at end of day (SQX ExitAtEndOfDay)</span></label>
            {(form.flattenAt22 ?? false) && (
              <NumberField
                label="EOD exit hour (broker local, 0–23)"
                value={form.endOfDayHour ?? 23}
                onChange={(value) => update("endOfDayHour", Math.min(23, Math.max(0, value ?? 23)))}
                min={0}
              />
            )}
          </>}
        </fieldset>
        <div className="form-footer discover-start-footer">
          <p>
            {startBlocker
              ? startBlocker
              : "Pipeline: Development gates → reservoir → breed → Holding (M1 80/130) → on-demand battery → Databank. OOS2 remains sealed."}
          </p>
          <button
            type="button"
            className="primary"
            disabled={!canStart}
            title={startBlocker ?? "Start Discover"}
            onClick={() => void start()}
          >
            {busy ? "Starting…" : form.mode === "new" ? "Start Discover" : "Continue optimization"}
          </button>
        </div>
      </section>
      <DiscoverContractSummary form={form} active={active} />
      </div>
      )}

      {discoverTab === "progress" && (
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
              label="Holding"
              value={
                job?.targetDatabankElites
                  ? `${formatNumber((job?.holdingElites ?? 0) + (job?.databankElites ?? 0))}/${job.targetDatabankElites}`
                  : formatNumber(job?.holdingElites ?? 0)
              }
              note={
                job?.targetDatabankElites
                  ? "Quota · M1 80/130"
                  : job?.breedingActive
                    ? "Post-breed M1 80/130"
                    : "Empty until breeding unlocks"
              }
            />
            <Kpi
              label="Databank"
              value={formatNumber(job?.databankElites ?? job?.coverage ?? 0)}
              note="After Holding battery"
            />
            <Kpi label="Pot admissions" value={formatNumber(job?.potNewNiches ?? 0)} note={job?.breedingActive ? "Queued for Holding pipeline" : "Breeding stock only"} />
            <Kpi label="Rejected" value={formatNumber(rejected)} note="Did not stay" />
            <Kpi
              label="Evals / hour · rolling"
              value={formatNumber(job?.rollingEvaluationsPerHour ?? 0, 0)}
              note={[
                `${formatNumber(job?.lifetimeEvaluationsPerHour ?? job?.evaluationsPerHour ?? 0, 0)} lifetime avg`,
                `${formatNumber(job?.acceptsPerHour ?? 0, 1)} accepts/hr`,
                `${job?.workerThreads ?? "—"} scout`,
                job?.breedingActive ? `${job?.promotionWorkerThreads ?? "—"} promo` : null,
              ].filter(Boolean).join(" · ")}
            />
            <Kpi
              label="Promo queue"
              value={`${formatNumber(job?.promotionQueueDepth ?? 0)}/${formatNumber(job?.promotionQueueCapacity ?? 64)}`}
              note={
                job?.breedingActive
                  ? `${formatNumber(job?.promotionInflight ?? 0)} in flight · ${formatNumber(job?.promotionsPerHour ?? 0, 0)} promo/hr`
                  : "Idle until breeding unlocks"
              }
            />
            <Kpi label="QD score" value={formatNumber(job?.qdScore ?? 0, 2)} note="Databank evidence mass" />
          </div>
          <div className="reject-funnel">
            <p className="eyebrow">Reject reasons</p>
            <p className="funnel-group-label">H1 pot gates — before breeding</p>
            <ul>
              <li><span>Scout gate</span><strong>{formatNumber(job?.rejectedGate ?? 0)}</strong></li>
              <li><span>Deposit gate</span><strong>{formatNumber(job?.rejectedDepositGate ?? 0)}</strong></li>
              <li><span>Ambiguous same-bar</span><strong>{formatNumber(job?.rejectedAmbiguous ?? 0)}</strong></li>
              <li><span>Clone</span><strong>{formatNumber(job?.rejectedClone ?? 0)}</strong></li>
              <li><span>Correlation</span><strong>{formatNumber(job?.rejectedCorrelated ?? 0)}</strong></li>
              <li><span>Eval error</span><strong>{formatNumber(job?.rejectedEvaluation ?? 0)}</strong></li>
            </ul>
            <p className="funnel-group-label">Post-breed Holding pipeline — after breeding unlocks</p>
            <ul>
              <li><span>OOS1 leakage guard (must remain 0)</span><strong>{formatNumber(job?.rejectedOos1 ?? 0)}</strong></li>
              <li><span>Development expectancy floor</span><strong>{formatNumber(job?.rejectedDevelopmentExpectancy ?? 0)}</strong></li>
              <li><span>M1 fidelity</span><strong>{formatNumber(job?.rejectedM1Fidelity ?? 0)}</strong></li>
              <li><span>Development CPCV</span><strong>{formatNumber(job?.rejectedWalkForward ?? 0)}</strong></li>
              <li><span>Monte Carlo</span><strong>{formatNumber(job?.rejectedMonteCarlo ?? 0)}</strong></li>
              <li><span>Param plateau</span><strong>{formatNumber(job?.rejectedParamNeighborhood ?? 0)}</strong></li>
              <li><span>Niche not improved</span><strong>{formatNumber(job?.rejectedNicheNotImproved ?? 0)}</strong></li>
            </ul>
            {job?.bestIsExpectancy != null && (
              <p className="funnel-best">
                Best Development expectancy — {formatNumber(job.bestIsExpectancy, 2)}R. This is research evidence, not OOS certification.
              </p>
            )}
            <p className="funnel-best">
              OOS1 validation — runs only after a candidate passes the complete Development battery; it never feeds breeding or ranking.
            </p>
            <p className="funnel-best">
              OOS2 sealed final — held out from Discover selection. It is assessed only in the one-shot Sealed Final stage; the Databank equity chart may display it but never uses it as a Discover gate.
            </p>
            {(job?.m1BarsRepaired ?? 0) > 0 && (
              <p className="funnel-best">Aligned {formatNumber(job!.m1BarsRepaired)} decision bars to M1 OHLC before search.</p>
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
            <button className="danger" onClick={() => void control(stopDiscover)}>Stop & save final</button>
            <small>Pause and stop take effect at a generation boundary. Elites are written to the databank as soon as they pass.</small>
          </div>}
          {job?.status === "completed" && job.outputPath && <div className="completion-card"><div><span>Checkpoint</span><code>{job.outputPath}</code></div><button className="primary" onClick={() => onOpenDatabank(job.outputPath!)}>Open in Databank</button></div>}
          {job?.status === "completed" && !job.outputPath && <div className="job-empty-note">{job.message}</div>}
          {job?.status === "failed" && <div className="job-error">{job.message}</div>}
        </div>
      </section>
      )}

      {discoverTab === "results" && (
      <div className="results-pane">
        {resultsViewFp ? (
          <ResultsDetailPage
            fingerprint={resultsViewFp}
            workspace={liveWorkspace}
            backLabel="Discover results"
            onBack={() => setResultsViewFp(null)}
            onError={onError}
            onExportIr={(target) => void exportEliteStrategy(target).catch((reason) => onError(String(reason)))}
            onExportMq5={(target) => void exportEliteEas([target], "H1", 42_424_242).catch((reason) => onError(String(reason)))}
            onStage={(target) => void promoteEliteToVault(target).then(() => undefined).catch((reason) => onError(String(reason)))}
            onOpenResult={setResultsViewFp}
          />
        ) : (
          <>
            <section className="panel results-elites-panel">
              <div className="panel-heading">
                <div>
                  <p className="eyebrow">Results</p>
                  <h2>
                    {liveEliteRows.length
                      ? `${liveEliteRows.length} holding + databank`
                      : "No strategies yet"}
                  </h2>
                </div>
                <span className="read-only-badge">click a row for full detail</span>
              </div>
              {liveEliteRows.length > 0 ? (
                <EliteTable
                  batchSelection={new Set()}
                  onDoubleClick={(fingerprint) => setResultsViewFp(fingerprint)}
                  onSelect={(fingerprint) => setResultsViewFp(fingerprint)}
                  onToggle={() => {}}
                  rows={liveEliteRows}
                  selected={resultsViewFp}
                  showBatch={false}
                  showActions
                  onExportIr={(fingerprint) => void exportEliteStrategy(fingerprint).catch((reason) => onError(String(reason)))}
                  onExportMq5={(fingerprint) => void exportEliteEas([fingerprint], "H1", 42_424_242).catch((reason) => onError(String(reason)))}
                  onStage={(fingerprint) => void promoteEliteToVault(fingerprint).then((path) => path && onError(null)).catch((reason) => onError(String(reason)))}
                />
              ) : (
                <p className="cluster-empty">
                  Run Discover until elites pass M1 robustness, or open a databank. Results appear here as promotions land.
                </p>
              )}
            </section>

            <section className="panel family-tester-panel condition-bakeoff-panel">
              <div className="panel-heading">
                <div>
                  <p className="eyebrow">Timeframe comparison</p>
                  <h2>H1 vs H4 bakeoff</h2>
                </div>
                <span className="read-only-badge">Paired candidates · OOS1 diagnostic</span>
              </div>
              <div className="family-tester-body">
                <p className="immutable-note">
                  Evaluates the same deterministic candidates on H1 and H4. Development gates select candidates;
                  OOS1 measures survival. The sealed holdout is never loaded. If the current recipe has OOS1 off,
                  this diagnostic uses a temporary 20% OOS1 reserve.
                </p>
                <div className="numeric-grid family-tester-numbers">
                  <NumberField
                    label="Draws / grammar cell"
                    value={timeframeBakeoffDraws}
                    onChange={(value) => setTimeframeBakeoffDraws(value ?? 20)}
                    min={1}
                  />
                </div>
                <div className="family-tester-actions">
                  <button
                    className="primary"
                    disabled={active || timeframeBakeoffBusy || !form.dataPath || !form.m1DataPath || !form.brokerPath}
                    onClick={() => void runTimeframeComparison()}
                  >
                    {timeframeBakeoffBusy ? "Running H1 vs H4…" : "Run H1 vs H4 bakeoff"}
                  </button>
                </div>
                {timeframeBakeoffReport && (
                  <>
                    <div className="family-tester-results">
                      <table>
                        <thead>
                          <tr>
                            <th>Lane</th>
                            <th>IS screen</th>
                            <th>OOS1 survival</th>
                            <th>Retention</th>
                            <th>Trades</th>
                            <th>DD</th>
                            <th>Selected future lift</th>
                          </tr>
                        </thead>
                        <tbody>
                          {timeframeBakeoffReport.rows.map((row) => (
                            <tr key={row.timeframe}>
                              <td>{row.timeframe}</td>
                              <td>{formatNumber(row.screened)} / {formatNumber(row.draws)}</td>
                              <td>{(row.oos1SurvivalRate * 100).toFixed(1)}%</td>
                              <td>{row.medianRetention == null ? "—" : formatNumber(row.medianRetention, 3)}</td>
                              <td>{row.medianTradeCount == null ? "—" : formatNumber(row.medianTradeCount, 0)}</td>
                              <td>{row.medianDrawdownPercent == null ? "—" : `${formatNumber(row.medianDrawdownPercent, 1)}%`}</td>
                              <td>{row.selectedFutureExpectancyLiftR == null ? "—" : `${formatNumber(row.selectedFutureExpectancyLiftR, 3)}R`}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                    <p className="immutable-note">
                      {timeframeBakeoffReport.pair.recommendation}
                      {" "}
                      H4 pass-rate lift: {(timeframeBakeoffReport.pair.h4PassRateLift * 100).toFixed(1)}pp;
                      paired OOS1 wins H1/H4: {formatNumber(timeframeBakeoffReport.pair.h1Oos1Wins)}/{formatNumber(timeframeBakeoffReport.pair.h4Oos1Wins)}.
                    </p>
                  </>
                )}
              </div>
            </section>

            <section className="panel family-tester-panel condition-bakeoff-panel">
              <div className="panel-heading">
                <div>
                  <p className="eyebrow">Methodology lite</p>
                  <h2>Condition bakeoff</h2>
                </div>
                <span className="read-only-badge">Development only · no OOS access</span>
              </div>
              <div className="family-tester-body">
                <p className="immutable-note">
                  Equal-budget research per entry-condition count using Development only. Expectancy is shown in R
                  ($1,000 fixed risk). It may guide grammar choice, so OOS1 and sealed OOS2 remain unavailable here.
                </p>
                <div className="family-tester-toggles" role="group" aria-label="Entry counts to test">
                  {[2, 3, 4].map((count) => (
                    <label key={count} className="check-field">
                      <input
                        type="checkbox"
                        checked={testerCounts.includes(count)}
                        disabled={active || testerBusy}
                        onChange={() => toggleTesterCount(count)}
                      />
                      <span>{count} entry conditions</span>
                    </label>
                  ))}
                </div>
                <div className="numeric-grid family-tester-numbers">
                  <NumberField
                    label="Generations / count"
                    value={testerGenerations}
                    onChange={(value) => setTesterGenerations(value ?? 3)}
                    min={1}
                  />
                  <NumberField
                    label="Initial candidates"
                    value={testerCandidates}
                    onChange={(value) => setTesterCandidates(value ?? 60)}
                    min={20}
                  />
                  <NumberField
                    label="Seed"
                    value={testerSeed}
                    onChange={(value) => setTesterSeed(value ?? 42)}
                    min={0}
                  />
                </div>
                <div className="family-tester-actions">
                  <button
                    className="primary"
                    disabled={
                      active
                      || testerBusy
                      || testerCounts.length < 1
                      || !form.dataPath
                      || !form.m1DataPath
                      || !form.brokerPath
                    }
                    onClick={() => void runTester()}
                  >
                    {testerBusy ? "Running condition bakeoff…" : "Run condition bakeoff"}
                  </button>
                  {testerReport?.recommended != null && (
                    <button
                      className="secondary"
                      disabled={active || testerBusy}
                      onClick={() => applyRecommendedCount(testerReport.recommended!)}
                    >
                      Use recommended ({testerReport.recommended} entry)
                    </button>
                  )}
                </div>
                {testerReport && (
                  <div className="family-tester-results">
                    <table>
                      <thead>
                        <tr>
                          <th>Entry</th>
                          <th>Development E (R)</th>
                          <th>Elites</th>
                          <th>Pot</th>
                          <th>Evals</th>
                          <th />
                        </tr>
                      </thead>
                      <tbody>
                        {testerReport.rows.map((row) => {
                          const recommended = testerReport.recommended === row.entryConditions;
                          return (
                            <tr key={row.entryConditions} className={recommended ? "recommended" : undefined}>
                              <td>
                                {row.entryConditions}
                                {recommended ? " · recommended" : ""}
                              </td>
                              <td>{formatNumber(row.medianIsExpectancyR, 3)}</td>
                              <td>{formatNumber(row.elites)}</td>
                              <td>{formatNumber(row.potElites)}</td>
                              <td>{formatNumber(row.evaluations)}</td>
                              <td>
                                <button
                                  type="button"
                                  className="secondary"
                                  disabled={active || testerBusy}
                                  onClick={() => applyRecommendedCount(row.entryConditions)}
                                >
                                  Use
                                </button>
                              </td>
                            </tr>
                          );
                        })}
                      </tbody>
                    </table>
                  </div>
                )}
              </div>
            </section>
          </>
        )}
      </div>
      )}
      </div>

      {(active || (job?.holdingElites ?? 0) > 0 || (job?.databankElites ?? 0) > 0) && (
        <section className={`panel discover-live-databank ${liveEliteRows.length ? "" : "empty"}`}>
          <div className="panel-heading">
            <div>
              <p className="eyebrow">Live Holding / Databank</p>
              <h2>
                {liveEliteRows.length
                  ? `${formatNumber(liveEliteRows.length)} strategies`
                  : "Waiting for first Holding deposit"}
              </h2>
            </div>
            <span className="read-only-badge">
              {active ? "search running · inspect without stopping" : "checkpoint loaded"}
            </span>
          </div>
          {liveEliteRows.length > 0 ? (
            <>
              <p className="live-strip-hint">
                Single-click to highlight. Double-click any row for equity curve, partition metrics, and trade list.
                Updates automatically as new elites pass M1 robustness.
              </p>
              <EliteTable
                batchSelection={new Set()}
                compact
                onDoubleClick={(fingerprint) => void inspectLiveElite(fingerprint)}
                onSelect={setLiveSelected}
                onToggle={() => {}}
                rows={liveEliteRows}
                selected={liveSelected}
                showBatch={false}
              />
            </>
          ) : (
            <p>
              {active
                ? "Strategies appear here after they pass the deposit gate, M1 retention, walk-forward, Monte Carlo, and parameter neighborhood checks."
                : "No databank elites in the latest checkpoint."}
            </p>
          )}
        </section>
      )}

      {liveDetailOpen && (
        <EliteDetailModal
          detail={liveDetail}
          loading={liveDetailLoading}
          m1FidelityVerified={liveWorkspace?.m1FidelityVerified ?? true}
          onClose={() => setLiveDetailOpen(false)}
          onError={onError}
          onParity={async () => {
            if (liveDetail) {
              setLiveDetailOpen(false);
              onOpenDatabank(job?.outputPath ?? "");
            }
          }}
          onTab={setLiveInspectorTab}
          researchGrade={liveWorkspace?.researchGrade ?? false}
          tab={liveInspectorTab}
        />
      )}
    </div>
  );
}

function DiscoverContractSummary({
  active,
  form,
}: {
  active: boolean;
  form: DiscoverRequest;
}) {
  const grammar = form.universalGrammar;
  const enabledModules = [
    (form.allowMarketEntries ?? true) ? "Market entries" : null,
    form.allowStopEntries ? "Stop entries" : null,
    form.allowLimitEntries ? "Limit entries" : null,
    form.allowBreakEven ? "Break-even" : null,
    form.allowTrailingStops ? "Trailing stop" : null,
    form.allowPartialExits ? "Partial exits" : null,
    form.flattenAt22 ? `EOD exit ${String(form.endOfDayHour ?? 23).padStart(2, "0")}:00` : null,
    entryWindowSummary(form).replace("Entries allowed ", "Entries "),
    form.maxOneEntryPerDay ? "One fill / day" : null,
  ].filter((value): value is string => Boolean(value));
  const boundSources = [
    ["Decision data", Boolean(form.dataPath)],
    ["M1 chronology", Boolean(form.m1DataPath)],
    ["Broker profile", Boolean(form.brokerPath)],
    ["New databank", Boolean(form.databankPath)],
  ] as const;

  return (
    <aside className="panel discover-contract" aria-label="Discover research contract summary">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Research contract</p>
          <h2>Run summary</h2>
        </div>
        <span className={active ? "contract-state running" : "contract-state"}>
          {active ? "Running" : "Ready to configure"}
        </span>
      </div>

      <div className="contract-section">
        <p className="eyebrow">Hypothesis</p>
        <div className="contract-hero">
          <strong>{form.decisionTimeframe ?? "—"}</strong>
          <span>{form.runMode === "fast_scout" ? "Fast scout" : form.runMode === "quota_harvest" ? "Quota harvest" : form.runMode === "high_performance_islands" ? "High-performance islands" : "Full harvest"}</span>
        </div>
        <SummaryLine label="History start" value={String(form.historyStartYear ?? 2016)} />
        <SummaryLine label="Entry conditions" value={grammar ? `${grammar.minimumEntryConditions}–${grammar.maximumEntryConditions}` : "—"} />
        <SummaryLine label="Exit conditions" value={grammar ? `${grammar.minimumExitConditions}–${grammar.maximumExitConditions}` : "—"} />
        <SummaryLine label="Closed-bar shifts" value={grammar ? `${grammar.minimumShift}–${grammar.maximumShift}` : "—"} />
        <SummaryLine
          label="Parameter ranges"
          value={(() => {
            const preset = detectBuiltInSearchPreset(form.searchRanges ?? DEFAULT_SEARCH_RANGES);
            if (preset === "h1_compact") return "H1 compact · periods 10–20";
            if (preset === "sqx_random") return "SQX random · periods 10–50";
            return "Custom / saved profile";
          })()}
        />
        <SummaryLine label="Fixed risk / trade" value="$1,000" accent />
      </div>

      <div className="contract-section">
        <p className="eyebrow">Search budget</p>
        <SummaryLine label="Initial candidates" value={formatNumber(form.initialCandidates ?? 0)} />
        <SummaryLine label="Batch / generation" value={formatNumber(form.batchSize ?? 0)} />
        <SummaryLine label="Scout workers" value={(form.workerThreads ?? 0) === 0 ? "Auto" : formatNumber(form.workerThreads ?? 0)} />
        <SummaryLine label="Promotion workers" value={(form.promotionWorkerThreads ?? 0) === 0 ? "Auto 2–4" : formatNumber(form.promotionWorkerThreads ?? 0)} />
        <SummaryLine label="Breeding begins" value={`${formatNumber(form.mutateAfterElites ?? 0)} pot elites`} />
        <SummaryLine label="Generation limit" value={form.runUntilStopped ? "Continuous" : formatNumber(form.generations)} />
      </div>

      <div className="contract-section">
        <p className="eyebrow">Validation firewall</p>
        <SummaryLine label="Development / Holdout" value={splitCaption(form.validationFraction ?? 0, form.sealedFraction ?? 1 / 3)} />
        <SummaryLine label="M1 return retention" value={`${formatNumber((form.minimumM1ReturnRetention ?? .9) * 100, 0)}%`} />
        <SummaryLine label="Minimum Development expectancy" value={`≥ ${formatNumber(form.minimumDevelopmentExpectancyR ?? 0, 2)}R`} />
        <SummaryLine label="OOS1 pick" value="Off · fold-stable Development R replaces it" />
        <SummaryLine label="Robustness" value={`6C2 Development CPCV · ${form.robustnessMonteCarloTrials ?? 0} MC · block ${form.robustnessMonteCarloBlockLength ?? 5} · P80 ${(form.robustnessMonteCarloP80ProfitRetention ?? 0.6) * 100}% · ${form.robustnessNeighborhoodSamples ?? 0} params`} />
        <SummaryLine label="Sealed holdout" value="Display only · not a Discover gate" accent />
      </div>

      <div className="contract-section">
        <p className="eyebrow">Execution modules</p>
        <div className="contract-chips">
          {enabledModules.map((module) => <span key={module}>{module}</span>)}
        </div>
      </div>

      <div className="contract-section contract-bindings">
        <p className="eyebrow">Bindings</p>
        {boundSources.map(([label, bound]) => (
          <div className={bound ? "contract-binding bound" : "contract-binding"} key={label}>
            <i aria-hidden="true" />
            <span>{label}</span>
            <strong>{bound ? "Bound" : "Required"}</strong>
          </div>
        ))}
      </div>

      <p className="contract-note">
        Development alone drives breeding and ranking. OOS1 is opened only as a post-Development validation gate and its metrics are stored with passing elites. OOS2 is never loaded by Discover and opens once for final portfolio certification. The full contract is sealed into the databank manifest.
      </p>
    </aside>
  );
}

function SummaryLine({
  accent = false,
  label,
  value,
}: {
  accent?: boolean;
  label: string;
  value: string;
}) {
  return <div className="summary-line"><span>{label}</span><strong className={accent ? "accent" : ""}>{value}</strong></div>;
}

function SymbolSelect({
  disabled = false,
  onError,
  onSelect,
  selectedSymbol,
  selectedDataPath,
}: {
  disabled?: boolean;
  onError: (message: string | null) => void;
  onSelect: (symbol: SymbolPack) => void;
  selectedSymbol?: string | null;
  selectedDataPath?: string;
}) {
  const [symbols, setSymbols] = useState<SymbolPack[]>([]);
  const boundSymbol = selectedSymbol ?? symbolFromDataPath(selectedDataPath);
  useEffect(() => {
    void listSymbols()
      .then((items) => {
        setSymbols(items);
        if (!boundSymbol && items[0]) {
          onSelect(items[0]);
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
        value={boundSymbol ?? ""}
        onChange={(event) => {
          const next = symbols.find((item) => item.symbol === event.target.value);
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
  const [form, setForm] = useState<JudgeRequest>({ decisionDataPath: "", decisionMetadataPath: null, decisionSourceTimezone: "Etc/UTC", m1DataPath: "", m1MetadataPath: null, m1SourceTimezone: "Etc/UTC", quotePath: null, splitPlanPath: null, strategyPath: "", brokerPath: "", outputPath: "", commissionPerLotRoundTurn: 7, slippagePointsPerSide: 0, fallbackSpreadPoints: null, maxSpreadPoints: null, initialBalance: 100000, entryWindowStartHour: 2, entryWindowEndHour: 19 });
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
              quotePath: symbol.quotePath,
              brokerPath: symbol.brokerPath,
              commissionPerLotRoundTurn: defaultCommissionPerLotRoundTurn(symbol.symbol),
            }));
          }}
          selectedDataPath={form.decisionDataPath}
        />
        <PathField label="Split plan (uses frozen OOS1 only)" path={form.splitPlanPath ?? ""} choose={() => chooseJsonFile("Choose split plan")} onChange={(value) => update("splitPlanPath", value || null)} />
        <PathField label="M1 bid/ask quote sidecar (required for certification)" path={form.quotePath ?? ""} choose={() => chooseTesterCsv("Choose M1 quote sidecar")} onChange={(value) => update("quotePath", value || null)} />
        <PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required />
        <PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required />
        <PathField label="New Judge artifact" path={form.outputPath} choose={() => chooseOutputJson("Save Judge artifact", "judge.json")} onChange={(value) => update("outputPath", value)} required />
      </div>
      <div className="numeric-grid"><NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value ?? 0)} step={.01} /><NumberField label="Slippage / side" value={form.slippagePointsPerSide} onChange={(value) => update("slippagePointsPerSide", value ?? 0)} step={.1} /><NumberField label="Initial balance" value={form.initialBalance} onChange={(value) => update("initialBalance", value ?? 100000)} min={1} /><NumberField label="Entry window start (broker)" value={form.entryWindowStartHour ?? 2} onChange={(value) => update("entryWindowStartHour", Math.min(23, Math.max(0, value ?? 2)))} min={0} /><NumberField label="Entry window end (broker, exclusive)" value={form.entryWindowEndHour ?? 19} onChange={(value) => update("entryWindowEndHour", Math.min(24, Math.max(1, value ?? 19)))} min={1} /></div>
      <div className="form-footer"><p>M1 files are bid OHLC. The matching <code>.quotes.csv</code> sidecar is required for stop/limit certification and for strict MT5 parity; without it the replay is diagnostic-only (synthetic ask = bid + spread).</p><button className="primary" disabled={busy || !form.decisionDataPath || !form.m1DataPath || !form.strategyPath || !form.brokerPath || !form.outputPath} onClick={run}>{busy ? "Replaying…" : "Run M1 Judge"}</button></div>
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
  const [form, setForm] = useState<ExportRequest>({ strategyPath: "", brokerPath: "", outputDirectory: "", expertName: "", expertDirectory: "QuantForge", timeframe: "H1", magic: 42424242, deviationPoints: 10, maxSpreadPoints: null, slippagePointsPerSide: 0, commissionPerLotRoundTurn: 7, deposit: 100000, currency: "USD", leverage: 100, testerModel: 1, entryWindowStartHour: 2, entryWindowEndHour: 19 });
  const [result, setResult] = useState<ExportView | null>(null); const [busy, setBusy] = useState(false);
  useEffect(() => {
    setForm((current) => ({ ...current, ...preset }));
  }, [preset]);
  function update<K extends keyof ExportRequest>(key: K, value: ExportRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await exportMql5(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Guarded compiler target</p><h2>Generate MT5 expert + tester pack</h2></div><span className="read-only-badge">Live off</span></div><div className="form-stack compact"><PathField label="Strategy IR" path={form.strategyPath} choose={() => chooseJsonFile("Choose strategy IR")} onChange={(value) => update("strategyPath", value)} required /><PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required /><PathField label="New export directory" path={form.outputDirectory} choose={() => chooseNewDirectory("Create EA export directory", "quantforge-ea-export")} onChange={(value) => update("outputDirectory", value)} required /><label className="field-row"><span>Expert name</span><input value={form.expertName} placeholder="auto: symbol + generation, e.g. EURUSD_g898_84" onChange={(event) => update("expertName", event.target.value)} /></label><label className="field-row"><span>Timeframe</span><input value={form.timeframe} onChange={(event) => update("timeframe", event.target.value.toUpperCase())} /></label></div><div className="numeric-grid"><NumberField label="Magic" value={form.magic} onChange={(value) => update("magic", value ?? 42424242)} min={1} /><NumberField label="Commission / lot RT" value={form.commissionPerLotRoundTurn} onChange={(value) => update("commissionPerLotRoundTurn", value ?? 0)} step={.01} /><NumberField label="Deposit" value={form.deposit} onChange={(value) => update("deposit", value ?? 100000)} min={1} /><NumberField label="Leverage" value={form.leverage} onChange={(value) => update("leverage", value ?? 100)} min={1} /><NumberField label="Entry window start (broker)" value={form.entryWindowStartHour ?? 2} onChange={(value) => update("entryWindowStartHour", Math.min(23, Math.max(0, value ?? 2)))} min={0} /><NumberField label="Entry window end (broker, exclusive)" value={form.entryWindowEndHour ?? 19} onChange={(value) => update("entryWindowEndHour", Math.min(24, Math.max(1, value ?? 19)))} min={1} /></div><div className="form-footer"><p>The generated EA inherits this entry window as inputs, and the set file keeps AllowLiveTrading=false. Protective stops and all selected management/order genes are emitted into the EA.</p><button className="primary" disabled={busy || !form.strategyPath || !form.brokerPath || !form.outputDirectory} onClick={run}>{busy ? "Generating…" : "Generate EA pack"}</button></div></section>{result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">EA export</p><h2>{result.symbol} · {result.timeframe}</h2></div><span className="grade-pill">guarded</span></div><ArtifactPath label="MQL5 source" value={result.sourcePath} /><ArtifactPath label="Tester config" value={result.testerPath} /><ArtifactPath label="Evidence" value={result.evidencePath} /><div className="safety-card"><strong>Live trading default</strong><span>{String(result.liveTradingDefault)}</span></div></section> : <WorkspacePrimer title="No EA generated yet" copy="Export writes MQL5 source, guarded settings, Strategy Tester configuration and a source-hash evidence card into one new directory." />}</div>;
}

function ParityComparePanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<ParityRequest>({ referencePath: "", evidencePath: "", mq5Path: "", mt5DealsPath: "", mt5EquityPath: "", mt5MetadataPath: "", quotePath: null, outputPath: "", initialBalance: 100000, tradeCountRelative: .1, tradeCountAbsolute: 3, netProfitRelative: .15, maxDrawdownRelative: .15, maxEquityDivergencePercent: 5, tradeTimestampToleranceMs: 0, minimumAlignedTradeFraction: .9, strictOneToOne: true });
  const [result, setResult] = useState<ParityView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof ParityRequest>(key: K, value: ParityRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await compareExternalParity(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">External truth</p><h2>Compare QuantForge with MT5 Strategy Tester</h2></div></div><div className="form-stack compact"><PathField label="Scout reference artifact" path={form.referencePath} choose={() => chooseJsonFile("Choose Scout artifact")} onChange={(value) => update("referencePath", value)} required /><PathField label="Export evidence" path={form.evidencePath} choose={() => chooseJsonFile("Choose export evidence")} onChange={(value) => update("evidencePath", value)} required /><PathField label="Generated MQL5" path={form.mq5Path} choose={chooseMql5File} onChange={(value) => update("mq5Path", value)} required /><PathField label="MT5 deals CSV" path={form.mt5DealsPath} choose={() => chooseTesterCsv("Choose MT5 deals export")} onChange={(value) => update("mt5DealsPath", value)} required /><PathField label="MT5 equity CSV" path={form.mt5EquityPath} choose={() => chooseTesterCsv("Choose MT5 equity export")} onChange={(value) => update("mt5EquityPath", value)} required /><PathField label="MT5 parity metadata" path={form.mt5MetadataPath} choose={() => chooseTesterCsv("Choose MT5 parity metadata")} onChange={(value) => update("mt5MetadataPath", value)} required /><PathField label="MT5 bid/ask M1 quote sidecar (required for certification)" path={form.quotePath ?? ""} choose={() => chooseTesterCsv("Choose MT5 quote sidecar")} onChange={(value) => update("quotePath", value || null)} /><PathField label="New parity artifact" path={form.outputPath} choose={() => chooseOutputJson("Save parity artifact", "parity.json")} onChange={(value) => update("outputPath", value)} required /></div><label className="check-field"><input type="checkbox" checked={form.strictOneToOne !== false} onChange={(event) => update("strictOneToOne", event.target.checked)} /><span>Certification mode: require one-to-one MT5 deals, equity and quote sidecar (recommended)</span></label><details className="advanced-settings"><summary>Diagnostic tolerances (only used when certification mode is off)</summary><div className="numeric-grid"><NumberField label="Trade count relative" value={form.tradeCountRelative} onChange={(value) => update("tradeCountRelative", value ?? .1)} step={.01} /><NumberField label="Trade count absolute" value={form.tradeCountAbsolute} onChange={(value) => update("tradeCountAbsolute", value ?? 3)} min={0} /><NumberField label="Net profit relative" value={form.netProfitRelative} onChange={(value) => update("netProfitRelative", value ?? .15)} step={.01} /><NumberField label="Drawdown relative" value={form.maxDrawdownRelative} onChange={(value) => update("maxDrawdownRelative", value ?? .15)} step={.01} /><NumberField label="Equity divergence %" value={form.maxEquityDivergencePercent} onChange={(value) => update("maxEquityDivergencePercent", value ?? 5)} step={.1} /><NumberField label="Aligned fraction" value={form.minimumAlignedTradeFraction} onChange={(value) => update("minimumAlignedTradeFraction", value ?? .9)} step={.01} /></div></details><div className="form-footer"><p>The source hash, tester metadata and quote feed must match the evidence card before certification is accepted.</p><button className="primary" disabled={busy || Object.entries(form).some(([key, value]) => key.endsWith("Path") && value === "") || (form.strictOneToOne !== false && !form.quotePath)} onClick={run}>{busy ? "Reconciling…" : "Compare MT5 run"}</button></div></section>{result ? <section className={`panel result-panel result-${result.passed ? "pass" : "fail"}`}><div className="result-hero"><div><p className="eyebrow">External parity</p><h2>{result.passed ? "MT5 parity passed" : "Execution diff exceeds tolerance"}</h2></div><span className="grade-pill">{result.grade}</span></div><div className="job-kpis"><Kpi label="Trades" value={`${result.referenceTrades} / ${result.externalTrades}`} note="QuantForge / MT5" /><Kpi label="Aligned" value={`${result.alignedTrades} / ${result.requiredAlignedTrades}`} note="Required minimum" /><Kpi label="Win rate" value={`${formatNumber(result.referenceWinRate, 1)}% / ${formatNumber(result.externalWinRate, 1)}%`} note={`${formatNumber(result.referenceWinningTrades)} / ${formatNumber(result.externalWinningTrades)} wins`} /><Kpi label="Profit factor" value={`${result.referenceProfitFactor === null ? "—" : formatNumber(result.referenceProfitFactor, 2)} / ${result.externalProfitFactor === null ? "—" : formatNumber(result.externalProfitFactor, 2)}`} note="QuantForge / MT5" /><Kpi label="Recovery factor" value={`${result.referenceRecoveryFactor === null ? "—" : formatNumber(result.referenceRecoveryFactor, 2)} / ${result.externalRecoveryFactor === null ? "—" : formatNumber(result.externalRecoveryFactor, 2)}`} note={result.recoveryFactorPassed ? "QF / MT5 · gate passed" : "QF / MT5 · gate failed"} /><Kpi label="Profit delta" value={`${formatNumber(result.netProfitDeltaRelative * 100, 2)}%`} note="Relative" /><Kpi label="Equity divergence" value={`${formatNumber(result.equityDivergencePercent, 2)}%`} note={result.protectiveOrdersPresent ? "Stops verified" : "Stops missing"} /></div><ArtifactPath label="Parity artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No external run compared" copy="Run the generated EA in MT5 Strategy Tester, then select its deals, equity and metadata exports here. Passing output becomes the external parity gate." />}</div>;
}

function IndicatorParityPanel({ onError }: { onError: (message: string | null) => void }) {
  const [form, setForm] = useState<IndicatorParityRequest>({ referencePath: "", outputPath: "", warmupRows: 1000, absoluteEpsilon: 1e-10, relativeEpsilon: 1e-9 });
  const [result, setResult] = useState<IndicatorParityView | null>(null); const [busy, setBusy] = useState(false);
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await compareIndicatorParity(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Buffer-level reconciliation</p><h2>Compare Rust indicators with MT5</h2></div></div><div className="form-stack"><PathField label="Indicator probe CSV" path={form.referencePath} choose={() => chooseTesterCsv("Choose MT5 indicator probe export")} onChange={(value) => setForm((current) => ({ ...current, referencePath: value }))} required /><PathField label="New indicator parity artifact" path={form.outputPath} choose={() => chooseOutputJson("Save indicator parity artifact", "indicator-parity.json")} onChange={(value) => setForm((current) => ({ ...current, outputPath: value }))} required /></div><div className="numeric-grid"><NumberField label="Warmup rows" value={form.warmupRows} onChange={(value) => setForm((current) => ({ ...current, warmupRows: value ?? 1000 }))} min={0} /><NumberField label="Absolute epsilon" value={form.absoluteEpsilon} onChange={(value) => setForm((current) => ({ ...current, absoluteEpsilon: value ?? 1e-10 }))} step={1e-10} /><NumberField label="Relative epsilon" value={form.relativeEpsilon} onChange={(value) => setForm((current) => ({ ...current, relativeEpsilon: value ?? 1e-9 }))} step={1e-9} /></div><div className="form-footer"><p>All thirteen supported indicator buffers must match after warmup before certification.</p><button className="primary" disabled={busy || !form.referencePath || !form.outputPath} onClick={run}>{busy ? "Comparing buffers…" : "Compare indicators"}</button></div></section>{result ? <section className={`panel result-panel result-${result.passed ? "pass" : "fail"}`}><div className="result-hero"><div><p className="eyebrow">Indicator parity</p><h2>{result.symbol} · {result.timeframe}</h2></div><span className="grade-pill">{result.passed ? "passed" : "failed"}</span></div><div className="job-kpis"><Kpi label="Fields" value={formatNumber(result.fieldCount)} note="Required buffers" /><Kpi label="Compared rows" value={formatNumber(result.comparedRows)} note={`${formatNumber(result.sourceRows)} source rows`} /><Kpi label="Mismatches" value={formatNumber(result.mismatchCount)} note="Across all fields" /></div><ArtifactPath label="Indicator parity artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No indicator probe compared" copy="Run the bundled QuantForge indicator probe EA in MT5, export its CSV, then verify SMA, EMA, WMA, RSI, ATR, ranges, z-score, percentile and rate-of-change buffers here." />}</div>;
}

function PortfolioWorkspace({ onError, workspace }: { onError: (message: string | null) => void; workspace: DatabankWorkspace | null }) {
  const [form, setForm] = useState<PortfolioRequest>({ databankPath: workspace?.sourcePath ?? "", brokerPath: "", outputPath: "", objective: "risk_adjusted_return", maximumPairwiseCorrelation: .7, maximumWeightPerStrategy: .25, maximumSymbolExposure: 1, maximumCohortExposure: .5, maximumStrategies: 10, minimumReturnPercent: 0, cvarTailFraction: .05, stressTrials: 1000, stressBlockLength: 5, seed: 42 });
  const [result, setResult] = useState<PortfolioView | null>(null); const [busy, setBusy] = useState(false);
  function update<K extends keyof PortfolioRequest>(key: K, value: PortfolioRequest[K]) { setForm((current) => ({ ...current, [key]: value })); }
  async function run() { setBusy(true); setResult(null); onError(null); try { setResult(await buildPortfolio(form)); } catch (reason) { onError(String(reason)); } finally { setBusy(false); } }
  return <div className="tool-content"><section className="panel setup-panel"><div className="panel-heading"><div><p className="eyebrow">Hard exposure caps</p><h2>Pack a correlation-constrained portfolio</h2></div></div><div className="form-stack compact"><PathField label="Promotion-grade databank" path={form.databankPath} choose={chooseDatabank} onChange={(value) => update("databankPath", value)} required /><PathField label="Broker profile" path={form.brokerPath} choose={chooseBrokerFile} onChange={(value) => update("brokerPath", value)} required /><PathField label="New portfolio artifact" path={form.outputPath} choose={() => chooseOutputJson("Save portfolio artifact", "portfolio.json")} onChange={(value) => update("outputPath", value)} required /><label className="field-row"><span>Objective</span><select value={form.objective} onChange={(event) => update("objective", event.target.value as PortfolioRequest["objective"])}><option value="risk_adjusted_return">Risk-adjusted return</option><option value="cvar">CVaR</option><option value="minimize_drawdown">Minimize drawdown</option></select></label></div><div className="numeric-grid"><NumberField label="Correlation ceiling" value={form.maximumPairwiseCorrelation} onChange={(value) => update("maximumPairwiseCorrelation", value ?? .7)} step={.01} /><NumberField label="Max strategy weight" value={form.maximumWeightPerStrategy} onChange={(value) => update("maximumWeightPerStrategy", value ?? .25)} step={.01} /><NumberField label="Max cohort exposure" value={form.maximumCohortExposure} onChange={(value) => update("maximumCohortExposure", value ?? .5)} step={.01} /><NumberField label="Strategy cap" value={form.maximumStrategies} onChange={(value) => update("maximumStrategies", value ?? 10)} min={1} /><NumberField label="Stress trials" value={form.stressTrials} onChange={(value) => update("stressTrials", value ?? 1000)} min={1} /><NumberField label="Seed" value={form.seed} onChange={(value) => update("seed", value ?? 42)} min={0} /></div><div className="form-footer"><p>Weights, symbol exposure, cohort exposure and correlation ceilings are hard constraints—not suggestions.</p><button className="primary" disabled={busy || !form.databankPath || !form.brokerPath || !form.outputPath} onClick={run}>{busy ? "Stress packing…" : "Build portfolio"}</button></div></section>{result ? <section className="panel result-panel result-pass"><div className="result-hero"><div><p className="eyebrow">Portfolio pack</p><h2>{result.selectedStrategies} strategies selected</h2></div><span className="grade-pill">audited</span></div><div className="job-kpis"><Kpi label="Expected return" value={`${formatNumber(result.expectedReturnPercent, 2)}%`} note={`${result.sourceCandidates} source candidates`} /><Kpi label="Path drawdown" value={`${formatNumber(result.maximumDrawdownPercent, 2)}%`} note="Combined path" /><Kpi label="Max correlation" value={formatNumber(result.maximumPairwiseCorrelation, 3)} note="Observed pair" /><Kpi label="Stress CVaR" value={`${formatNumber(result.cvarReturnPercent, 2)}%`} note={`P95 DD ${formatNumber(result.p95DrawdownPercent, 2)}%`} /></div><div className="allocation-list">{result.allocations.map((item) => <div key={item.fingerprint}><code>{item.fingerprint.slice(0, 12)}</code><span>{item.cohort}</span><strong>{formatNumber(item.weight * 100, 1)}%</strong></div>)}</div><ArtifactPath label="Portfolio artifact" value={result.outputPath} /></section> : <WorkspacePrimer title="No portfolio packed" copy="The packer selects complementary elites from one immutable databank and stress-tests the combined return path with block bootstrap trials." />}</div>;
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

function SearchRangesWall({ value, onChange }: { value: SearchRangeProfile; onChange: (next: SearchRangeProfile) => void }) {
  const groups: Array<[string, Array<[keyof SearchRangeProfile, string]>]> = [
    ["Indicators", [["indicatorPeriod", "General indicator period"], ["atrPeriod", "ATR period"], ["rsiUpper", "RSI upper"], ["rsiLower", "RSI lower"], ["adxThreshold", "ADX strength threshold"], ["rocThreshold", "ROC threshold"], ["percentileLow", "Percentile low"], ["zscoreThreshold", "Z-score threshold"]]],
    ["Stops and orders", [["atrStopMultiple", "ATR stop"], ["atrTargetMultiple", "ATR target"], ["riskTargetMultiple", "R target"], ["pendingDistanceAtr", "Pending distance (ATR)"], ["pendingExpiryBars", "Pending expiry bars"], ["timeStopBars", "Time-stop bars"]]],
    ["Price action and sessions", [["impulseBodyRatio", "Impulse body ratio"], ["impulseCloseLocation", "Impulse close location"], ["atrPercentileMax", "ATR percentile maximum"], ["atrPercentileLookback", "ATR percentile lookback"], ["sessionStartHour", "Session start hour"], ["sessionRangeBars", "Session range bars"], ["swingBars", "Swing bars"], ["baseBars", "Base bars"], ["liquiditySweepThreshold", "Liquidity sweep score"]]],
  ];
  function set(key: keyof SearchRangeProfile, field: "minimum" | "maximum" | "step", next: number) {
    onChange({ ...value, [key]: { ...value[key], [field]: next } });
  }
  return (
    <div className="search-ranges-wall">
      {groups.map(([title, rows]) => (
        <div className="range-group" key={title}>
          <div className="range-group-head">
            <p className="eyebrow">{title}</p>
            <span>{rows.length} genes</span>
          </div>
          <div className="range-group-body">
            <div className="range-row range-row-head">
              <span>Gene</span>
              <span>Minimum</span>
              <span>Maximum</span>
              <span>Step</span>
            </div>
            {rows.map(([key, label]) => {
              const range = value[key];
              const invalid = !(range.minimum <= range.maximum) || !(range.step > 0);
              return (
                <div className={invalid ? "range-row invalid" : "range-row"} key={key}>
                  <span>{label}</span>
                  <label>
                    <input aria-label={`${label} minimum`} type="number" value={range.minimum} step="any" onChange={(event) => set(key, "minimum", Number(event.target.value))} />
                  </label>
                  <label>
                    <input aria-label={`${label} maximum`} type="number" value={range.maximum} step="any" onChange={(event) => set(key, "maximum", Number(event.target.value))} />
                  </label>
                  <label>
                    <input aria-label={`${label} step`} type="number" value={range.step} step="any" onChange={(event) => set(key, "step", Number(event.target.value))} />
                  </label>
                </div>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

function NumberField({
  label,
  min,
  max,
  onChange,
  optional = false,
  step = 1,
  value,
}: {
  label: string;
  min?: number;
  max?: number;
  onChange: (value: number | null) => void;
  optional?: boolean;
  step?: number;
  value: number | null;
}) {
  return <label className="number-field"><span>{label}{optional && <small>optional</small>}</span><input type="number" min={min} max={max} step={step} value={value ?? ""} onChange={(event) => onChange(event.target.value === "" ? null : Number(event.target.value))} /></label>;
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
  const entryClass = (count: number) => `cluster-entry-${count}`;

  return (
    <section className="panel cluster-panel">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">Behavioral cluster</p>
          <h2>Evidence × novelty</h2>
        </div>
        <div className="cluster-legend" aria-label="Entry condition counts">
          {[2, 3, 4].map((count) => (
            <span key={count}><i className={entryClass(count)} />{count} entry</span>
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
                className={`cluster-point ${entryClass(row.entryConditions)} ${selected === row.fingerprint ? "selected" : ""}`}
                key={row.fingerprint}
                onClick={() => onSelect(row.fingerprint)}
                style={{ left: `${x}%`, top: `${y}%` }}
                title={`${row.strategyId} · ${conditionLabel(row.entryConditions, row.exitConditions)} · evidence ${row.evidence.toFixed(2)} · novelty ${row.novelty.toFixed(3)}`}
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
        returnRetention: 0.90,
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
          <p className="eyebrow">Final parity gate · SQX RetestWithHigherPrecision</p>
          <h2>M1 fidelity (after Discover)</h2>
        </div>
        <button className="primary" disabled={busy} onClick={() => void run()}>
          {busy ? "Retesting…" : "Run M1 fidelity gate"}
        </button>
      </div>
      <p className="immutable-note">
        Selected-TF elites only. Retest on M1 with strict bands (≥90% return, ≥80% trades, DD ≤130% of Selected TF).
        This is the final parity filter — not part of Discover breeding. Passers write a promotion-grade databank.
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
  group,
  onSelect,
}: {
  group: ConditionCoverage;
  onSelect: (fingerprint: string) => void;
}) {
  return (
    <article className="family-grid-card">
      <header>
        <strong>{group.label}</strong>
        <span>{group.occupied} / {group.total}</span>
      </header>
      <div className="coverage-grid">
        {group.cells.map((cell) => (
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
            {m1FidelityVerified
              ? "Reconstructed from each strategy's stored M1-verified equity signature; start and end balances match its recorded backtest."
              : "Reconstructed from each strategy's stored Selected-TF signature. This is not a M1 or MT5 parity result; use the M1 recheck below and the Fidelity demo before relying on it."}
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
  onDoubleClick,
  compact = false,
  showBatch = true,
  showActions = false,
  onExportIr,
  onExportMq5,
  onStage,
  rowHeightOverride,
  viewportHeightOverride,
}: {
  batchSelection: Set<string>;
  rows: EliteRow[];
  selected: string | null;
  onSelect: (fingerprint: string) => void;
  onToggle: (fingerprint: string) => void;
  onDoubleClick?: (fingerprint: string) => void;
  compact?: boolean;
  showBatch?: boolean;
  showActions?: boolean;
  onExportIr?: (fingerprint: string) => void;
  onExportMq5?: (fingerprint: string) => void;
  onStage?: (fingerprint: string) => void;
  rowHeightOverride?: number;
  viewportHeightOverride?: number;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const rowHeight = rowHeightOverride ?? (showActions ? 56 : 48);
  const viewportHeight = viewportHeightOverride ?? (compact ? 168 : 360);
  const overscan = 5;
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const end = Math.min(rows.length, start + Math.ceil(viewportHeight / rowHeight) + overscan * 2);

  return (
    <div className={`elite-table ${compact ? "compact" : ""} ${showBatch ? "" : "no-batch"} ${showActions ? "with-actions" : ""}`}>
      <div className="elite-row elite-header">
        {showBatch && <span>Select</span>}
        <span>Strategy</span>
        <span>Entry/Exit</span>
        <span>Mean R</span>
        <span>Fold R</span>
        <span>Trades</span>
        <span>Return</span>
        <span>DD</span>
        <span>Recovery</span>
        <span>Sharpe</span>
        {showActions && <span>Actions</span>}
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
              onDoubleClick={() => onDoubleClick?.(row.fingerprint)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") onSelect(row.fingerprint);
              }}
              role="button"
              style={{ height: rowHeight, transform: `translateY(${(start + offset) * rowHeight}px)` }}
              tabIndex={0}
            >
              {showBatch && (
                <span className="batch-check">
                  <input
                    aria-label={`Select ${row.strategyId} for batch export`}
                    checked={batchSelection.has(row.fingerprint)}
                    onChange={() => onToggle(row.fingerprint)}
                    onClick={(event) => event.stopPropagation()}
                    type="checkbox"
                  />
                </span>
              )}
              <span className="strategy-cell"><strong>{row.strategyId}</strong><small>{row.fingerprint.slice(0, 10)}</small></span>
              <span>{conditionLabel(row.entryConditions, row.exitConditions)}</span>
              <span>{(row.expectancyR ?? 0).toFixed(3)}</span>
              <span title={row.foldUsable ? `spread ${(row.foldSpread ?? 0).toFixed(3)} · ${row.foldCount} years` : "pooled R; too few trades per year"}>{(row.foldMedianR ?? 0).toFixed(3)}</span>
              <span>{formatNumber(row.trades)}</span>
              <span className={row.returnPercent >= 0 ? "positive" : "negative"}>{row.returnPercent.toFixed(2)}%</span>
              <span>{row.drawdownPercent.toFixed(2)}%</span>
              <span>{formatRecoveryFactor(row)}</span>
              <span>{row.sharpeRatio === null ? "—" : row.sharpeRatio.toFixed(2)}</span>
              {showActions && (
                <span className="row-actions" onClick={(event) => event.stopPropagation()}>
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

function EliteInspector({
  detail,
  loading,
  researchGrade,
  m1FidelityVerified,
  onError,
  onParity,
  onOpenResearch,
  tab,
  onTab,
}: {
  detail: EliteDetail | null;
  loading: boolean;
  researchGrade: boolean;
  m1FidelityVerified: boolean;
  onError: (message: string | null) => void;
  onParity: (fingerprint: string) => Promise<void>;
  onOpenResearch?: (fingerprint: string) => void;
  tab: "overview" | "ir" | "mq5";
  onTab: (tab: "overview" | "ir" | "mq5") => void;
}) {
  const [partitionView, setPartitionView] = useState<PartitionEquityView | null>(null);
  const [partitionBusy, setPartitionBusy] = useState(false);
  const [mq5View, setMq5View] = useState<EliteMql5SourceView | null>(null);
  const [mq5Busy, setMq5Busy] = useState(false);
  const [mq5Timeframe, setMq5Timeframe] = useState<"M1" | "M15" | "H1">("H1");
  const [mq5Copied, setMq5Copied] = useState(false);

  async function copyMq5Source() {
    if (!mq5View) return;
    try {
      await writeClipboardText(mq5View.source);
      setMq5Copied(true);
      window.setTimeout(() => setMq5Copied(false), 1_800);
    } catch (reason) {
      onError(`Could not copy MQL5 source: ${String(reason)}`);
    }
  }

  useEffect(() => {
    if (!detail || tab !== "overview" || detail.sealedProtected) {
      setPartitionView(null);
      setPartitionBusy(false);
      return;
    }
    let cancelled = false;
    setPartitionBusy(true);
    setPartitionView(null);
    void getElitePartitionEquity(detail.fingerprint)
      .then((next) => {
        if (!cancelled) setPartitionView(next);
      })
      .catch((reason) => {
        if (!cancelled) onError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setPartitionBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [detail?.fingerprint, detail?.sealedProtected, tab, onError]);

  useEffect(() => {
    if (!detail || tab !== "mq5") {
      setMq5View(null);
      return;
    }
    let cancelled = false;
    setMq5Busy(true);
    setMq5View(null);
    void getEliteMql5Source(detail.fingerprint, mq5Timeframe)
      .then((next) => {
        if (!cancelled) setMq5View(next);
      })
      .catch((reason) => {
        if (!cancelled) onError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setMq5Busy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [detail?.fingerprint, tab, mq5Timeframe, onError]);

  const displayMetrics = partitionView
    ? {
        returnPercent: partitionView.fullRunReturnPercent,
        maxDrawdownPercent: partitionView.fullRunMaxDrawdownPercent,
        tradeCount: partitionView.fullRunTrades,
        winRate: partitionView.fullRunWinRate,
        profitFactor: partitionView.fullRunProfitFactor,
        sharpeRatio: partitionView.fullRunSharpeRatio,
        source: "m1-full-run" as const,
      }
    : {
        returnPercent: Number(detail?.metrics.return_percent ?? 0),
        maxDrawdownPercent: Number(detail?.metrics.max_drawdown_percent ?? 0),
        tradeCount: Number(detail?.metrics.trade_count ?? 0),
        winRate: Number(detail?.metrics.win_rate ?? 0),
        profitFactor: detail?.metrics.profit_factor === null || detail?.metrics.profit_factor === undefined
          ? null
          : Number(detail.metrics.profit_factor),
        sharpeRatio: detail ? effectiveDetailSharpe(detail) : null,
        source: m1FidelityVerified ? "stored-is" as const : "stored-scout" as const,
      };

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
            <button className={tab === "mq5" ? "active" : ""} onClick={() => onTab("mq5")}>MQL5 source</button>
            <button onClick={() => void exportEliteStrategy(detail.fingerprint).catch((reason) => onError(String(reason)))}>Export IR</button>
            <button onClick={() => void exportEliteEas([detail.fingerprint], "H1", 42_424_242).catch((reason) => onError(String(reason)))}>Export MQ5</button>
            <button onClick={() => void promoteEliteToVault(detail.fingerprint).catch((reason) => onError(String(reason)))}>Stage for certification</button>
            <button onClick={() => void onParity(detail.fingerprint)}>Go to M1 / MT5 →</button>
            {onOpenResearch && <button className="primary" onClick={() => onOpenResearch(detail.fingerprint)}>Open research workspace</button>}
          </div>
          {tab === "overview" ? (
            <div className="inspector-body">
              <section>
                <p className="eyebrow">Thesis</p>
                <p>{detail.thesis}</p>
              </section>
              {!m1FidelityVerified && !partitionView && (
                <section className="legacy-metrics-warning">
                  <p className="eyebrow">Research-only archive</p>
                  <p>
                    The metric cards below are stored Selected-TF scout results. The M1 chronology panel is a recheck,
                    not an external MT5 parity pass. Promote only after the Fidelity demo and Parity Lab both pass.
                  </p>
                </section>
              )}
              {detail.sealedProtected ? (
                <section className="legacy-metrics-warning">
                  <p className="eyebrow">Sealed final protected</p>
                  <p>
                    Production Lane used Development only. Future M1 equity and expectancy are hidden here;
                    reveal them once through the explicit Sealed Final workflow after the shortlist is frozen.
                  </p>
                </section>
              ) : (
                <PartitionEquityChart
                  view={partitionView}
                  busy={partitionBusy}
                  researchGrade={researchGrade}
                  m1FidelityVerified={m1FidelityVerified}
                />
              )}
              <section className="metric-list">
                <Metric label="Mean R" value={(Number(detail.metrics.expectancy_r) || 0).toFixed(3)} />
                <Metric
                  label="Fold median R"
                  value={detail.foldUsable ? detail.foldMedianR.toFixed(3) : `${detail.foldPooledR.toFixed(3)} pooled`}
                />
                <Metric label="Fold spread" value={detail.foldUsable ? detail.foldSpread.toFixed(3) : "—"} />
                <Metric label="Evidence (rank mix)" value={Number(detail.evidence.total).toFixed(2)} />
                <Metric
                  label={displayMetrics.source === "m1-full-run" ? "Return (M1 full run)" : m1FidelityVerified ? "Return (IS stored)" : "Selected-TF return"}
                  value={`${displayMetrics.returnPercent.toFixed(2)}%`}
                />
                <Metric
                  label={displayMetrics.source === "m1-full-run" ? "Max drawdown (M1)" : "Max drawdown"}
                  value={`${displayMetrics.maxDrawdownPercent.toFixed(2)}%`}
                />
                <Metric
                  label="Recovery factor"
                  value={formatRecoveryFactor({
                    recoveryFactor: partitionView?.fullRunRecoveryFactor
                      ?? finiteRecoveryFromMetrics(detail?.metrics ?? {}),
                  })}
                />
                <Metric
                  label={displayMetrics.source === "m1-full-run" ? "Trades (M1 full run)" : "Trades"}
                  value={formatNumber(displayMetrics.tradeCount)}
                />
                <Metric label="Win rate" value={`${displayMetrics.winRate.toFixed(1)}%`} />
                <Metric
                  label="Profit factor"
                  value={displayMetrics.profitFactor === null ? "—" : displayMetrics.profitFactor.toFixed(2)}
                />
                <Metric
                  label="Sharpe"
                  value={displayMetrics.sharpeRatio?.toFixed(2) ?? "—"}
                />
              </section>
              {partitionView && m1FidelityVerified && (
                <section className="legacy-metrics-warning">
                  <p className="eyebrow">IS gate metrics (archived)</p>
                  <p>
                    Promotion used IS-only M1: {formatNumber(Number(detail.metrics.trade_count))} trades ·{" "}
                    {Number(detail.metrics.return_percent).toFixed(2)}% return. Metric cards above are the full-run M1
                    replay that Parity Lab and MT5 should match.
                  </p>
                </section>
              )}
              <section>
                <p className="eyebrow">Conditions · niche</p>
                <p>{conditionLabel(detail.entryConditions, detail.exitConditions)}</p>
                <code className="niche-code">{detail.niche}</code>
              </section>
              <div className="gate-stack">
                <span className="gate-pass">Development CPCV research</span>
                {detail.oos1ExpectancyRatio === null
                  ? <span className="gate-pending">OOS1 validation unavailable on this legacy elite</span>
                  : <span className="gate-pass">OOS1 validated · {detail.oos1ExpectancyRatio.toFixed(2)}× Development</span>}
                <span className="gate-pending">OOS2 sealed · not evaluated by Discover</span>
                <span className="gate-pending">External parity unknown</span>
              </div>
            </div>
          ) : tab === "ir" ? (
            <pre className="ir-tree">{JSON.stringify(detail.strategyIr, null, 2)}</pre>
          ) : (
            <div className="mq5-source-view">
              <div className="mq5-source-toolbar">
                <div>
                  <strong>{mq5View?.expertName ?? "Generating QuantForge expert…"}</strong>
                  <small>
                    {mq5View
                      ? `${mq5View.exportStyle} · SHA-256 ${mq5View.sourceHash}`
                      : "The preview is generated from this elite and the loaded broker profile."}
                  </small>
                </div>
                <label>
                  Timeframe
                  <select
                    value={mq5Timeframe}
                    onChange={(event) => setMq5Timeframe(event.target.value as "M1" | "M15" | "H1")}
                  >
                    <option value="M1">M1</option>
                    <option value="M15">M15</option>
                    <option value="H1">H1</option>
                  </select>
                </label>
                <button
                  type="button"
                  className="secondary"
                  disabled={!mq5View || mq5Busy}
                  onClick={() => void copyMq5Source()}
                >
                  {mq5Copied ? "Copied" : "Copy MQ5"}
                </button>
              </div>
              {mq5Busy || !mq5View
                ? <div className="inspector-loading">Generating deterministic MQL5 preview…</div>
                : <pre className="ir-tree mq5-source"><code>{mq5View.source}</code></pre>}
            </div>
          )}
        </>
      )}
    </aside>
  );
}

function PartitionEquityChart({
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
  /** Results detail uses the full-width, taller chart from the reference layout. */
  large?: boolean;
}) {
  const [sample, setSample] = useState<"full" | "is" | "oos1" | "oos2">("full");
  if (busy && !view) {
    return <div className="partition-equity loading">Replaying full Development / OOS1 / OOS2 equity…</div>;
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
  const oos1Pct = totalBars > 0 ? Math.round((view.oos1Bars / totalBars) * 100) : 0;
  const oos2Pct = totalBars > 0 ? Math.round((view.oos2Bars / totalBars) * 100) : 0;
  const twoWay = view.oos1Bars === 0;
  const splitNote = twoWay
    ? `${view.executionEngine} · Development ${isPct}% · Holdout ${oos2Pct}% (display only)`
    : `${view.executionEngine} · Development ${isPct}% · OOS1 ${oos1Pct}% · OOS2 ${oos2Pct}% (display only)`;

  return (
    <section className={large ? "partition-equity large" : "partition-equity"}>
      <div className="partition-equity-head">
        <div>
          <p className="eyebrow">M1-chronology full-run equity</p>
          <small>
            {splitNote}
            {researchGrade && !m1FidelityVerified ? " · research recheck; not an external parity pass" : ""}
          </small>
        </div>
        <div className="partition-equity-scale">
          <label className="partition-sample-select">Sample
            <select value={sample} onChange={(event) => setSample(event.target.value as typeof sample)}>
              <option value="full">Full (IS + OOS)</option>
              <option value="is">Development</option>
              {!twoWay && <option value="oos1">OOS 1</option>}
              <option value="oos2">{twoWay ? "Holdout" : "OOS 2"}</option>
            </select>
          </label>
          <span>${formatNumber(Math.max(...equities), 0)} peak</span>
          <span>${formatNumber(Math.min(...equities), 0)} trough</span>
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
        {sample === "full" && <>
          <rect x={pad} y={pad} width={isX - pad} height={height - pad * 2} className="region-is" />
          <rect x={isX} y={pad} width={Math.max(0, oos1X - isX)} height={height - pad * 2} className="region-oos1" />
          <rect x={oos1X} y={pad} width={Math.max(0, width - pad - oos1X)} height={height - pad * 2} className="region-oos2" />
          <line x1={isX} y1={pad} x2={isX} y2={height - pad} className="divider" />
          <line x1={oos1X} y1={pad} x2={oos1X} y2={height - pad} className="divider" />
        </>}
        <path d={areaPath} fill={`url(#${gradientId})`} className="equity-area" />
        <path d={path} className="equity-path" />
        <circle cx={lastX} cy={yAt(chartPoints.at(-1)!.equity)} r={large ? 3.2 : 2.5} className="equity-endpoint" />
        {sample === "full" ? <>
          <text x={pad + 6} y={pad + 14} className="region-label">IS</text>
          {!twoWay && <text x={isX + 6} y={pad + 14} className="region-label">OOS1</text>}
          <text x={(twoWay ? isX : oos1X) + 6} y={pad + 14} className="region-label">{twoWay ? "Holdout" : "OOS2"}</text>
        </> : <text x={pad + 6} y={pad + 14} className="region-label">{sample === "is" ? "IS" : sample.toUpperCase()}</text>}
      </svg>
      <div className="partition-kpis">
        <Kpi label="IS expectancy (R)" value={formatNumber(view.isExpectancy / 1000, 2)} note={`${formatNumber(view.isTrades)} trades · ${formatNumber(view.isReturnPercent, 2)}% return`} />
        {!twoWay && (
          <Kpi label="OOS1 expectancy (R)" value={formatNumber(view.oos1Expectancy / 1000, 2)} note={view.oos1ExpectancyRatio === null ? "no ratio" : `${view.oos1ExpectancyRatio.toFixed(2)}× Development · chart split`} />
        )}
        <Kpi label={twoWay ? "Holdout expectancy (R)" : "OOS2 expectancy (R)"} value={formatNumber(view.oos2Expectancy / 1000, 2)} note="display only · not a gate" />
        <Kpi label="Bars" value={twoWay ? `${formatNumber(view.isBars)} / ${formatNumber(view.oos2Bars)}` : `${formatNumber(view.isBars)} / ${formatNumber(view.oos1Bars)} / ${formatNumber(view.oos2Bars)}`} note={twoWay ? "Development / Holdout · databank split" : "Development / OOS1 / OOS2 · databank split"} />
      </div>
    </section>
  );
}

function EliteDetailModal({
  detail,
  loading,
  researchGrade,
  m1FidelityVerified,
  onClose,
  onError,
  onParity,
  tab,
  onTab,
}: {
  detail: EliteDetail | null;
  loading: boolean;
  researchGrade: boolean;
  m1FidelityVerified: boolean;
  onClose: () => void;
  onError: (message: string | null) => void;
  onParity: (fingerprint: string) => Promise<void>;
  tab: "overview" | "ir" | "mq5";
  onTab: (tab: "overview" | "ir" | "mq5") => void;
}) {
  const [partitionView, setPartitionView] = useState<PartitionEquityView | null>(null);
  const [partitionBusy, setPartitionBusy] = useState(false);

  useEffect(() => {
    if (!detail || tab !== "overview" || detail.sealedProtected) {
      setPartitionView(null);
      setPartitionBusy(false);
      return;
    }
    let cancelled = false;
    setPartitionBusy(true);
    setPartitionView(null);
    void getElitePartitionEquity(detail.fingerprint)
      .then((next) => {
        if (!cancelled) setPartitionView(next);
      })
      .catch((reason) => {
        if (!cancelled) onError(String(reason));
      })
      .finally(() => {
        if (!cancelled) setPartitionBusy(false);
      });
    return () => {
      cancelled = true;
    };
  }, [detail?.fingerprint, detail?.sealedProtected, tab, onError]);

  return (
    <div
      className="elite-detail-modal-backdrop"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") onClose();
      }}
      role="presentation"
    >
      <div
        className="elite-detail-modal"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={detail?.strategyId ?? "Strategy detail"}
      >
        <div className="elite-detail-modal-head">
          <div>
            <h2>{detail?.strategyId ?? "Loading strategy…"}</h2>
            <small>Inspection does not pause Discover · Esc to close</small>
          </div>
          <button className="secondary" type="button" onClick={onClose}>Close</button>
        </div>
        <div className="elite-detail-modal-body">
          <EliteInspector
            detail={detail}
            loading={loading}
            m1FidelityVerified={m1FidelityVerified}
            onError={onError}
            onParity={onParity}
            onTab={onTab}
            researchGrade={researchGrade}
            tab={tab}
          />
          <TradeListPanel busy={partitionBusy} trades={partitionView?.trades ?? []} />
        </div>
      </div>
    </div>
  );
}

function TradeListPanel({ trades, busy }: { trades: TradeRowView[]; busy: boolean }) {
  return (
    <aside className="trade-list-panel">
      <header>
        <p className="eyebrow">Trade list</p>
        <p>{busy ? "Replaying M1 chronology…" : `${formatNumber(trades.length)} trades (full run)`}</p>
      </header>
      <div className="trade-list-scroll">
        {trades.length === 0 ? (
          <p className="empty-inline">{busy ? "Loading…" : "No trades in replay."}</p>
        ) : (
          <table className="trade-list-table">
            <thead>
              <tr>
                <th>Side</th>
                <th>Entry</th>
                <th>Exit</th>
                <th>Net</th>
                <th>Reason</th>
              </tr>
            </thead>
            <tbody>
              {trades.map((trade, index) => (
                <tr key={`${trade.entryTimestampMs}-${index}`}>
                  <td>{trade.side}</td>
                  <td>{formatTradeTimestamp(trade.entryTimestampMs)} @ {trade.entryPrice.toFixed(5)}</td>
                  <td>{formatTradeTimestamp(trade.exitTimestampMs)} @ {trade.exitPrice.toFixed(5)}</td>
                  <td className={trade.netProfit >= 0 ? "profit" : "loss"}>
                    {trade.netProfit >= 0 ? "+" : ""}{trade.netProfit.toFixed(2)}
                  </td>
                  <td>{trade.exitReason}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </aside>
  );
}

function formatTradeTimestamp(timestampMs: number): string {
  return new Date(timestampMs).toISOString().replace("T", " ").slice(0, 16);
}

function Metric({ label, value }: { label: string; value: string }) {
  return <div><span>{label}</span><strong>{value}</strong></div>;
}

function formatRecoveryFactor(
  value: Pick<EliteRow, "recoveryFactor"> | { recoveryFactor: number | null | undefined },
): string {
  const factor = value.recoveryFactor;
  if (factor === null || factor === undefined) return "—";
  if (!Number.isFinite(factor)) return factor > 0 ? "∞" : "—";
  return factor.toFixed(2);
}

function finiteRecoveryFromMetrics(metrics: Record<string, unknown>): number | null {
  const netProfit = Number(metrics.net_profit ?? NaN);
  const maxDrawdown = Number(metrics.max_drawdown ?? NaN);
  if (!Number.isFinite(netProfit) || !Number.isFinite(maxDrawdown) || maxDrawdown <= 1e-12) {
    return null;
  }
  const value = netProfit / maxDrawdown;
  return Number.isFinite(value) ? value : null;
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
