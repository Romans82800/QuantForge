import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  DatabankWorkspace,
  BatchEaExportRequest,
  BatchEaExportView,
  BatchTradeCsvExportView,
  BatchExportView,
  CertifyRequest,
  AssembleEvidenceRequest,
  EvidenceView,
  ChallengeRequest,
  ChallengeView,
  DataLabRequest,
  DataLabView,
  MarketFolderImportRequest,
  MarketFolderImportView,
  DeployRequest,
  DeployView,
  DiscoverJobView,
  DiscoverRequest,
  EliteDetail,
  EliteMql5SourceView,
  ConditionBakeoffReport,
  ConditionBakeoffRequest,
  ExportRequest,
  ExportView,
  FidelityDemoRequest,
  FidelityDemoView,
  JudgeRequest,
  JudgeView,
  IncubationRecordRequest,
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
  VaultRequest,
  VaultView,
  AssetProfile,
  SymbolPack,
  PartitionEquityView,
  ResultsRobustnessRequest,
  ResultsRobustnessView,
  SavedDiscoverProfile,
  SavedSearchRangeProfile,
} from "./types";

async function chooseFile(
  title: string,
  name: string,
  extensions: string[],
): Promise<string | null> {
  const selected = await open({
    title,
    multiple: false,
    directory: false,
    filters: [{ name, extensions }],
  });
  return selected === null || Array.isArray(selected) ? null : selected;
}

export async function chooseAndLoadDatabank(): Promise<DatabankWorkspace | null> {
  const selected = await chooseDatabank();
  return selected ? loadDatabankPath(selected) : null;
}

export function loadDatabankPath(path: string): Promise<DatabankWorkspace> {
  return invoke<DatabankWorkspace>("load_databank", { path });
}

export function runFidelityDemo(request: FidelityDemoRequest): Promise<FidelityDemoView> {
  return invoke<FidelityDemoView>("run_fidelity_demo", { request });
}

export async function loadElite(fingerprint: string): Promise<EliteDetail> {
  return invoke<EliteDetail>("get_elite", { fingerprint });
}

export function getEliteMql5Source(
  fingerprint: string,
  timeframe = "H1",
  magic = 42_424_242,
): Promise<EliteMql5SourceView> {
  return invoke<EliteMql5SourceView>("get_elite_mql5_source", {
    fingerprint,
    timeframe,
    magic,
  });
}

export async function exportEliteStrategy(fingerprint: string): Promise<string | null> {
  const path = await save({
    title: "Export strategy IR",
    defaultPath: "strategy.ir.json",
    filters: [{ name: "Strategy IR", extensions: ["json"] }],
  });
  return path ? invoke<string>("export_elite_strategy", { fingerprint, path }) : null;
}

export async function exportEliteStrategies(
  fingerprints: string[],
): Promise<BatchExportView | null> {
  const directory = await chooseDirectory("Choose an empty batch export folder");
  return directory
    ? invoke<BatchExportView>("export_elite_strategies", { fingerprints, directory })
    : null;
}

export async function exportEliteEas(
  fingerprints: string[],
  timeframe: string,
  baseMagic: number,
): Promise<BatchEaExportView | null> {
  const directory = await chooseDirectory("Choose a folder for MQ5 experts");
  if (!directory) return null;
  const request: BatchEaExportRequest = {
    fingerprints,
    directory,
    timeframe,
    baseMagic,
  };
  return invoke<BatchEaExportView>("export_elite_eas", { request });
}

export async function exportEliteTradeCsvs(
  fingerprints: string[],
): Promise<BatchTradeCsvExportView | null> {
  const directory = await chooseDirectory("Choose a folder for strategy trade CSV files");
  return directory
    ? invoke<BatchTradeCsvExportView>("export_elite_trade_csvs", { fingerprints, directory })
    : null;
}

export function chooseDataFile(): Promise<string | null> {
  return chooseFile("Choose MT5 OHLC data", "OHLC data", ["csv", "tsv", "txt"]);
}

export function chooseMetadataFile(): Promise<string | null> {
  return chooseFile("Choose QuantForge MT5 metadata", "MT5 metadata", ["csv"]);
}

export function chooseBrokerFile(): Promise<string | null> {
  return chooseFile("Choose broker profile", "Broker profile", ["json"]);
}

export function chooseJsonFile(title = "Choose JSON artifact"): Promise<string | null> {
  return chooseFile(title, "JSON artifact", ["json"]);
}

export function chooseMql5File(): Promise<string | null> {
  return chooseFile("Choose generated MQL5 source", "MQL5 source", ["mq5"]);
}

export function chooseTesterCsv(title: string): Promise<string | null> {
  return chooseFile(title, "MT5 tester export", ["csv", "tsv", "txt"]);
}

export async function chooseDirectory(title: string): Promise<string | null> {
  const selected = await open({ title, multiple: false, directory: true });
  return selected === null || Array.isArray(selected) ? null : selected;
}

export function chooseNewDirectory(title: string, defaultPath: string): Promise<string | null> {
  return save({ title, defaultPath });
}

export function chooseOutputJson(title: string, defaultPath: string): Promise<string | null> {
  return save({
    title,
    defaultPath,
    filters: [{ name: "JSON artifact", extensions: ["json"] }],
  });
}

export function chooseDatabank(): Promise<string | null> {
  return chooseFile("Choose QuantForge databank", "QuantForge databank", ["json"]);
}

export function chooseNewDatabank(): Promise<string | null> {
  return save({
    title: "Create QuantForge databank",
    defaultPath: "quantforge-databank.json",
    filters: [{ name: "QuantForge databank", extensions: ["json"] }],
  });
}

