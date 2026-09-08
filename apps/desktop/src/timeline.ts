import type { DataRangePart } from "./types";

const DAY = 86400000;

export function timelinePreset(
  kinds: DataRangePart["kind"][],
  weights: number[],
  startDate: string,
  endDate: string,
): DataRangePart[] {
  const start = Date.parse(`${startDate}T00:00:00Z`);
  const days = Math.round((Date.parse(`${endDate}T00:00:00Z`) - start) / DAY) + 1;
  if (!Number.isFinite(days) || days < kinds.length || weights.length !== kinds.length || weights.some((weight) => weight <= 0)) return [];
  const total = weights.reduce((sum, weight) => sum + weight, 0);
  let used = 0;
  let cumulative = 0;
  return kinds.map((kind, index) => {
    cumulative += weights[index];
    const next = index === kinds.length - 1
      ? days
      : Math.max(used + 1, Math.min(days - (kinds.length - index - 1), Math.round(days * cumulative / total)));
    const date = (offset: number) => new Date(start + offset * DAY).toISOString().slice(0, 10);
    const part = { id: "", kind, startDate: date(used), endDate: date(next - 1) };
    used = next;
    return part;
  });
}

export function splitTimelinePart(parts: DataRangePart[], index: number): DataRangePart[] {
  const part = parts[index];
  if (!part) return parts;
  const halves = timelinePreset([part.kind, part.kind], [1, 1], part.startDate, part.endDate);
  return halves.length ? [...parts.slice(0, index), ...halves, ...parts.slice(index + 1)] : parts;
}
