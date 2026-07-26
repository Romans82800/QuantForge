export type SelectionBiasLevel = "recorded" | "elevated" | "high";

export type WorkspaceName =
  | "Home"
  | "Databank"
  | "Discover"
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
}

export interface CoverageCell {
  index: number;
  niche: string;
  occupied: boolean;
  fingerprint: string | null;
  intensity: number;
}

export interface FamilyCoverage {
  family: string;
  occupied: number;
  total: number;
  cells: CoverageCell[];
}

export interface EliteRow {
  fingerprint: string;
  strategyId: string;
  family: string;
  evidence: number;
  novelty: number;
  trades: number;
  returnPercent: number;
  drawdownPercent: number;
  returnDrawdown: number | null;
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
  maxOneEntryPerDay: boolean;
  families: FamilyCoverage[];
  elites: EliteRow[];
}

export interface BatchExportView {
  directory: string;
  indexPath: string;
  strategyPaths: string[];
}

export interface EliteDetail {
  fingerprint: string;
  strategyId: string;
  thesis: string;
  family: string;
  niche: string;
  grade: string;
  parity: string;
  evidence: Record<string, number>;
  descriptor: Record<string, string | number>;
  metrics: Record<string, number | null>;
  strategyIr: unknown;
  equitySignature: number[];
}

export type EliteSort =
  | "evidence"
  | "novelty"
  | "trades"
  | "drawdown"
  | "returnDrawdown"
  | "sharpe"
  | "family"
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
  firstTimestampMs: number;
  lastTimestampMs: number;
  quality: DataQualityView;
  discoverReady: boolean;
}

export type DiscoverMode = "new" | "continue";
export type DiscoverJobStatus =
  | "idle"
  | "running"
  | "paused"
  | "completed"
  | "failed";

export type SearchFamilyId =
  | "trend_pullback"
  | "momentum_burst"
  | "donchian_breakout"
  | "mean_reversion_band"
  | "zscore_reversion"
  | "session_orb"
  | "impulse_candle"
  | "vol_squeeze_break"
  | "supply_demand_reclaim"
  | "sweep_reclaim";

export type DiscoverRunModeId = "fast_scout" | "full_harvest";

export interface FamilyBakeoffRow {
  family: SearchFamilyId;
  medianOos1Expectancy: number;
  medianRetention: number;
  passRate: number;
  elites: number;
  potElites: number;
  evaluations: number;
}

export interface FamilyBakeoffReport {
  rows: FamilyBakeoffRow[];
  recommended: SearchFamilyId | null;
}

export interface FamilyBakeoffRequest {
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
  /** Families to test; empty is rejected by the backend. */
  families: SearchFamilyId[];
}

export interface DiscoverRequest {
  mode: DiscoverMode;
  dataPath: string;
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
  searchFamily: SearchFamilyId | null;
  runMode: DiscoverRunModeId | null;
  earlyStopPotElites: number | null;
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
  oos1ExpectancyRetention: number | null;
  requireM1Precision: boolean | null;
  simpleExits: boolean | null;
  flattenAt22: boolean | null;
  maxOneEntryPerDay: boolean | null;
  mutateAfterElites: number | null;
  randomFillFraction: number | null;
  workerThreads: number | null;
  requireM1Robustness: boolean | null;
  robustnessFolds: number | null;
  robustnessMonteCarloTrials: number | null;
  robustnessNeighborhoodSamples: number | null;
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
  mutateAfterElites: number;
  breedingActive: boolean;
  workerThreads: number;
  coverage: number;
  qdScore: number;
  rejectedGate: number;
  rejectedDepositGate: number;
  rejectedPrecision: number;
  rejectedAmbiguous: number;
  rejectedOos1: number;
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
  evaluationsPerHour: number;
  acceptsPerHour: number;
  bestIsExpectancy: number | null;
  bestOos1Expectancy: number | null;
  topEvaluationErrors: EvaluationErrorCount[];
  m1BarsRepaired: number;
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
  folds: number;
  monteCarloTrials: number;
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
  splitPlanPath: string | null;
  strategyPath: string;
  brokerPath: string;
  outputPath: string;
  commissionPerLotRoundTurn: number;
  slippagePointsPerSide: number;
  fallbackSpreadPoints: number | null;
  maxSpreadPoints: number | null;
  initialBalance: number;
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
  outputPath: string;
  initialBalance: number;
  tradeCountRelative: number;
  tradeCountAbsolute: number;
  netProfitRelative: number;
  maxDrawdownRelative: number;
  maxEquityDivergencePercent: number;
  tradeTimestampToleranceMs: number;
  minimumAlignedTradeFraction: number;
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
  maximumFamilyExposure: number;
  maximumStrategies: number;
  minimumReturnPercent: number;
  cvarTailFraction: number;
  stressTrials: number;
  stressBlockLength: number;
  seed: number;
}

export interface PortfolioAllocationView {
  fingerprint: string;
  family: string;
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
