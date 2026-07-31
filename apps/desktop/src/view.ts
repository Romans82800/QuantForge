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
  entryConditions: string,
  sort: EliteSort,
  entryOrder = "all",
  expression = "",
): EliteRow[] {
  const needle = query.trim().toLowerCase();
  const expr = expression.trim();
  let parsed: ReturnType<typeof parseDatabankFilter> | null = null;
  let parseError: string | null = null;
  if (expr.length > 0) {
    try {
      parsed = parseDatabankFilter(expr);
    } catch (reason) {
      parseError = String(reason);
    }
  }
  return elites
    .filter(
      (elite) =>
        (entryConditions === "all" || String(elite.entryConditions) === entryConditions) &&
        (entryOrder === "all" || (elite.entryOrder ?? "market") === entryOrder) &&
        (needle.length === 0 ||
          elite.strategyId.toLowerCase().includes(needle) ||
          elite.fingerprint.toLowerCase().includes(needle) ||
          (elite.management ?? "").toLowerCase().includes(needle) ||
          String(elite.islandId ?? "").includes(needle)) &&
        (parsed === null || (parseError === null && evalDatabankFilter(parsed, elite))),
    )
    .sort((left, right) => {
      if (sort === "entryConditions") {
        return (
          left.entryConditions - right.entryConditions ||
          left.exitConditions - right.exitConditions ||
          right.evidence - left.evidence
        );
      }
      if (sort === "grade") {
        return left.grade.localeCompare(right.grade) || right.evidence - left.evidence;
      }
      if (sort === "drawdown") return left.drawdownPercent - right.drawdownPercent;
      if (sort === "recoveryFactor") {
        return compareDescending(
          comparableRecoveryFactor(left),
          comparableRecoveryFactor(right),
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

/** SQX-like databank expression: `PF > 1.5 AND Drawdown < 20`. */
export type DatabankFilterNode =
  | { type: "and" | "or"; children: DatabankFilterNode[] }
  | { type: "not"; child: DatabankFilterNode }
  | { type: "cmp"; column: string; op: string; value: number | string | boolean | null };

export function parseDatabankFilter(source: string): DatabankFilterNode {
  const parser = new FilterParser(source);
  const node = parser.parseOr();
  parser.skipWs();
  if (!parser.done()) throw new Error(`unexpected trailing input near \`${parser.rest()}\``);
  return node;
}

export function evalDatabankFilter(node: DatabankFilterNode, elite: EliteRow): boolean {
  switch (node.type) {
    case "and":
      return node.children.every((child) => evalDatabankFilter(child, elite));
    case "or":
      return node.children.some((child) => evalDatabankFilter(child, elite));
    case "not":
      return !evalDatabankFilter(node.child, elite);
    case "cmp": {
      const left = columnValue(elite, node.column);
      return compareFilterValues(left, node.op, node.value);
    }
  }
}

export function databankFilterError(expression: string): string | null {
  const trimmed = expression.trim();
  if (!trimmed) return null;
  try {
    parseDatabankFilter(trimmed);
    return null;
  } catch (reason) {
    return String(reason).replace(/^Error:\s*/, "");
  }
}

/** SQX DatabankFilterByCorrelation — greedy keep by evidence, drop high-corr peers. */
export function filterElitesByCorrelation(
  elites: EliteRow[],
  maximumCorrelation: number,
): { kept: EliteRow[]; rejectedCount: number; maxPairwise: number } {
  const threshold = Number.isFinite(maximumCorrelation)
    ? Math.min(1, Math.max(0, maximumCorrelation))
    : 0.7;
  const ordered = [...elites].sort(
    (left, right) =>
      right.evidence - left.evidence || left.fingerprint.localeCompare(right.fingerprint),
  );
  const kept: EliteRow[] = [];
  let rejectedCount = 0;
  let maxPairwise = 0;
  for (const candidate of ordered) {
    const signature = candidate.equitySignature ?? [];
    if (signature.length < 2) {
      kept.push(candidate);
      continue;
    }
    let blocked = false;
    for (const existing of kept) {
      const other = existing.equitySignature ?? [];
      if (other.length < 2) continue;
      const corr = equitySignatureCorrelation(signature, other);
      maxPairwise = Math.max(maxPairwise, corr);
      if (corr > threshold + 1e-12) {
        blocked = true;
        break;
      }
    }
    if (blocked) rejectedCount += 1;
    else kept.push(candidate);
  }
  let keptMax = 0;
  for (let left = 0; left < kept.length; left += 1) {
    for (let right = left + 1; right < kept.length; right += 1) {
      keptMax = Math.max(
        keptMax,
        equitySignatureCorrelation(kept[left].equitySignature ?? [], kept[right].equitySignature ?? []),
      );
    }
  }
  return {
    kept,
    rejectedCount,
    maxPairwise: kept.length >= 2 ? keptMax : maxPairwise,
  };
}

function equitySignatureCorrelation(left: number[], right: number[]): number {
  const length = Math.min(left.length, right.length);
  if (length < 2) return 0;
  const a = left.slice(0, length);
  const b = right.slice(0, length);
  const meanA = a.reduce((sum, value) => sum + value, 0) / length;
  const meanB = b.reduce((sum, value) => sum + value, 0) / length;
  let covariance = 0;
  let varA = 0;
  let varB = 0;
  for (let index = 0; index < length; index += 1) {
    const da = a[index] - meanA;
    const db = b[index] - meanB;
    covariance += da * db;
    varA += da * da;
    varB += db * db;
  }
  const denom = Math.sqrt(varA * varB);
  if (denom <= Number.EPSILON) return 0;
  return Math.max(0, Math.min(1, Math.max(-1, covariance / denom)));
}

function columnValue(elite: EliteRow, raw: string): number | string | boolean | null {
  const key = canonicalizeFilterColumn(raw);
  const map: Record<string, number | string | boolean | null> = {
    fingerprint: elite.fingerprint,
    strategyId: elite.strategyId,
    entryConditions: elite.entryConditions,
    exitConditions: elite.exitConditions,
    islandId: elite.islandId,
    entryOrder: elite.entryOrder ?? "market",
    management: elite.management ?? "",
    evidence: elite.evidence,
    novelty: elite.novelty,
    trades: elite.trades,
    returnPercent: elite.returnPercent,
    drawdownPercent: elite.drawdownPercent,
    recoveryFactor: elite.recoveryFactor,
    profitFactor: elite.profitFactor,
    sharpeRatio: elite.sharpeRatio,
    isExpectancy: elite.isExpectancy,
    oos1Expectancy: elite.oos1Expectancy,
    oos1ExpectancyRatio: elite.oos1ExpectancyRatio,
    complexity: elite.complexity,
    generation: elite.generation,
    grade: elite.grade,
    parity: elite.parity,
  };
  return map[key] ?? null;
}

function canonicalizeFilterColumn(raw: string): string {
  const key = raw.trim().toLowerCase().replace(/[_\s-]/g, "");
  const aliases: Record<string, string> = {
    fingerprint: "fingerprint",
    strategyid: "strategyId",
    id: "strategyId",
    name: "strategyId",
    entryconditions: "entryConditions",
    exitconditions: "exitConditions",
    islandid: "islandId",
    island: "islandId",
    entryorder: "entryOrder",
    management: "management",
    evidence: "evidence",
    fitness: "evidence",
    novelty: "novelty",
    trades: "trades",
    numberoftrades: "trades",
    returnpercent: "returnPercent",
    return: "returnPercent",
    netprofit: "returnPercent",
    netprofitpct: "returnPercent",
    drawdownpercent: "drawdownPercent",
    drawdown: "drawdownPercent",
    maxdd: "drawdownPercent",
    recoveryfactor: "recoveryFactor",
    rf: "recoveryFactor",
    profitfactor: "profitFactor",
    pf: "profitFactor",
    sharperatio: "sharpeRatio",
    sharpe: "sharpeRatio",
    isexpectancy: "isExpectancy",
    expectancy: "isExpectancy",
    complexity: "complexity",
    generation: "generation",
    grade: "grade",
    parity: "parity",
  };
  const mapped = aliases[key];
  if (!mapped) throw new Error(`unknown column \`${raw}\``);
  return mapped;
}

function compareFilterValues(
  left: number | string | boolean | null,
  op: string,
  right: number | string | boolean | null,
): boolean {
  if (left === null || right === null) {
    if (op === "==" || op === "=") return left === right;
    if (op === "!=") return left !== right;
    return false;
  }
  if (typeof left === "number" && typeof right === "number") {
    switch (op) {
      case ">": return left > right;
      case ">=": return left >= right;
      case "<": return left < right;
      case "<=": return left <= right;
      case "==":
      case "=": return Math.abs(left - right) <= 1e-12;
      case "!=": return Math.abs(left - right) > 1e-12;
    }
  }
  const ls = String(left).toLowerCase();
  const rs = String(right).toLowerCase();
  switch (op) {
    case ">": return ls > rs;
    case ">=": return ls >= rs;
    case "<": return ls < rs;
    case "<=": return ls <= rs;
    case "==":
    case "=": return ls === rs;
    case "!=": return ls !== rs;
  }
  return false;
}

class FilterParser {
  private chars: string[];
  private pos = 0;
  constructor(source: string) {
    this.chars = [...source];
  }
  done() { return this.pos >= this.chars.length; }
  rest() { return this.chars.slice(this.pos).join(""); }
  skipWs() { while (this.pos < this.chars.length && /\s/.test(this.chars[this.pos])) this.pos += 1; }
  parseOr(): DatabankFilterNode {
    const nodes = [this.parseAnd()];
    while (this.consumeKeyword("OR")) nodes.push(this.parseAnd());
    return nodes.length === 1 ? nodes[0]! : { type: "or", children: nodes };
  }
  parseAnd(): DatabankFilterNode {
    const nodes = [this.parseUnary()];
    while (this.consumeKeyword("AND")) nodes.push(this.parseUnary());
    return nodes.length === 1 ? nodes[0]! : { type: "and", children: nodes };
  }
  parseUnary(): DatabankFilterNode {
    this.skipWs();
    if (this.consumeKeyword("NOT")) return { type: "not", child: this.parseUnary() };
    if (this.consumeChar("(")) {
      const inner = this.parseOr();
      this.skipWs();
      if (!this.consumeChar(")")) throw new Error("expected `)`");
      return inner;
    }
    return this.parseCompare();
  }
  parseCompare(): DatabankFilterNode {
    this.skipWs();
    const column = this.parseIdent();
    canonicalizeFilterColumn(column);
    this.skipWs();
    const op = this.parseOp();
    this.skipWs();
    const value = this.parseValue();
    return { type: "cmp", column, op, value };
  }
  parseIdent(): string {
    const start = this.pos;
    if (this.done() || !/[A-Za-z_#]/.test(this.chars[this.pos]!)) throw new Error("expected column name");
    this.pos += 1;
    while (!this.done() && /[A-Za-z0-9_#% ]/.test(this.chars[this.pos]!)) this.pos += 1;
    return this.chars.slice(start, this.pos).join("").trim();
  }
  parseOp(): string {
    for (const token of [">=", "<=", "!=", "==", "=", ">", "<"]) {
      if (this.consumeStr(token)) return token;
    }
    throw new Error("expected comparison operator");
  }
  parseValue(): number | string | boolean | null {
    this.skipWs();
    if (this.done()) throw new Error("expected value");
    const ch = this.chars[this.pos]!;
    if (ch === "'" || ch === '"') return this.parseString(ch);
    if (this.consumeKeyword("TRUE")) return true;
    if (this.consumeKeyword("FALSE")) return false;
    if (this.consumeKeyword("NULL")) return null;
    const start = this.pos;
    if (ch === "-" || ch === "+") this.pos += 1;
    let saw = false;
    while (!this.done() && /[0-9.]/.test(this.chars[this.pos]!)) { saw = true; this.pos += 1; }
    if (!saw) throw new Error("expected number or string");
    return Number(this.chars.slice(start, this.pos).join(""));
  }
  parseString(quote: string): string {
    this.pos += 1;
    const start = this.pos;
    while (!this.done() && this.chars[this.pos] !== quote) this.pos += 1;
    if (this.done()) throw new Error("unterminated string");
    const text = this.chars.slice(start, this.pos).join("");
    this.pos += 1;
    return text;
  }
  consumeKeyword(keyword: string): boolean {
    this.skipWs();
    const end = this.pos + keyword.length;
    const slice = this.chars.slice(this.pos, end).join("");
    if (slice.toUpperCase() !== keyword) return false;
    const next = this.chars[end];
    if (next && /[A-Za-z0-9_]/.test(next)) return false;
    this.pos = end;
    return true;
  }
  consumeStr(token: string): boolean {
    const end = this.pos + token.length;
    if (this.chars.slice(this.pos, end).join("") !== token) return false;
    this.pos = end;
    return true;
  }
  consumeChar(ch: string): boolean {
    if (this.chars[this.pos] !== ch) return false;
    this.pos += 1;
    return true;
  }
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

function comparableRecoveryFactor(elite: EliteRow): number {
  if (elite.recoveryFactor !== null) return elite.recoveryFactor;
  return Number.NEGATIVE_INFINITY;
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

const DEFAULT_ENTRY_WINDOW_START_HOUR = 2;
const DEFAULT_ENTRY_WINDOW_END_HOUR = 19;

/// The end hour is exclusive, so a window ending at 19 stops admitting entries
/// at 19:00 and the label reads 18:59 to match what a trade list shows.
export function entryWindowSummary(
  request: Pick<DiscoverRequest, "entryWindowStartHour" | "entryWindowEndHour">,
): string {
  const start = request.entryWindowStartHour ?? DEFAULT_ENTRY_WINDOW_START_HOUR;
  const end = request.entryWindowEndHour ?? DEFAULT_ENTRY_WINDOW_END_HOUR;
  if (end <= start) {
    return "Entry window is empty: the start hour must be earlier than the end hour.";
  }
  const pad = (hour: number) => String(hour).padStart(2, "0");
  return `Entries allowed ${pad(start)}:00–${pad(end - 1)}:59 broker time.`;
}

export function entryWindowError(
  request: Pick<DiscoverRequest, "entryWindowStartHour" | "entryWindowEndHour">,
): string | null {
  const start = request.entryWindowStartHour ?? DEFAULT_ENTRY_WINDOW_START_HOUR;
  const end = request.entryWindowEndHour ?? DEFAULT_ENTRY_WINDOW_END_HOUR;
  if (start < 0 || start > 23 || end < 1 || end > 24) {
    return "Entry window hours must be 0–23 for the start and 1–24 for the end.";
  }
  if (start >= end) {
    return "Entry window start hour must be earlier than its end hour.";
  }
  return null;
}

type EntryOrderToggles = Pick<
  DiscoverRequest,
  "allowMarketEntries" | "allowStopEntries" | "allowLimitEntries" | "allowStopLimitEntries"
>;

function enabledEntryOrders(request: EntryOrderToggles): string[] {
  return [
    (request.allowMarketEntries ?? true) ? "market" : null,
    request.allowStopEntries ? "stop" : null,
    request.allowLimitEntries ? "limit" : null,
    request.allowStopLimitEntries ? "stop_limit" : null,
  ].filter((value): value is string => value !== null);
}

/// Discover samples the enabled kinds in equal shares, so the summary states the
/// resulting split rather than just listing the checkboxes.
export function entryOrderSummary(request: EntryOrderToggles): string {
  const enabled = enabledEntryOrders(request);
  if (enabled.length === 0) {
    return "No entry order type is enabled, so no candidate can place an order.";
  }
  if (enabled.length === 1) {
    return `Every candidate uses a ${enabled[0]} entry.`;
  }
  const share = Math.round(100 / enabled.length);
  return `Each candidate draws one of ${enabled.join(", ")} — roughly ${share}% each.`;
}

export function entryOrderError(request: EntryOrderToggles): string | null {
  if (enabledEntryOrders(request).length === 0) {
    return "Enable at least one entry order type: market, stop, limit or stop-limit.";
  }
  return null;
}

const DEFAULT_PERTURBATION_FRACTION = 0.2;

/// The form edits whole percent while the request carries a fraction.
export function perturbationPercent(
  request: Pick<DiscoverRequest, "robustnessPerturbationFraction">,
): number {
  const fraction = request.robustnessPerturbationFraction ?? DEFAULT_PERTURBATION_FRACTION;
  return Math.round(fraction * 100);
}

/// The plateau probe jitters every numeric gene by this fraction. Too small and
/// a knife-edge fit survives; too large and a genuinely robust plateau fails.
export function perturbationError(
  request: Pick<DiscoverRequest, "robustnessPerturbationFraction">,
): string | null {
  const fraction = request.robustnessPerturbationFraction ?? DEFAULT_PERTURBATION_FRACTION;
  if (!Number.isFinite(fraction) || fraction < 0.01 || fraction > 1) {
    return "Parameter jitter must be between 1% and 100%.";
  }
  return null;
}

export function conditionLabel(entryConditions: number, exitConditions?: number): string {
  if (exitConditions === undefined) return `${entryConditions} entry`;
  return `${entryConditions}e / ${exitConditions}x`;
}

type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): JsonRecord | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as JsonRecord)
    : null;
}

function numberText(value: unknown): string {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return "?";
  return Number.isInteger(parsed) ? String(parsed) : String(Number(parsed.toFixed(4)));
}

const INDICATOR_LABELS: Record<string, string> = {
  sma: "SMA",
  ema: "EMA",
  wma: "WMA",
  rsi: "RSI",
  atr: "ATR",
  adx: "ADX",
  plus_di: "+DI",
  minus_di: "-DI",
  donchian_high: "Donchian High",
  donchian_low: "Donchian Low",
  highest: "Highest",
  lowest: "Lowest",
  standard_deviation: "StdDev",
  z_score: "Z-Score",
  percentile_in_range: "Percentile",
  rate_of_change: "ROC",
  session_range_high: "Session Range High",
  session_range_low: "Session Range Low",
  body_range_ratio: "Body / Range",
  close_location_in_bar: "Close Location",
  atr_percentile: "ATR Percentile",
  swing_base_zone_high: "Swing Zone High",
  swing_base_zone_low: "Swing Zone Low",
  liquidity_sweep_score: "Liquidity Sweep",
  macd_main: "MACD",
  macd_signal: "MACD Signal",
  macd_histogram: "MACD Histogram",
  bollinger_mid: "Bollinger Mid",
  bollinger_upper: "Bollinger Upper",
  bollinger_lower: "Bollinger Lower",
  bollinger_bandwidth: "Bollinger Bandwidth",
  ichimoku_tenkan: "Tenkan",
  ichimoku_kijun: "Kijun",
  ichimoku_senkou_a: "Senkou A",
  ichimoku_senkou_b: "Senkou B",
  qqe_line: "QQE",
  qqe_trail: "QQE Trail",
  vwap: "VWAP",
  cci: "CCI",
};

const INDICATOR_PARAM_KEYS = [
  "period",
  "atr_period",
  "lookback",
  "fast_period",
  "slow_period",
  "signal_period",
  "rsi_period",
  "smoothing_period",
  "start_hour",
  "range_bars",
  "swing_left",
  "swing_right",
  "base_bars",
  "deviation_tenths",
  "factor_tenths",
] as const;

function titleFromSnake(value: string): string {
  return value
    .split("_")
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(" ");
}

function describeIndicator(node: JsonRecord): string {
  const operator = typeof node.operator === "string" ? node.operator : "indicator";
  const label = INDICATOR_LABELS[operator] ?? titleFromSnake(operator);
  const parts: string[] = [];
  for (const key of INDICATOR_PARAM_KEYS) {
    if (node[key] === undefined || node[key] === null) continue;
    const raw = Number(node[key]);
    if (key === "deviation_tenths") parts.push(`${numberText(raw / 10)} dev`);
    else if (key === "factor_tenths") parts.push(numberText(raw / 10));
    else parts.push(numberText(raw));
  }
  if (typeof node.source === "string" && node.source !== "close") parts.push(node.source);
  const shift = Number(node.shift ?? 1);
  if (Number.isFinite(shift) && shift > 1) parts.push(`shift ${shift}`);
  return parts.length > 0 ? `${label}(${parts.join(", ")})` : label;
}

function describeNumeric(value: unknown): string {
  const node = asRecord(value);
  if (!node) return "?";
  const kind = typeof node.kind === "string" ? node.kind : "";
  if (kind === "constant") return numberText(node.value);
  if (kind === "price") {
    const field = typeof node.field === "string" ? titleFromSnake(node.field) : "Price";
    const shift = Number(node.shift ?? 1);
    return Number.isFinite(shift) && shift > 1 ? `${field}(shift ${shift})` : field;
  }
  if (kind === "context") {
    const context = typeof node.value === "string" ? titleFromSnake(node.value) : "Context";
    return context;
  }
  if (kind === "indicator") {
    const indicator = asRecord(node.value);
    return indicator ? describeIndicator(indicator) : "Indicator";
  }
  return "?";
}

/** Human-readable single condition, e.g. `RSI(14) < 30`. */
export function describeCondition(value: unknown): string {
  const node = asRecord(value);
  if (!node) return "—";
  const operator = typeof node.operator === "string" ? node.operator : "";
  if (operator === "compare") {
    const symbol = node.comparison === "less_than" ? "<" : ">";
    return `${describeNumeric(node.left)} ${symbol} ${describeNumeric(node.right)}`;
  }
  if (operator === "cross_above" || operator === "cross_below") {
    const verb = operator === "cross_above" ? "crosses above" : "crosses below";
    return `${describeNumeric(node.left)} ${verb} ${describeNumeric(node.right)}`;
  }
  if (operator === "between") {
    return `${describeNumeric(node.value)} between ${describeNumeric(node.lower)} and ${describeNumeric(node.upper)}`;
  }
  if (operator === "not") return `NOT ${describeCondition(node.child)}`;
  if (operator === "and" || operator === "or") {
    const children = Array.isArray(node.children) ? node.children : [];
    const joiner = operator === "and" ? " AND " : " OR ";
    return children.map((child) => describeCondition(child)).join(joiner) || "—";
  }
  return "—";
}

function flattenCondition(value: unknown, splitOn: "and" | "or"): string[] {
  const node = asRecord(value);
  if (!node) return [];
  if (node.operator === splitOn && Array.isArray(node.children)) {
    return node.children.flatMap((child) => flattenCondition(child, splitOn));
  }
  const described = describeCondition(node);
  return described === "—" ? [] : [described];
}

export interface StrategyConditionSummary {
  entry: string[];
  exit: string[];
  side: string | null;
  order: string | null;
  stopLoss: string | null;
  takeProfit: string | null;
}

/**
 * Reads the stored strategy IR into the flat entry / exit condition lists the
 * Results detail header shows. Unknown or legacy shapes degrade to empty lists
 * instead of throwing, because the archive is never rewritten.
 */
export function describeStrategyConditions(ir: unknown): StrategyConditionSummary {
  const root = asRecord(ir);
  if (!root) {
    return { entry: [], exit: [], side: null, order: null, stopLoss: null, takeProfit: null };
  }
  const entry = asRecord(root.entry);
  const entryLong = entry ? (entry.long ?? entry.short) : null;
  const exitSource = root.exit_long ?? root.exit ?? root.exit_short;
  const stops = asRecord(root.stops);
  const order = asRecord(entry?.order);
  return {
    entry: flattenCondition(entryLong, "and"),
    exit: flattenCondition(exitSource, "or"),
    side: typeof root.side === "string" ? titleFromSnake(root.side) : null,
    order: order && typeof order.kind === "string" ? titleFromSnake(order.kind) : null,
    stopLoss: describePolicy(stops?.stop_loss),
    takeProfit: describePolicy(stops?.take_profit),
  };
}

function describePolicy(value: unknown): string | null {
  const node = asRecord(value);
  if (!node || typeof node.kind !== "string") return null;
  if (node.kind === "atr_multiple") {
    return `${numberText(node.multiplier)}× ATR(${numberText(node.period)})`;
  }
  if (node.kind === "range_multiple") {
    return `${numberText(node.multiplier)}× Range(${numberText(node.period)})`;
  }
  if (node.kind === "fixed_points") return `${numberText(node.points)} points`;
  if (node.kind === "risk_multiple") return `${numberText(node.multiple)}R`;
  return titleFromSnake(node.kind);
}

/** Compound annual growth rate, or null when the window is too short to annualize. */
export function cagrPercent(
  returnPercent: number,
  startTimestampMs: number,
  endTimestampMs: number,
): number | null {
  const years = (endTimestampMs - startTimestampMs) / (365.2425 * 24 * 60 * 60 * 1000);
  if (!Number.isFinite(years) || years < 0.25) return null;
  const growth = 1 + returnPercent / 100;
  if (growth <= 0) return null;
  const value = (growth ** (1 / years) - 1) * 100;
  return Number.isFinite(value) ? value : null;
}

/** Best-effort symbol name from a bound OHLC export path. */
export function symbolFromDataPath(path: string | null | undefined): string | null {
  if (!path) return null;
  const file = path.split(/[\\/]/).at(-1) ?? "";
  const token = file.split(".")[0]?.split(/[_-]/)[0] ?? "";
  return token.length > 0 ? token.toUpperCase() : null;
}

/** Best-effort decision timeframe token from a bound OHLC export path. */
export function timeframeFromDataPath(path: string | null | undefined): string | null {
  if (!path) return null;
  const file = path.split(/[\\/]/).at(-1) ?? "";
  const token = file.split(".")[0]?.split(/[_-]/)[1] ?? "";
  return /^(M1|M5|M15|M30|H1|H4|D1|W1|MN1)$/i.test(token) ? token.toUpperCase() : null;
}

export function formatDateRange(
  startTimestampMs: number | null,
  endTimestampMs: number | null,
): string {
  if (startTimestampMs === null || endTimestampMs === null) return "—";
  const iso = (value: number) => new Date(value).toISOString().slice(0, 10);
  return `${iso(startTimestampMs)} → ${iso(endTimestampMs)}`;
}
