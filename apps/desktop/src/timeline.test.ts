import { describe, expect, it } from "vitest";
import { splitTimelinePart, timelinePreset } from "./timeline";

describe("research timeline", () => {
  it("creates an alternating, gap-free schedule", () => {
    const kinds = ["training", "validation", "training", "validation", "holdout"] as const;
    const parts = timelinePreset([...kinds], [30, 10, 30, 10, 20], "2016-01-04", "2024-01-01");
    expect(parts.map((part) => part.kind)).toEqual(kinds);
    expect(parts[0].startDate).toBe("2016-01-04");
    expect(parts.at(-1)?.endDate).toBe("2024-01-01");
    parts.slice(1).forEach((part, index) => {
      expect(Date.parse(part.startDate) - Date.parse(parts[index].endDate)).toBe(86400000);
    });
  });

  it("splits a period without changing the covered window", () => {
    const original = timelinePreset(["training", "holdout"], [80, 20], "2020-01-01", "2024-01-01");
    const split = splitTimelinePart(original, 0);
    expect(split).toHaveLength(3);
    expect(split[0].startDate).toBe(original[0].startDate);
    expect(split[1].endDate).toBe(original[0].endDate);
    expect(split[2]).toEqual(original[1]);
  });

  it("rejects an impossible window", () => {
    expect(timelinePreset(["training", "holdout"], [80, 20], "2024-01-02", "2024-01-01")).toEqual([]);
  });
});