export function inspectData(request: DataLabRequest): Promise<DataLabView> {
  return invoke<DataLabView>("inspect_data", { request });
}

export function importMarketFolder(request: MarketFolderImportRequest): Promise<MarketFolderImportView> {
  return invoke<MarketFolderImportView>("import_market_folder", { request });
}

export function startDiscover(request: DiscoverRequest): Promise<DiscoverJobView> {
  return invoke<DiscoverJobView>("start_discover", { request });
}

export function runConditionBakeoff(
  request: ConditionBakeoffRequest,
): Promise<ConditionBakeoffReport> {
  return invoke<ConditionBakeoffReport>("run_condition_bakeoff", { request });
}

export async function promoteEliteToVault(fingerprint: string): Promise<string | null> {
  const vaultDirectory = await chooseDirectory("Choose Vault directory for candidate staging");
  return vaultDirectory
    ? invoke<string>("promote_elite_to_vault", { fingerprint, vaultDirectory })
    : null;
}

export function getDiscoverJob(): Promise<DiscoverJobView> {
  return invoke<DiscoverJobView>("get_discover_job");
}

export function pauseDiscover(): Promise<DiscoverJobView> {
  return invoke<DiscoverJobView>("pause_discover");
}

export function resumeDiscover(): Promise<DiscoverJobView> {
  return invoke<DiscoverJobView>("resume_discover");
}

export function stopDiscover(): Promise<DiscoverJobView> {
  return invoke<DiscoverJobView>("stop_discover");
}

export function runChallenge(request: ChallengeRequest): Promise<ChallengeView> {
  return invoke<ChallengeView>("run_challenge_workflow", { request });
}

export function runSealedFinal(request: SealedRequest): Promise<SealedView> {
  return invoke<SealedView>("run_sealed_final", { request });
}

export function startIncubation(request: IncubationStartRequest): Promise<IncubationView> {
  return invoke<IncubationView>("start_incubation", { request });
}

export function recordIncubation(request: IncubationRecordRequest): Promise<IncubationView> {
  return invoke<IncubationView>("record_incubation", { request });
}

export function finalizeIncubation(startPath: string): Promise<IncubationView> {
  return invoke<IncubationView>("finalize_incubation", { request: { startPath } });
}

export function assembleEvidence(request: AssembleEvidenceRequest): Promise<EvidenceView> {
  return invoke<EvidenceView>("assemble_evidence", { request });
}

export function runM1Judge(request: JudgeRequest): Promise<JudgeView> {
  return invoke<JudgeView>("run_m1_judge", { request });
}

export function exportMql5(request: ExportRequest): Promise<ExportView> {
  return invoke<ExportView>("export_mql5", { request });
}

export function compareExternalParity(request: ParityRequest): Promise<ParityView> {
  return invoke<ParityView>("compare_external_parity", { request });
}

export function compareIndicatorParity(request: IndicatorParityRequest): Promise<IndicatorParityView> {
  return invoke<IndicatorParityView>("compare_indicator_parity", { request });
}

export function buildPortfolio(request: PortfolioRequest): Promise<PortfolioView> {
  return invoke<PortfolioView>("build_portfolio", { request });
}

export function inspectVault(request: VaultRequest): Promise<VaultView> {
  return invoke<VaultView>("inspect_vault", { request });
}

export function certifyToVault(request: CertifyRequest): Promise<VaultView> {
  return invoke<VaultView>("certify_to_vault", { request });
}

export function buildDeploymentPack(request: DeployRequest): Promise<DeployView> {
  return invoke<DeployView>("build_deployment_pack", { request });
}

export function listAssets(): Promise<AssetProfile[]> {
  return invoke<AssetProfile[]>("list_assets");
}

export function listSymbols(): Promise<SymbolPack[]> {
  return invoke<SymbolPack[]>("list_symbols");
}

export function listSearchRangeProfiles(): Promise<SavedSearchRangeProfile[]> {
  return invoke<SavedSearchRangeProfile[]>("list_search_range_profiles");
}

export function saveSearchRangeProfile(profile: SavedSearchRangeProfile): Promise<SavedSearchRangeProfile[]> {
  return invoke<SavedSearchRangeProfile[]>("save_search_range_profile", { profile });
}

export function deleteSearchRangeProfile(id: string): Promise<SavedSearchRangeProfile[]> {
  return invoke<SavedSearchRangeProfile[]>("delete_search_range_profile", { id });
}

export function listDiscoverProfiles(): Promise<SavedDiscoverProfile[]> {
  return invoke<SavedDiscoverProfile[]>("list_discover_profiles");
}

export function saveDiscoverProfile(profile: SavedDiscoverProfile): Promise<SavedDiscoverProfile[]> {
  return invoke<SavedDiscoverProfile[]>("save_discover_profile", { profile });
}

export function deleteDiscoverProfile(id: string): Promise<SavedDiscoverProfile[]> {
  return invoke<SavedDiscoverProfile[]>("delete_discover_profile", { id });
}

export function getElitePartitionEquity(fingerprint: string): Promise<PartitionEquityView> {
  return invoke<PartitionEquityView>("get_elite_partition_equity", { fingerprint });
}

export function runEliteRobustness(
  request: ResultsRobustnessRequest,
): Promise<ResultsRobustnessView> {
  return invoke<ResultsRobustnessView>("run_elite_robustness", { request });
}

export function upsertAsset(asset: AssetProfile): Promise<AssetProfile[]> {
  return invoke<AssetProfile[]>("upsert_asset", { asset });
}

export function deleteAsset(id: string): Promise<AssetProfile[]> {
  return invoke<AssetProfile[]>("delete_asset", { id });
}
