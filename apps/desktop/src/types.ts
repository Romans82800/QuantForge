export type SelectionBiasLevel = "recorded" | "elevated" | "high";

export type WorkspaceName =
  | "Home"
  | "Databank"
  | "Discover"
  | "Search Settings"
  | "Vault"
  | "Data Lab"
  | "Parity Lab"
  | "Portfolio"
  | "Deploy";

export interface SelectionBiasView {
  evaluationCount: number;
  level: SelectionBiasLevel;
  message: string;
}

export interface RejectionTelemetry {
  gate: number;
  clone: number;
  correlated: number;
  nicheNotImproved: number;
  precision: number;
  ambiguous?: number;
  oos1: number;
  developmentExpectancy: number;
  evaluation: number;
  total: number;
}

/** Saved H1 / M1 / metadata / broker binding for one symbol. */
export interface AssetProfile {
  id: string;
  name: string;
  dataPath: string;
  metadataPath: string | null;
  sourceTimezone: string | null;
  m1DataPath: string | null;
  m1MetadataPath: string | null;
  m1SourceTimezone: string | null;
  brokerPath: string;
  updatedAt: string;
}

/** Auto-scanned complete symbol from the ICMarkets data pack. */
export interface SymbolPack {
  symbol: string;
  dataPath: string;
  metadataPath: string;
  m1DataPath: string;
  m1MetadataPath: string;
  /** Bid/ask M1 quote sidecar when present beside the pack M1 file. */
  quotePath: string | null;
  brokerPath: string;
  defaultDatabankPath: string;
  packRoot: string;
}

export interface PartitionEquityPoint {
  timestampMs: number;
  equity: number;
}

export interface PartitionEquityView {
  fingerprint: string;
  strategyId: string;
  executionEngine: string;
  initialBalance: number;
  points: PartitionEquityPoint[];
  isEndTimestampMs: number;
  oos1EndTimestampMs: number;
  oos2EndTimestampMs: number;
  isBars: number;
  oos1Bars: number;
  oos2Bars: number;
  isExpectancy: number;
  oos1Expectancy: number;
  oos1ExpectancyRatio: number | null;
  oos2Expectancy: number;
  isReturnPercent: number;
  oos1ReturnPercent: number;
  oos2ReturnPercent: number;
  isTrades: number;
  oos1Trades: number;
  oos2Trades: number;
  fullRunTrades: number;
  fullRunReturnPercent: number;
  fullRunNetProfit: number;
  fullRunMaxDrawdown: number;
  fullRunMaxDrawdownPercent: number;
  fullRunProfitFactor: number | null;
  fullRunWinRate: number;
  fullRunSharpeRatio: number | null;
  fullRunRecoveryFactor: number | null;
  trades: TradeRowView[];
}

export interface TradeRowView {
  side: string;
  entryTimestampMs: number;
  exitTimestampMs: number;
  entryPrice: number;
  exitPrice: number;
  netProfit: number;
  exitReason: string;
}

export interface CoverageCell {
  index: number;
  niche: string;
  occupied: boolean;
  fingerprint: string | null;
  intensity: number;
}

export interface ConditionCoverage {
  entryConditions: number;
  label: string;
  occupied: number;
  total: number;
  cells: CoverageCell[];
}

export interface EliteRow {
  fingerprint: string;
  strategyId: string;
  entryConditions: number;
  exitConditions: number;
  evidence: number;
  novelty: number;
  trades: number;
  returnPercent: number;
  drawdownPercent: number;
  recoveryFactor: number | null;
  profitFactor: number | null;
  sharpeRatio: number | null;
  isExpectancy: number;
  oos1Expectancy: number | null;
  oos1ExpectancyRatio: number | null;
  complexity: number;
  generation: number;
  grade: string;
  parity: string;
  equitySignature: number[];
}

