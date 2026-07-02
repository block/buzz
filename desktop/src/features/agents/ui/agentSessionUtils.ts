import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

export function getToolString(
  record: Record<string, unknown>,
  keys: string[],
): string | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim();
    }
  }
  return null;
}

export function getToolStringList(
  record: Record<string, unknown>,
  keys: string[],
): string[] {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return [value.trim()];
    }
    if (Array.isArray(value)) {
      return value.filter(
        (item): item is string =>
          typeof item === "string" && item.trim().length > 0,
      );
    }
  }
  return [];
}

export function getResultArray(
  resultValue: unknown,
  resultRecord: Record<string, unknown>,
  key: string,
) {
  if (Array.isArray(resultValue)) return resultValue;
  const value = resultRecord[key];
  return Array.isArray(value) ? value : null;
}

export function formatCodeValue(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return value;
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return value;
  }
}

export type ShellToolOutput = {
  exitCode: number | null;
  raw: string;
  stderr: string;
  stdout: string;
  timedOut: boolean;
};

export function parseShellToolOutput(result: string): ShellToolOutput {
  const parsed = parseToolResultValue(result);
  const record = asRecord(parsed);
  const hasShellShape =
    "stdout" in record ||
    "stderr" in record ||
    "exit_code" in record ||
    "exitCode" in record ||
    "timed_out" in record ||
    "timedOut" in record;

  if (!hasShellShape) {
    return {
      exitCode: null,
      raw: typeof parsed === "string" ? parsed : result,
      stderr: "",
      stdout: "",
      timedOut: false,
    };
  }

  return {
    exitCode: getToolNumber(record, ["exit_code", "exitCode"]),
    raw: "",
    stderr: getOptionalString(record, ["stderr"]),
    stdout: getOptionalString(record, ["stdout"]),
    timedOut: getOptionalBoolean(record, ["timed_out", "timedOut"]),
  };
}

export function titleCase(value: string): string {
  return value
    .replace(/[_-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/\b\w/g, (match) => match.toUpperCase());
}

export function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

/**
 * True when a tool image source is an inline `data:image/` URI that should be
 * rendered as-is. This is the dual-layer image-scheme guard: only the
 * `data:image/` prefix is treated as a safe passthrough — every other scheme
 * (including other `data:` subtypes) must be routed through the relay rewriter.
 * Never widen this beyond `data:image/`.
 */
export function isInlineImageData(source: string): boolean {
  return source.startsWith("data:image/");
}

/** Resolve a tool image source for display (inline data URIs or relay URLs). */
export function resolveToolImageSrc(source: string): string {
  return isInlineImageData(source) ? source : rewriteRelayUrl(source);
}

function getToolNumber(
  record: Record<string, unknown>,
  keys: string[],
): number | null {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) {
      return value;
    }
  }
  return null;
}

function getOptionalBoolean(
  record: Record<string, unknown>,
  keys: string[],
): boolean {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "boolean") {
      return value;
    }
  }
  return false;
}

function getOptionalString(
  record: Record<string, unknown>,
  keys: string[],
): string {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") {
      return value;
    }
  }
  return "";
}

/** Format a millisecond duration; negative input yields null. */
export function formatDurationMs(ms: number): string | null {
  if (ms < 0) return null;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) {
    return totalSeconds < 10
      ? `${totalSeconds.toFixed(1)}s`
      : `${Math.round(totalSeconds)}s`;
  }
  let minutes = Math.floor(totalSeconds / 60);
  let seconds = Math.round(totalSeconds % 60);
  if (seconds === 60) {
    minutes += 1;
    seconds = 0;
  }
  return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
}

/**
 * Parse a tool result string into a value. Handles the double-encoding case
 * where a JSON string itself contains JSON. Returns null on empty or invalid
 * input.
 */
export function parseToolResultValue(result: string): unknown {
  const trimmed = result.trim();
  if (!trimmed) return null;

  try {
    const parsed = JSON.parse(trimmed);
    if (typeof parsed !== "string") return parsed;
    try {
      return JSON.parse(parsed);
    } catch {
      return parsed;
    }
  } catch {
    return null;
  }
}

/**
 * Resolve a tool's display duration. Prefers the start/complete timestamps,
 * then falls back to `duration_ms`/`elapsed_ms` fields inside the parsed
 * result payload.
 */
export function getToolDurationDisplay(item: {
  startedAt?: string | null;
  completedAt?: string | null;
  result: string;
}): string | null {
  if (item.startedAt && item.completedAt) {
    return formatDuration(item.startedAt, item.completedAt);
  }

  const resultRecord = asRecord(parseToolResultValue(item.result));
  const durationMs =
    getToolNumber(resultRecord, ["duration_ms", "durationMs"]) ??
    getToolNumber(resultRecord, ["elapsed_ms", "elapsedMs"]);
  return durationMs == null ? null : formatDurationMs(durationMs);
}

export function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

export function shorten(value: string) {
  return value.length > 14
    ? `${value.slice(0, 8)}...${value.slice(-4)}`
    : value;
}

export function shortenMiddle(value: string, maxLength: number) {
  if (value.length <= maxLength) return value;
  const edgeLength = Math.max(4, Math.floor((maxLength - 3) / 2));
  return `${value.slice(0, edgeLength)}...${value.slice(-edgeLength)}`;
}

const transcriptTimeFormat = new Intl.DateTimeFormat("en-US", {
  hour: "numeric",
  hour12: true,
  minute: "2-digit",
});

const transcriptTitleTimeFormat = new Intl.DateTimeFormat(undefined, {
  weekday: "long",
  year: "numeric",
  month: "long",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
  second: "2-digit",
});

export function formatTranscriptTime(isoTimestamp: string): string | null {
  const date = new Date(isoTimestamp);
  if (Number.isNaN(date.getTime())) return null;
  return transcriptTimeFormat.format(date);
}

export function formatTranscriptTimestampTitle(
  isoTimestamp: string,
): string | undefined {
  const date = new Date(isoTimestamp);
  if (Number.isNaN(date.getTime())) return isoTimestamp || undefined;
  return transcriptTitleTimeFormat.format(date);
}

export function formatDuration(
  startIso: string,
  endIso: string,
): string | null {
  if (!startIso || !endIso) return null;
  const start = new Date(startIso).getTime();
  const end = new Date(endIso).getTime();
  if (Number.isNaN(start) || Number.isNaN(end)) return null;
  const ms = end - start;
  if (ms < 0) return null;
  const totalSeconds = ms / 1000;
  if (totalSeconds < 60) {
    return totalSeconds < 10
      ? `${totalSeconds.toFixed(1)}s`
      : `${Math.round(totalSeconds)}s`;
  }
  let minutes = Math.floor(totalSeconds / 60);
  let seconds = Math.round(totalSeconds % 60);
  if (seconds === 60) {
    minutes += 1;
    seconds = 0;
  }
  return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
}

/**
 * Format a live elapsed duration (epoch-ms delta) for a ticking counter.
 * Tiers: `<60s → "Ns"` · `<60m → "Nm Ns"` · `≥60m → "Nh Nm Ns"`.
 * Negative input clamps to 0; carries roll cleanly (e.g. 3600s → "1h 0m 0s").
 */
export function formatElapsed(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) return `${totalMinutes}m ${seconds}s`;
  const minutes = totalMinutes % 60;
  const hours = Math.floor(totalMinutes / 60);
  return `${hours}h ${minutes}m ${seconds}s`;
}
