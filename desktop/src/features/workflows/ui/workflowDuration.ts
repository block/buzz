const DURATION_PARTS_PATTERN =
  /^\s*(?:(\d+)\s*w)?\s*(?:(\d+)\s*d)?\s*(?:(\d+)\s*h)?\s*(?:(\d+)\s*m)?\s*(?:(\d+)\s*s)?\s*$/i;

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY = 24 * SECONDS_PER_HOUR;
const SECONDS_PER_WEEK = 7 * SECONDS_PER_DAY;

/** Parse compact durations such as `5s`, `1h 2s`, `2d`, or `3w`. */
export function parseDurationSeconds(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;

  if (/^\d+$/.test(trimmed)) {
    const seconds = Number(trimmed);
    return Number.isSafeInteger(seconds) ? seconds : null;
  }

  const match = DURATION_PARTS_PATTERN.exec(trimmed);
  if (!match || match.slice(1).every((part) => part === undefined)) return null;

  const weeks = Number(match[1] ?? 0);
  const days = Number(match[2] ?? 0);
  const hours = Number(match[3] ?? 0);
  const minutes = Number(match[4] ?? 0);
  const seconds = Number(match[5] ?? 0);
  const total =
    weeks * SECONDS_PER_WEEK +
    days * SECONDS_PER_DAY +
    hours * SECONDS_PER_HOUR +
    minutes * SECONDS_PER_MINUTE +
    seconds;

  return Number.isSafeInteger(total) ? total : null;
}

/** Format whole seconds as a compact duration, omitting empty units. */
export function formatDurationSeconds(totalSeconds: number): string {
  if (!Number.isSafeInteger(totalSeconds) || totalSeconds < 0) return "";
  if (totalSeconds === 0) return "0s";

  const weeks = Math.floor(totalSeconds / SECONDS_PER_WEEK);
  const days = Math.floor((totalSeconds % SECONDS_PER_WEEK) / SECONDS_PER_DAY);
  const hours = Math.floor((totalSeconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR);
  const minutes = Math.floor(
    (totalSeconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE,
  );
  const seconds = totalSeconds % SECONDS_PER_MINUTE;
  const parts: string[] = [];

  if (weeks > 0) parts.push(`${weeks}w`);
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  if (seconds > 0) parts.push(`${seconds}s`);

  return parts.join(" ");
}

function verboseUnit(value: number, unit: string): string {
  return `${value} ${unit}${value === 1 ? "" : "s"}`;
}

/** Format whole seconds with fully spelled-out units for summary UI. */
export function formatDurationSecondsVerbose(totalSeconds: number): string {
  if (!Number.isSafeInteger(totalSeconds) || totalSeconds < 0) return "";
  if (totalSeconds === 0) return "0 seconds";

  const weeks = Math.floor(totalSeconds / SECONDS_PER_WEEK);
  const days = Math.floor((totalSeconds % SECONDS_PER_WEEK) / SECONDS_PER_DAY);
  const hours = Math.floor((totalSeconds % SECONDS_PER_DAY) / SECONDS_PER_HOUR);
  const minutes = Math.floor(
    (totalSeconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE,
  );
  const seconds = totalSeconds % SECONDS_PER_MINUTE;
  const parts: string[] = [];

  if (weeks > 0) parts.push(verboseUnit(weeks, "week"));
  if (days > 0) parts.push(verboseUnit(days, "day"));
  if (hours > 0) parts.push(verboseUnit(hours, "hour"));
  if (minutes > 0) parts.push(verboseUnit(minutes, "minute"));
  if (seconds > 0) parts.push(verboseUnit(seconds, "second"));

  return parts.join(" ");
}

function steppedRange(start: number, end: number, step: number): number[] {
  const values: number[] = [];
  for (let value = start; value <= end; value += step) values.push(value);
  return values;
}

/**
 * Slider stops favor the short delays people use most, then relax precision as
 * the duration grows. The typed field still accepts exact values between stops.
 */
export const DURATION_SLIDER_STOPS = [
  ...steppedRange(1, 120, 1),
  ...steppedRange(125, 600, 5),
  ...steppedRange(615, 1_800, 15),
  ...steppedRange(1_860, 7_200, 60),
  ...steppedRange(7_500, 10_800, 300),
];

export const DEFAULT_DURATION_SECONDS = 1;

export function durationSliderIndex(totalSeconds: number): number {
  if (totalSeconds <= DURATION_SLIDER_STOPS[0]) return 0;

  const lastIndex = DURATION_SLIDER_STOPS.length - 1;
  if (totalSeconds >= DURATION_SLIDER_STOPS[lastIndex]) return lastIndex;

  let low = 0;
  let high = lastIndex;
  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    const value = DURATION_SLIDER_STOPS[middle];
    if (value === totalSeconds) return middle;
    if (value < totalSeconds) low = middle + 1;
    else high = middle - 1;
  }

  return totalSeconds - DURATION_SLIDER_STOPS[high] <=
    DURATION_SLIDER_STOPS[low] - totalSeconds
    ? high
    : low;
}