export interface DatabankWorkspace {
  sourcePath: string;
  dataPath: string;
  metadataPath: string | null;
  m1DataPath: string | null;
  m1MetadataPath: string | null;
  brokerPath: string;
  commissionPerLotRoundTurn: number;
  slippagePointsPerSide: number;
  initialBalance: number;
  artifactHash: string;
  runId: string;
  createdAt: string;
  dataHash: string;
  brokerSpecHash: string;
  grammarVersion: string;
  legacyReadOnly: boolean;
  qualityGrade: string;
  qualityScore: number;
  coverage: number;
  totalNiches: number;
  qdScore: number;
  completedGenerations: number;
  selectionBias: SelectionBiasView;
  rejections: RejectionTelemetry;
  researchGrade: boolean;
  requireM1Precision: boolean;
  m1FidelityVerified: boolean;
  simpleExits: boolean;
  allowBreakEven: boolean;
  allowTrailingStops: boolean;
  allowPartialExits: boolean;
  allowMarketEntries: boolean;
  allowStopEntries: boolean;
  allowLimitEntries: boolean;
  maxOneEntryPerDay: boolean;
  validationFraction: number;
  sealedFraction: number;
  conditionGroups: ConditionCoverage[];
  elites: EliteRow[];
}

export interface BatchExportView {
  directory: string;
  indexPath: string;
  strategyPaths: string[];
}

export interface BatchEaExportRequest {
  fingerprints: string[];
  directory: string;
  timeframe: string;
  baseMagic: number;
}

export interface BatchEaExportView {
  directory: string;
  indexPath: string;
  expertPaths: string[];
  settingsPaths: string[];
  testerPaths: string[];
  evidencePaths: string[];
}

export interface BatchTradeCsvExportView {
  directory: string;
  indexPath: string;
  csvPaths: string[];
}

export interface WalkForwardFold {
  fold: number;
  test_groups?: number[];
  start_timestamp_ms: number;
  end_timestamp_ms: number;
  decision_bars: number;
  trades_in_fold: number;
  metrics: Record<string, number | null>;
  passed: boolean;
}

/**
 * Serialized `quantforge_discover::RobustnessEvidence`. These keys stay
 * snake_case because the databank record is embedded verbatim.
 */
export interface RobustnessEvidence {
  m1_retention: {
    selected_timeframe_metrics: Record<string, number | null>;
    minimum_return_retention: number;
    return_retention?: number | null;
    trade_retention?: number | null;
    drawdown_expansion?: number | null;
  };
  walk_forward: {
    fold_scheme: string;
    purge_bars?: number;
    embargo_bars?: number;
    total_folds: number;
    passing_folds: number;
    passing_fraction: number;
    required_passing_fraction: number;
    folds?: WalkForwardFold[];
  };
  monte_carlo: {
    method: string;
    seed: number;
    trials: number;
    block_length: number;
    skip_trade_probability?: number;
    minimum_p05_net_profit?: number;
    maximum_p95_drawdown_percent?: number;
    baseline_max_drawdown_percent?: number;
    maximum_drawdown_ratio?: number;
    minimum_p80_profit_retention?: number;
    baseline_net_profit?: number;
    p05_net_profit: number;
    median_net_profit: number;
    p80_net_profit?: number;
    p95_drawdown_percent: number;
    worst_drawdown_percent: number;
    sample_paths?: number[][];
    passed: boolean;
  };
  parameter_neighborhood: {
    method?: string;
    perturbation_fraction: number;
    samples_requested: number;
    samples_evaluated: number;
    surviving_samples: number;
    survival_fraction: number;
    required_survival_fraction: number;
    plateau_neighbors: number;
    plateau_surviving: number;
    plateau_survival_fraction?: number | null;
    original_metrics?: Record<string, number | null> | null;
    samples?: ParameterNeighborhoodSample[];
  };
}

