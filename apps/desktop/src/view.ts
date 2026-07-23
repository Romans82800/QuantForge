import type {
  DiscoverRequest,
  EliteRow,
  EliteSort,
  SelectionBiasLevel,
} from "./types";

export function formatNumber(value: number, maximumFractionDigits = 0): string {
  return new Intl.NumberFormat("en-GB", { maximumFractionDigits }).format(value);
}

export function selectionBiasTone(level: SelectionBiasLevel): string {
  return `bias-${level}`;
}

export function filterAndSortElites(
  elites: EliteRow[],
  query: string,
  family: string,
  sort: EliteSort,
): EliteRow[] {
  const needle = query.trim().toLowerCase();
  return elites
    .filter(
      (elite) =>
        (family === "all" || elite.family === family) &&
        (needle.length === 0 ||
          elite.strategyId.toLowerCase().includes(needle) ||
          elite.fingerprint.toLowerCase().includes(needle)),
    )
    .sort((left, right) => {
      if (sort === "family") {
        return left.family.localeCompare(right.family) || right.evidence - left.evidence;
      }
      if (sort === "grade") {
        return left.grade.localeCompare(right.grade) || right.evidence - left.evidence;
      }
      if (sort === "drawdown") return left.drawdownPercent - right.drawdownPercent;
      if (sort === "trades") return right.trades - left.trades;
      if (sort === "novelty") return right.novelty - left.novelty;
      return right.evidence - left.evidence;
    });
}

export function visibleEliteFingerprint(
  elites: EliteRow[],
  selectedFingerprint: string | null,
): string | null {
  if (
    selectedFingerprint &&
    elites.some((elite) => elite.fingerprint === selectedFingerprint)
  ) {
    return selectedFingerprint;
  }
  return elites[0]?.fingerprint ?? null;
}

export function discoverProgress(completed: number, requested: number): number {
  if (requested <= 0) return 0;
  return Math.max(0, Math.min(100, (completed / requested) * 100));
}

export function bindDiscoverTimezone(request: DiscoverRequest): DiscoverRequest {
  return {
    ...request,
    sourceTimezone: request.metadataPath ? null : request.sourceTimezone,
    m1SourceTimezone: request.m1MetadataPath ? null : request.m1SourceTimezone,
  };
}
