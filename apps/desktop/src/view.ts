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
      if (sort === "returnDrawdown") {
        return compareDescending(
          comparableReturnDrawdown(left),
          comparableReturnDrawdown(right),
          left.evidence,
          right.evidence,
        );
      }
      if (sort === "sharpe") {
        return compareDescending(
          left.sharpeRatio ?? Number.NEGATIVE_INFINITY,
          right.sharpeRatio ?? Number.NEGATIVE_INFINITY,
          left.evidence,
          right.evidence,
        );
      }
      if (sort === "trades") return right.trades - left.trades;
      if (sort === "novelty") return right.novelty - left.novelty;
      return right.evidence - left.evidence;
    });
}

function compareDescending(
  left: number,
  right: number,
  leftTieBreaker: number,
  rightTieBreaker: number,
): number {
  if (left === right) return rightTieBreaker - leftTieBreaker;
  return left > right ? -1 : 1;
}

function comparableReturnDrawdown(elite: EliteRow): number {
  if (elite.returnDrawdown !== null) return elite.returnDrawdown;
  return elite.drawdownPercent <= 1e-12 && elite.returnPercent > 0
    ? Number.POSITIVE_INFINITY
    : Number.NEGATIVE_INFINITY;
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

export function discoverProgress(
  completed: number,
  requested: number,
  runUntilStopped = false,
): number {
  if (runUntilStopped && requested <= 0) {
    // Indeterminate continuous run — keep the bar visually full while active generations tick.
    return completed > 0 ? 100 : 0;
  }
  if (requested <= 0) return 0;
  return Math.max(0, Math.min(100, (completed / requested) * 100));
}

export function discoverProgressLabel(
  completed: number,
  requested: number,
  runUntilStopped: boolean,
): string {
  if (runUntilStopped && requested <= 0) {
    return `Generation ${completed} · running until stopped`;
  }
  if (runUntilStopped) {
    return `${completed} / ${requested} soft budget · until stopped`;
  }
  return `${completed} / ${requested} generations this run`;
}

export function bindDiscoverTimezone(request: DiscoverRequest): DiscoverRequest {
  return {
    ...request,
    sourceTimezone: request.metadataPath ? null : request.sourceTimezone,
    m1SourceTimezone: request.m1MetadataPath ? null : request.m1SourceTimezone,
  };
}