export interface ParameterNeighborhoodSample {
  sample_index: number;
  net_profit: number;
  return_percent: number;
  max_drawdown_percent: number;
  trade_count: number;
  profit_factor?: number | null;
  sharpe_ratio?: number | null;
  survived: boolean;
}

export type ResultsRobustnessMode = "standard" | "deep";

export interface ResultsRobustnessRequest {
  fingerprint: string;
  mode: ResultsRobustnessMode;
}

export interface ResultsRobustnessView {
  fingerprint: string;
  strategyId: string;
  mode: ResultsRobustnessMode;
  passed: boolean;
  blocker: string | null;
  message: string;
  artifactPath: string;
  folds: number;
  monteCarloTrials: number;
  neighborhoodSamples: number;
  evidence: RobustnessEvidence | null;
}

export interface EliteRobustnessView {
  monteCarlo?: string | null;
  walkForward?: string | null;
  paramPermutation?: string | null;
  summary?: string | null;
  /** Full battery record; absent for elites deposited before it was persisted. */
  evidence?: RobustnessEvidence | null;
}

export interface EliteDetail {
  fingerprint: string;
  strategyId: string;
  thesis: string;
  entryConditions: number;
  exitConditions: number;
  niche: string;
  grade: string;
  parity: string;
  evidence: Record<string, number>;
  descriptor: Record<string, string | number>;
  metrics: Record<string, number | null>;
  oos1Expectancy: number | null;
  oos1ExpectancyRatio: number | null;
  strategyIr: unknown;
  equitySignature: number[];
  /** Present when Discover persisted robustness gate detail; otherwise UI shows "not recorded". */
  robustness?: EliteRobustnessView | null;
}

export interface EliteMql5SourceView {
  fingerprint: string;
  expertName: string;
  timeframe: string;
  exportStyle: string;
  sourceHash: string;
  source: string;
}

export type EliteSort =
  | "evidence"
  | "novelty"
  | "trades"
  | "drawdown"
  | "recoveryFactor"
  | "sharpe"
  | "entryConditions"
  | "grade";

export interface DataLabRequest {
  dataPath: string;
  metadataPath: string | null;
  sourceTimezone: string | null;
  brokerPath: string | null;
}

export interface DataQualityView {
  grade: string;
  score: number;
  barCount: number;
  expectedIntervalSeconds: number | null;
  missingBarEstimate: number;
  gapEvents: number;
  duplicateRowsRemoved: number;
  zeroRangeBars: number;
  ohlcViolations: number;
  spikeBars: number;
  weekendBars: number;
  inputWasSorted: boolean;
}

export interface DataLabView {
  sourcePath: string;
  metadataPath: string | null;
  brokerPath: string | null;
  dataHash: string;
  metadataHash: string | null;
  brokerSpecHash: string | null;
  symbol: string | null;
  timeframe: string | null;
  brokerProfile: string | null;
  sourceRows: number;
  bars: number;
  duplicateRowsRemoved: number;
  inputWasSorted: boolean;
  delimiter: string;
  sourceTimezone: string;
  feedMode: string;
  quotePath: string | null;
  certificationReady: boolean;
  firstTimestampMs: number;
  lastTimestampMs: number;
  quality: DataQualityView;
  discoverReady: boolean;
}

export interface MarketFolderImportRequest {
  sourceDirectory: string;
  outputDirectory: string | null;
  sourceTimezone: string;
  aggregateTicksToBars: boolean;
}

export interface MarketFileImportView {
  sourcePath: string;
  symbol: string | null;
  kind: string;
  sourceRows: number;
  bars: number;
  m1Path: string | null;
  m1MetadataPath: string | null;
  h1Path: string | null;
  h1MetadataPath: string | null;
  quotePath: string | null;
  quoteMetadataPath: string | null;
  priceBasis: string | null;
  status: string;
  message: string | null;
}

export interface MarketFolderImportView {
  sourceDirectory: string;
  outputDirectory: string;
  sourceTimezone: string;
  files: MarketFileImportView[];
  importedCount: number;
  skippedCount: number;
}

