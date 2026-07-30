import { describe, expect, it } from "vitest";
import {
  bindDiscoverTimezone,
  cagrPercent,
  describeCondition,
  describeStrategyConditions,
  discoverProgress,
  entryOrderError,
  entryOrderSummary,
  entryWindowError,
  entryWindowSummary,
  filterAndSortElites,
  perturbationError,
  perturbationPercent,
  formatDateRange,
  selectionBiasTone,
  symbolFromDataPath,
  timeframeFromDataPath,
  visibleEliteFingerprint,
} from "./view";
import type { DiscoverRequest, EliteRow } from "./types";

const elite = (overrides: Partial<EliteRow>): EliteRow => ({
  fingerprint: "abc123",
  strategyId: "trend-one",
  entryConditions: 2,
  exitConditions: 1,
  evidence: 10,
  novelty: 0.5,
  trades: 20,
  returnPercent: 4,
  drawdownPercent: 6,
  recoveryFactor: 4 / 6,
  profitFactor: 1.4,
  sharpeRatio: 0.8,
  isExpectancy: 12,
  oos1Expectancy: 9,
  oos1ExpectancyRatio: 0.75,
  complexity: 8,
  generation: 2,
  grade: "illuminated",
  parity: "unknown",
  equitySignature: [0, 1],
  ...overrides,
});

describe("databank view rules", () => {
  it("keeps selection-bias severity visible", () => {
    expect(selectionBiasTone("elevated")).toBe("bias-elevated");
  });

  it("describes the broker-local entry window without claiming it is a failure", () => {
    expect(entryWindowSummary({ entryWindowStartHour: 2, entryWindowEndHour: 23 })).toBe(
      "Entries allowed 02:00–22:59 broker time.",
    );
    expect(entryWindowError({ entryWindowStartHour: 22, entryWindowEndHour: 3 })).toMatch(
      /earlier than its end hour/,
    );
  });

  it("states the resulting entry-order split, including single-kind runs", () => {
    expect(
      entryOrderSummary({ allowMarketEntries: false, allowStopEntries: true, allowLimitEntries: false }),
    ).toBe("Every candidate uses a stop entry.");
    expect(
      entryOrderSummary({ allowMarketEntries: true, allowStopEntries: true, allowLimitEntries: true }),
    ).toBe("Each candidate draws one of market, stop, limit — roughly 33% each.");
    // Market defaults to on, so an untouched form is never empty.
    expect(
      entryOrderError({ allowMarketEntries: null, allowStopEntries: null, allowLimitEntries: null }),
    ).toBeNull();
    expect(
      entryOrderError({ allowMarketEntries: false, allowStopEntries: false, allowLimitEntries: false }),
    ).toMatch(/at least one entry order type/);
  });

  it("edits the parameter jitter in whole percent and rejects useless bands", () => {
    expect(perturbationPercent({ robustnessPerturbationFraction: null })).toBe(20);
    expect(perturbationPercent({ robustnessPerturbationFraction: 0.35 })).toBe(35);
    expect(perturbationError({ robustnessPerturbationFraction: 0 })).toMatch(/between 1% and 100%/);
    expect(perturbationError({ robustnessPerturbationFraction: 1.5 })).toMatch(/between 1% and 100%/);
    expect(perturbationError({ robustnessPerturbationFraction: 0.35 })).toBeNull();
  });

  it("filters by entry conditions and sorts drawdown conservatively", () => {
    const rows = filterAndSortElites(
      [
        elite({ strategyId: "slow", drawdownPercent: 9 }),
        elite({ strategyId: "tight", drawdownPercent: 3 }),
        elite({ strategyId: "other", entryConditions: 3, drawdownPercent: 1 }),
      ],
      "",
      "2",
      "drawdown",
    );
    expect(rows.map((row) => row.strategyId)).toEqual(["tight", "slow"]);
  });

  it("keeps the inspector inside the visible filtered result", () => {
    const visible = [elite({ fingerprint: "trend", strategyId: "trend" })];
    expect(visibleEliteFingerprint(visible, "breakout")).toBe("trend");
    expect(visibleEliteFingerprint(visible, "trend")).toBe("trend");
    expect(visibleEliteFingerprint([], "trend")).toBeNull();
  });

  it("sorts by entry conditions with evidence as a deterministic tie-breaker", () => {
    const rows = filterAndSortElites(
      [
        elite({ strategyId: "two-low", entryConditions: 2, evidence: 1 }),
        elite({ strategyId: "three", entryConditions: 3, evidence: 2 }),
        elite({ strategyId: "two-high", entryConditions: 2, evidence: 3 }),
      ],
      "",
      "all",
      "entryConditions",
    );
    expect(rows.map((row) => row.strategyId)).toEqual([
      "two-high",
      "two-low",
      "three",
    ]);
  });

  it("ranks return-to-drawdown and Sharpe from strongest to weakest", () => {
    const rows = [
      elite({ strategyId: "weak", recoveryFactor: 0.4, sharpeRatio: 0.2 }),
      elite({ strategyId: "strong", recoveryFactor: 1.8, sharpeRatio: 1.1 }),
    ];
    expect(filterAndSortElites(rows, "", "all", "recoveryFactor")[0].strategyId)
      .toBe("strong");
    expect(filterAndSortElites(rows, "", "all", "sharpe")[0].strategyId)
      .toBe("strong");
  });

  it("bounds discovery progress while a job transitions", () => {
    expect(discoverProgress(2, 4)).toBe(50);
    expect(discoverProgress(5, 4)).toBe(100);
    expect(discoverProgress(1, 0)).toBe(0);
    expect(discoverProgress(3, 0, true)).toBe(100);
    expect(discoverProgress(0, 0, true)).toBe(0);
  });

  it("never sends two competing timezone authorities", () => {
    const request = {
      metadataPath: "/tmp/export.metadata.csv",
      sourceTimezone: "Etc/UTC",
    } as DiscoverRequest;
    expect(bindDiscoverTimezone(request).sourceTimezone).toBeNull();
  });
});

