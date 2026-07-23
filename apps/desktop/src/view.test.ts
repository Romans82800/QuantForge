import { describe, expect, it } from "vitest";
import {
  bindDiscoverTimezone,
  discoverProgress,
  filterAndSortElites,
  selectionBiasTone,
  visibleEliteFingerprint,
} from "./view";
import type { DiscoverRequest, EliteRow } from "./types";

const elite = (overrides: Partial<EliteRow>): EliteRow => ({
  fingerprint: "abc123",
  strategyId: "trend-one",
  family: "trend",
  evidence: 10,
  novelty: 0.5,
  trades: 20,
  returnPercent: 4,
  drawdownPercent: 6,
  profitFactor: 1.4,
  complexity: 8,
  generation: 2,
  grade: "illuminated",
  parity: "unknown",
  ...overrides,
});

describe("databank view rules", () => {
  it("keeps selection-bias severity visible", () => {
    expect(selectionBiasTone("elevated")).toBe("bias-elevated");
  });

  it("filters families and sorts drawdown conservatively", () => {
    const rows = filterAndSortElites(
      [
        elite({ strategyId: "slow", drawdownPercent: 9 }),
        elite({ strategyId: "tight", drawdownPercent: 3 }),
        elite({ strategyId: "other", family: "breakout", drawdownPercent: 1 }),
      ],
      "",
      "trend",
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

  it("sorts by family with evidence as a deterministic tie-breaker", () => {
    const rows = filterAndSortElites(
      [
        elite({ strategyId: "trend-low", family: "trend", evidence: 1 }),
        elite({ strategyId: "breakout", family: "breakout", evidence: 2 }),
        elite({ strategyId: "trend-high", family: "trend", evidence: 3 }),
      ],
      "",
      "all",
      "family",
    );
    expect(rows.map((row) => row.strategyId)).toEqual([
      "breakout",
      "trend-high",
      "trend-low",
    ]);
  });

  it("bounds discovery progress while a job transitions", () => {
    expect(discoverProgress(2, 4)).toBe(50);
    expect(discoverProgress(5, 4)).toBe(100);
    expect(discoverProgress(1, 0)).toBe(0);
  });

  it("never sends two competing timezone authorities", () => {
    const request = {
      metadataPath: "/tmp/export.metadata.csv",
      sourceTimezone: "Etc/UTC",
    } as DiscoverRequest;
    expect(bindDiscoverTimezone(request).sourceTimezone).toBeNull();
  });
});