export type DiscoverMode = "new" | "continue";
export type DiscoverJobStatus =
  | "idle"
  | "running"
  | "paused"
  | "completed"
  | "failed";

export interface UniversalGrammarConfig {
  minimumEntryConditions: number;
  maximumEntryConditions: number;
  minimumExitConditions: number;
  maximumExitConditions: number;
  minimumShift: number;
  maximumShift: number;
}

export type DiscoverRunModeId = "fast_scout" | "full_harvest" | "quota_harvest";

export interface ConditionBakeoffRow {
  entryConditions: number;
  medianIsExpectancyR: number;
  medianOos1ExpectancyR: number;
  medianRetention: number;
  passRate: number;
  elites: number;
  potElites: number;
  oos1Tested: number;
  evaluations: number;
}

export interface ConditionBakeoffReport {
  rows: ConditionBakeoffRow[];
  recommended: number | null;
}

export interface ConditionBakeoffRequest {
  dataPath: string;
  metadataPath: string | null;
  sourceTimezone: string | null;
  m1DataPath: string;
  m1MetadataPath: string | null;
  m1SourceTimezone: string | null;
  brokerPath: string;
  generations: number;
  initialCandidates: number;
  seed: number;
  commissionPerLotRoundTurn: number;
  slippagePointsPerSide: number;
  fallbackSpreadPoints: number | null;
  validationFraction: number;
  sealedFraction: number;
  /** Entry-condition counts to compare; empty defaults to 2, 3, 4 on the backend. */
  entryConditionCounts: number[];
}

export interface DiscoverRequest {
  mode: DiscoverMode;
  /** Explicit UI selection. The backend refuses data/broker bindings for another symbol. */
  selectedSymbol: string | null;
  dataPath: string;
  decisionTimeframe: "H1" | "M15" | null;
  metadataPath: string | null;
  sourceTimezone: string | null;
  m1DataPath: string;
  m1MetadataPath: string | null;
  m1SourceTimezone: string | null;
  brokerPath: string;
  databankPath: string;
  generations: number;
  runUntilStopped: boolean | null;
  initialCandidates: number | null;
  batchSize: number | null;
  correlationThreshold: number | null;
  noveltyWeight: number | null;
  seed: number | null;
  universalGrammar: UniversalGrammarConfig | null;
  runMode: DiscoverRunModeId | null;
  earlyStopPotElites: number | null;
  targetDatabankElites: number | null;
  searchRanges: SearchRangeProfile | null;
  minimumTrades: number | null;
  maximumDrawdownPercent: number | null;
  minimumReturnPercent: number | null;
  minimumProfitFactor: number | null;
  minimumReturnDrawdown: number | null;
  depositMinimumTrades: number | null;
  depositMaximumDrawdownPercent: number | null;
  depositMinimumReturnPercent: number | null;
  depositMinimumProfitFactor: number | null;
  depositMinimumReturnDrawdown: number | null;
  minimumM1ReturnRetention: number | null;
  minimumDevelopmentExpectancyR: number | null;
  oos1ExpectancyRetention: number | null;
  requireM1Precision: boolean | null;
  simpleExits: boolean | null;
  allowBreakEven: boolean | null;
  allowTrailingStops: boolean | null;
  allowPartialExits: boolean | null;
  allowMarketEntries: boolean | null;
  allowStopEntries: boolean | null;
  allowLimitEntries: boolean | null;
  flattenAt22: boolean | null;
  endOfDayHour: number | null;
  entryWindowStartHour: number | null;
  entryWindowEndHour: number | null;
  maxOneEntryPerDay: boolean | null;
  mutateAfterElites: number | null;
  randomFillFraction: number | null;
  workerThreads: number | null;
  /** Dedicated OOS1→M1 promotion workers; 0 / null = auto (2–4). */
  promotionWorkerThreads: number | null;
  /** Max waiting + in-flight promotions before backpressure. */
  promotionQueueCapacity: number | null;
  requireM1Robustness: boolean | null;
  robustnessFolds: number | null;
  robustnessMonteCarloTrials: number | null;
  robustnessMonteCarloBlockLength: number | null;
  robustnessMonteCarloSkipTradeProbability: number | null;
  robustnessMonteCarloP80ProfitRetention: number | null;
  robustnessMonteCarloMaxDrawdownRatio: number | null;
  robustnessNeighborhoodSamples: number | null;
  robustnessPerturbationFraction: number | null;
  minimumNeighborhoodSurvivalFraction: number | null;
  calendarYearFolds: boolean | null;
  minimumDeflatedTradeSharpe: number | null;
  multiSymbolMinimumPass: number | null;
  packDataDir: string | null;
  commissionPerLotRoundTurn: number | null;
  slippagePointsPerSide: number | null;
  fallbackSpreadPoints: number | null;
  maxSpreadPoints: number | null;
  initialBalance: number | null;
  promotionSplit: boolean | null;
  validationFraction: number | null;
  sealedFraction: number | null;
}