const compare = (
  comparison: "greater_than" | "less_than",
  left: unknown,
  right: unknown,
) => ({ operator: "compare", comparison, left, right });

const indicator = (value: Record<string, unknown>) => ({ kind: "indicator", value });
const constant = (value: number) => ({ kind: "constant", value });

describe("strategy IR presentation", () => {
  it("names indicators, periods and comparisons", () => {
    expect(
      describeCondition(
        compare(
          "less_than",
          indicator({ operator: "z_score", source: "close", period: 20, shift: 1 }),
          constant(-1.5),
        ),
      ),
    ).toBe("Z-Score(20) < -1.5");
    expect(
      describeCondition(
        compare(
          "greater_than",
          indicator({ operator: "rsi", source: "close", period: 14, shift: 2 }),
          indicator({ operator: "sma", source: "high", period: 20, shift: 1 }),
        ),
      ),
    ).toBe("RSI(14, shift 2) > SMA(20, high)");
  });

  it("flattens entry AND blocks and exit OR blocks into ordered lists", () => {
    const summary = describeStrategyConditions({
      entry: {
        long: {
          operator: "and",
          children: [
            compare("less_than", indicator({ operator: "rsi", period: 14, shift: 1 }), constant(30)),
            compare("greater_than", indicator({ operator: "adx", period: 14, shift: 1 }), constant(25)),
          ],
        },
        order: { kind: "market" },
      },
      exit_long: {
        operator: "or",
        children: [
          compare("greater_than", indicator({ operator: "z_score", period: 20, shift: 1 }), constant(0)),
        ],
      },
      side: "long_only",
      stops: {
        stop_loss: { kind: "atr_multiple", period: 14, multiplier: 2 },
        take_profit: { kind: "risk_multiple", multiple: 2.5 },
      },
    });
    expect(summary.entry).toEqual(["RSI(14) < 30", "ADX(14) > 25"]);
    expect(summary.exit).toEqual(["Z-Score(20) > 0"]);
    expect(summary.side).toBe("Long Only");
    expect(summary.stopLoss).toBe("2× ATR(14)");
    expect(summary.takeProfit).toBe("2.5R");
  });

  it("degrades to empty condition lists for legacy or unreadable IR", () => {
    expect(describeStrategyConditions(null).entry).toEqual([]);
    expect(describeStrategyConditions({ entry: {} }).exit).toEqual([]);
  });
});

describe("results detail derivations", () => {
  it("annualizes return only over a measurable window", () => {
    const start = Date.UTC(2018, 0, 1);
    const fourYears = Date.UTC(2022, 0, 1);
    expect(cagrPercent(100, start, fourYears)).toBeCloseTo(18.92, 1);
    expect(cagrPercent(50, start, Date.UTC(2018, 1, 1))).toBeNull();
    expect(cagrPercent(-140, start, fourYears)).toBeNull();
  });

  it("reads the universe and timeframe from a bound export path", () => {
    expect(symbolFromDataPath("/data/ICMarkets/EURUSD_H1_2020.csv")).toBe("EURUSD");
    expect(symbolFromDataPath(null)).toBeNull();
    expect(timeframeFromDataPath("/data/ICMarkets/EURUSD_H1_2020.csv")).toBe("H1");
    expect(timeframeFromDataPath("/data/ICMarkets/EURUSD-m15.csv")).toBe("M15");
    expect(timeframeFromDataPath("/data/ICMarkets/EURUSD.csv")).toBeNull();
  });

  it("formats the covered date range from replay timestamps", () => {
    expect(formatDateRange(Date.UTC(2018, 0, 2), Date.UTC(2024, 4, 24)))
      .toBe("2018-01-02 → 2024-05-24");
    expect(formatDateRange(null, 1)).toBe("—");
  });
});
