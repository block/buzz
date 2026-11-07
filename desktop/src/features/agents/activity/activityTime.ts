// Ported from Berd's agent-activity-panel (branch zmarley/agent-activity-panel).
// Builds a monotonic real-time ↔ display-time map that compresses long idle
// gaps between events into short fixed-width display intervals.

export interface TimeGap {
  realStart: number;
  realEnd: number;
  displayStart: number;
  displayEnd: number;
}

export interface TimeMap {
  toDisplay(t: number): number;
  toReal(d: number): number;
  displayDuration: number;
  gaps: TimeGap[];
}

interface NormalizedTimeMapOptions {
  gapThresholdMs: number;
  gapDisplayMs: number;
}

const DEFAULT_GAP_THRESHOLD_MS = 30_000;
const DEFAULT_GAP_DISPLAY_MS = 2_500;

function finiteOrDefault(value: number | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function normalizeOptions(options?: {
  gapThresholdMs?: number;
  gapDisplayMs?: number;
}): NormalizedTimeMapOptions {
  return {
    gapThresholdMs: Math.max(
      0,
      finiteOrDefault(options?.gapThresholdMs, DEFAULT_GAP_THRESHOLD_MS),
    ),
    gapDisplayMs: Math.max(
      0,
      finiteOrDefault(options?.gapDisplayMs, DEFAULT_GAP_DISPLAY_MS),
    ),
  };
}

function upperBound(values: readonly number[], target: number): number {
  let lo = 0;
  let hi = values.length;

  while (lo < hi) {
    const mid = Math.floor((lo + hi) / 2);
    if (values[mid] <= target) lo = mid + 1;
    else hi = mid;
  }

  return lo;
}

function interpolate(
  value: number,
  inStart: number,
  inEnd: number,
  outStart: number,
  outEnd: number,
): number {
  const inputDelta = inEnd - inStart;
  if (inputDelta <= 0) return outStart;
  return outStart + ((value - inStart) / inputDelta) * (outEnd - outStart);
}

export function buildTimeMap(
  sortedEventTimes: number[],
  options?: { gapThresholdMs?: number; gapDisplayMs?: number },
): TimeMap {
  const { gapThresholdMs, gapDisplayMs } = normalizeOptions(options);
  const realTimes = sortedEventTimes.filter(Number.isFinite);

  if (realTimes.length === 0) {
    return {
      toDisplay: () => 0,
      toReal: () => 0,
      displayDuration: 1,
      gaps: [],
    };
  }

  if (realTimes.length === 1) {
    const realTime = realTimes[0];
    return {
      toDisplay: () => 0,
      toReal: () => realTime,
      displayDuration: 1,
      gaps: [],
    };
  }

  const displayTimes = new Array<number>(realTimes.length);
  const gaps: TimeGap[] = [];
  displayTimes[0] = 0;

  for (let index = 1; index < realTimes.length; index += 1) {
    const realStart = realTimes[index - 1];
    const realEnd = realTimes[index];
    const realDelta = Math.max(0, realEnd - realStart);
    const displayStart = displayTimes[index - 1];
    const isGap = realDelta > gapThresholdMs;
    const displayDelta = isGap ? gapDisplayMs : realDelta;
    const displayEnd = displayStart + displayDelta;

    displayTimes[index] = displayEnd;
    if (isGap) {
      gaps.push({ realStart, realEnd, displayStart, displayEnd });
    }
  }

  const lastRealIndex = realTimes.length - 1;
  const lastDisplay = displayTimes[lastRealIndex];
  const displayDuration = Math.max(1, lastDisplay);

  return {
    toDisplay(t: number) {
      if (!Number.isFinite(t)) return t < 0 ? 0 : lastDisplay;
      if (t <= realTimes[0]) return 0;
      if (t >= realTimes[lastRealIndex]) return lastDisplay;

      const leftIndex = Math.max(0, upperBound(realTimes, t) - 1);
      const rightIndex = Math.min(lastRealIndex, leftIndex + 1);
      return interpolate(
        t,
        realTimes[leftIndex],
        realTimes[rightIndex],
        displayTimes[leftIndex],
        displayTimes[rightIndex],
      );
    },

    toReal(d: number) {
      if (!Number.isFinite(d))
        return d < 0 ? realTimes[0] : realTimes[lastRealIndex];
      if (d <= 0) return realTimes[0];
      if (d >= lastDisplay) return realTimes[lastRealIndex];

      const leftIndex = Math.max(0, upperBound(displayTimes, d) - 1);
      const rightIndex = Math.min(lastRealIndex, leftIndex + 1);
      return interpolate(
        d,
        displayTimes[leftIndex],
        displayTimes[rightIndex],
        realTimes[leftIndex],
        realTimes[rightIndex],
      );
    },

    displayDuration,
    gaps,
  };
}