export interface SavedDiscoverProfile {
  id: string;
  name: string;
  settings: DiscoverRequest;
  updatedAt: string;
}

export interface SearchRange {
  minimum: number;
  maximum: number;
  step: number;
}

export interface SearchRangeProfile {
  indicatorPeriod: SearchRange;
  atrPeriod: SearchRange;
  atrStopMultiple: SearchRange;
  atrTargetMultiple: SearchRange;
  riskTargetMultiple: SearchRange;
  pendingDistanceAtr: SearchRange;
  pendingExpiryBars: SearchRange;
  timeStopBars: SearchRange;
  rsiUpper: SearchRange;
  rsiLower: SearchRange;
  adxThreshold: SearchRange;
  rocThreshold: SearchRange;
  percentileLow: SearchRange;
  zscoreThreshold: SearchRange;
  impulseBodyRatio: SearchRange;
  impulseCloseLocation: SearchRange;
  atrPercentileMax: SearchRange;
  atrPercentileLookback: SearchRange;
  sessionStartHour: SearchRange;
  sessionRangeBars: SearchRange;
  swingBars: SearchRange;
  baseBars: SearchRange;
  liquiditySweepThreshold: SearchRange;
}

export interface SavedSearchRangeProfile {
  id: string;
  name: string;
  ranges: SearchRangeProfile;
  updatedAt: string;
}

export interface EvaluationErrorCount {
  message: string;
  count: number;
}

export interface FidelityDemoRequest {
  databankPath: string;
  m1DataPath: string;
  m1MetadataPath: string | null;
  m1SourceTimezone: string | null;
  outputPath: string;
  returnRetention: number | null;
  tradeRetention: number | null;
  drawdownExpansion: number | null;
}

export interface FidelityEliteResult {
  fingerprint: string;
  strategyId: string;
  passed: boolean;
  h1ReturnPercent: number;
  m1ReturnPercent: number;
  returnRetention: number;
  h1Trades: number;
  m1Trades: number;
  tradeRetention: number;
  h1DrawdownPercent: number;
  m1DrawdownPercent: number;
  reason: string;
}

export interface FidelityDemoView {
  evaluated: number;
  passed: number;
  failed: number;
  outputPath: string | null;
  results: FidelityEliteResult[];
}

export interface DiscoverJobView {
  jobId: string | null;
  status: DiscoverJobStatus;
  mode: DiscoverMode | null;
  phase: string;
  outputPath: string | null;
  completedGenerations: number;
  requestedGenerations: number;
  runUntilStopped: boolean;
  evaluationCount: number;
  acceptedTotal: number;
  potElites: number;
  potNewNiches: number;
  databankElites: number;
  liveDatabankRevision: number;
  targetDatabankElites?: number | null;
  mutateAfterElites: number;
  breedingActive: boolean;
  workerThreads: number;
  promotionWorkerThreads: number;
  promotionQueueCapacity: number;
  promotionQueueDepth: number;
  promotionInflight: number;
  promotionsEnqueued: number;
  promotionsCompleted: number;
  promotionBackpressureEvents: number;
  promotionsPerHour: number;
  coverage: number;
  qdScore: number;
  rejectedGate: number;
  rejectedDepositGate: number;
  rejectedPrecision: number;
  rejectedAmbiguous: number;
  rejectedOos1: number;
  rejectedDevelopmentExpectancy: number;
  rejectedM1Fidelity: number;
  rejectedWalkForward: number;
  rejectedMonteCarlo: number;
  rejectedParamNeighborhood: number;
  rejectedMultiSymbol: number;
  rejectedDeflatedSharpe: number;
  rejectedClone: number;
  rejectedCorrelated: number;
  rejectedNicheNotImproved: number;
  rejectedEvaluation: number;
  rejectedTotal: number;
  /** Five-minute moving candidate throughput. */
  rollingEvaluationsPerHour: number;
  /** Whole-run active-time candidate throughput. */
  lifetimeEvaluationsPerHour: number;
  /** Backward-compatible alias for lifetimeEvaluationsPerHour. */
  evaluationsPerHour: number;
  acceptsPerHour: number;
  bestIsExpectancy: number | null;
  bestOos1Expectancy: number | null;
  topEvaluationErrors: EvaluationErrorCount[];
  m1BarsRepaired: number;
  latestImmutableSnapshotPath: string | null;
  startedAtMs: number | null;
  stopRequested: boolean;
  message: string;
}

export interface ChallengeRequest {
  dataPath: string;
  metadataPath: string | null;
  sourceTimezone: string | null;
  strategyPath: string;
  strategyPaths: string[];
  brokerPath: string;
  outputDirectory: string;
  validationFraction: number;
  sealedFraction: number;
  evaluationsTouched: number;
  commissionPerLotRoundTurn: number;
  slippagePointsPerSide: number;
  fallbackSpreadPoints: number | null;
  maxSpreadPoints: number | null;
  initialBalance: number;
  entryWindowStartHour: number | null;
  entryWindowEndHour: number | null;
  folds: number;
  monteCarloTrials: number;
  monteCarloBlockLength: number;
  monteCarloMinimumP80ProfitRetention: number;
  neighborhoodSamples: number;
  seed: number;
}

export interface ChallengeItemView {
  strategyPath: string;
  strategyId: string;
  passed: boolean;
  grade: string;
  challengePath: string;
  validationTrades: number;
  returnPercent: number;
  profitFactor: number | null;
  maximumDrawdownPercent: number;
  passingFolds: number;
  totalFolds: number;
  passingCostShocks: number;
  totalCostShocks: number;
  blockers: string[];
  error: string | null;
}

export interface ChallengeView {
  passed: boolean;
  grade: string;
  splitPlanPath: string;
  challengePath: string;
  developmentBars: number;
  validationBars: number;
  sealedBars: number;
  isBars: number;
  oos1Bars: number;
  oos2Bars: number;
  validationTrades: number;
  returnPercent: number;
  profitFactor: number | null;
  maximumDrawdownPercent: number;
  passingFolds: number;
  totalFolds: number;
  passingCostShocks: number;
  totalCostShocks: number;
  blockers: string[];
  results: ChallengeItemView[];
  passedCount: number;
  failedCount: number;
  totalCount: number;
}

export interface SealedRequest {
  dataPath: string;
  metadataPath: string | null;
  sourceTimezone: string | null;
  strategyPath: string;
  brokerPath: string;
  splitPlanPath: string;
  challengePath: string;
  sealedRoot: string;
  minimumTrades: number;
  minimumReturnPercent: number;
  minimumProfitFactor: number;
  maximumDrawdownPercent: number;
}

export interface SealedView {
  outputPath: string;
  passed: boolean;
  grade: string;
  trades: number;
  returnPercent: number;
  profitFactor: number | null;
  maximumDrawdownPercent: number;
  blockers: string[];
}

export interface IncubationStartRequest {
  strategyPath: string;
  brokerPath: string;
  splitPlanPath: string;
  rootDirectory: string;
  startDate: string;
  initialBalance: string;
  maximumDailyLossPercent: string;
  maximumTotalDrawdownPercent: string;
  minimumObservationDays: number;
  minimumTotalTrades: number;
  maximumConsecutiveZeroTradeDays: number;
}

export interface IncubationRecordRequest {
  startPath: string;
  date: string;
  endingBalance: number;
  maximumDrawdownPercent: number;
  tradeCount: number;
  note: string | null;
}

export interface IncubationView {
  startPath: string;
  finalPath: string | null;
  status: string;
  observationDays: number;
  totalTrades: number;
  returnPercent: number | null;
  maximumDrawdownPercent: number | null;
  passed: boolean | null;
  blockers: string[];
}

export interface AssembleEvidenceRequest {
  strategyPath: string;
  brokerPath: string;
  splitPlanPath: string;
  databankPath: string;
  challengePath: string;
  judgePath: string;
  parityPath: string;
  indicatorParityPath: string;
  sealedFinalPath: string;
  incubationPath: string | null;
  outputDirectory: string;
}

export interface EvidenceView {
  outputDirectory: string;
  validationPath: string;
  evidencePath: string;
  bundlePath: string;
  gateCount: number;
  evaluationsTouched: number;
  certificationReady: boolean;
  incubationIncluded: boolean;
}

export interface JudgeRequest {
  decisionDataPath: string;
  decisionMetadataPath: string | null;
  decisionSourceTimezone: string | null;
  m1DataPath: string;
  m1MetadataPath: string | null;
  m1SourceTimezone: string | null;
  quotePath: string | null;
  splitPlanPath: string | null;
  strategyPath: string;
  brokerPath: string;
  outputPath: string;
  commissionPerLotRoundTurn: number;
  slippagePointsPerSide: number;
  fallbackSpreadPoints: number | null;
  maxSpreadPoints: number | null;
  initialBalance: number;
  entryWindowStartHour: number | null;
  entryWindowEndHour: number | null;
}

export interface JudgeView {
  outputPath: string;
  grade: string;
  trades: number;
  returnPercent: number;
  profitFactor: number | null;
  maximumDrawdownPercent: number;
  decisionBars: number;
  m1Bars: number;
  pendingOrdersFilled: number;
  partialExits: number;
  breakEvenMoves: number;
  trailingMoves: number;
  endOfDayFlattens: number;
  verifiedNoTickGapEvents: number;
  verifiedNoTickMinutes: number;
}

export interface ExportRequest {
  strategyPath: string;
  brokerPath: string;
  outputDirectory: string;
  expertName: string;
  expertDirectory: string;
  timeframe: string;
  magic: number;
  deviationPoints: number;
  maxSpreadPoints: number | null;
  slippagePointsPerSide: number;
  commissionPerLotRoundTurn: number;
  deposit: number;
  currency: string;
  leverage: number;
  testerModel: number;
  entryWindowStartHour: number | null;
  entryWindowEndHour: number | null;
}

export interface ExportView {
  outputDirectory: string;
  sourcePath: string;
  settingsPath: string;
  testerPath: string;
  evidencePath: string;
  strategyFingerprint: string;
  sourceHash: string;
  symbol: string;
  timeframe: string;
  liveTradingDefault: boolean;
}

export interface ParityRequest {
  referencePath: string;
  evidencePath: string;
  mq5Path: string;
  mt5DealsPath: string;
  mt5EquityPath: string;
  mt5MetadataPath: string;
  quotePath: string | null;
  outputPath: string;
  initialBalance: number;
  tradeCountRelative: number;
  tradeCountAbsolute: number;
  netProfitRelative: number;
  maxDrawdownRelative: number;
  maxEquityDivergencePercent: number;
  tradeTimestampToleranceMs: number;
  minimumAlignedTradeFraction: number;
  strictOneToOne?: boolean;
}

export interface ParityView {
  outputPath: string;
  passed: boolean;
  grade: string;
  referenceEngine: string;
  externalEngine: string;
  referenceTrades: number;
  externalTrades: number;
  alignedTrades: number;
  requiredAlignedTrades: number;
  netProfitDeltaRelative: number;
  drawdownDeltaRelative: number;
  equityDivergencePercent: number;
  protectiveOrdersPresent: boolean;
  referenceWinRate: number;
  externalWinRate: number;
  referenceWinningTrades: number;
  externalWinningTrades: number;
  referenceProfitFactor: number | null;
  externalProfitFactor: number | null;
  referenceRecoveryFactor: number | null;
  externalRecoveryFactor: number | null;
  recoveryFactorDeltaRelative: number | null;
  recoveryFactorPassed: boolean;
}

export interface IndicatorParityRequest {
  referencePath: string;
  outputPath: string;
  warmupRows: number;
  absoluteEpsilon: number;
  relativeEpsilon: number;
}

export interface IndicatorParityView {
  outputPath: string;
  passed: boolean;
  symbol: string;
  timeframe: string;
  sourceRows: number;
  comparedRows: number;
  fieldCount: number;
  mismatchCount: number;
}

export type PortfolioObjective =
  | "risk_adjusted_return"
  | "cvar"
  | "minimize_drawdown";

export interface PortfolioRequest {
  databankPath: string;
  brokerPath: string;
  outputPath: string;
  objective: PortfolioObjective;
  maximumPairwiseCorrelation: number;
  maximumWeightPerStrategy: number;
  maximumSymbolExposure: number;
  maximumCohortExposure: number;
  maximumStrategies: number;
  minimumReturnPercent: number;
  cvarTailFraction: number;
  stressTrials: number;
  stressBlockLength: number;
  seed: number;
}

export interface PortfolioAllocationView {
  fingerprint: string;
  cohort: string;
  symbol: string;
  weight: number;
  returnPercent: number;
  drawdownPercent: number;
}

export interface PortfolioView {
  outputPath: string;
  portfolioId: string;
  sourceCandidates: number;
  selectedStrategies: number;
  expectedReturnPercent: number;
  maximumDrawdownPercent: number;
  maximumPairwiseCorrelation: number;
  p05ReturnPercent: number;
  cvarReturnPercent: number;
  p95DrawdownPercent: number;
  allocations: PortfolioAllocationView[];
}

export interface VaultRequest {
  vaultDirectory: string;
}

export interface CertifyRequest {
  strategyPath: string;
  brokerPath: string;
  splitPlanPath: string;
  evidencePath: string;
  artifactPaths: string[];
  vaultDirectory: string;
  requireIncubation: boolean;
  selectionBiasWarningThreshold: number;
}

export interface VaultEntryView {
  path: string;
  entryId: string;
  strategyFingerprint: string;
  admittedAt: string;
  grade: string;
  evidenceHash: string;
  warnings: number;
  incubationRequired: boolean;
}

export interface VaultView {
  vaultDirectory: string;
  certifiedEntries: VaultEntryView[];
  rejectedFiles: number;
}

export interface DeployRequest {
  vaultEntryPath: string;
  outputDirectory: string;
}

export interface DeployView {
  outputDirectory: string;
  deploymentId: string;
  grade: string;
  expertName: string;
  symbol: string;
  timeframe: string;
  magic: number;
  fileCount: number;
  liveTradingDefault: boolean;
  certificationWarnings: number;
}
