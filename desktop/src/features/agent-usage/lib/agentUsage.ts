//! Frontend-owned local-day boundary construction, bigint-safe token
//! handling, and truthful-state derivation for the NIP-AM local agent usage
//! feature.
//!
//! Rust request validation (`agent_usage.rs::validate_request`) only bounds
//! query span and shape — per M5, the trusted frontend is the single source
//! of local-midnight civil-day construction. Every consumer of
//! `AgentUsageSeriesRequest.bucketBoundaries` must build them here.

import type {
  AgentUsage,
  AgentUsageModel,
  AgentUsageSeries,
  AgentUsageSeriesBucket,
  CostField,
  UsageField,
} from "@/shared/api/tauriArchive";

// ── Local-day boundary construction (M5, A9) ─────────────────────────────────

export type UsageWindowDays = number;

/**
 * A selected usage window. Preset ranges are a trailing day count ending
 * today; a custom range is an explicit inclusive local-date pair chosen in
 * the picker.
 */
export type UsageRange =
  | { kind: "preset"; days: 1 | 7 | 30 }
  | { kind: "custom"; startDate: string; endDate: string };

export const DEFAULT_USAGE_RANGE: UsageRange = { kind: "preset", days: 7 };

/**
 * Largest number of daily buckets a range may cover — one leap year. Mirrors
 * `MAX_BOUNDARIES = 367` (bucket count + 1) in
 * `desktop/src-tauri/src/archive/agent_usage.rs`. The picker clamps to this
 * so the backend's fail-closed arity check is never the UX error path.
 */
export const MAX_RANGE_DAYS = 366;

const DISTINCT_MIDNIGHT_MAX_STEP = 3;

/**
 * The local midnight strictly before `from` (which must itself be a local
 * midnight), found via `Date#setDate` day-arithmetic so ordinary DST
 * transitions land on the correct calendar day. `Date#setDate` normalizes a
 * *nonexistent* local date (a full civil day dropped by a date-line move,
 * e.g. `Pacific/Apia`'s 2011-12-30) forward to the next real one, which can
 * renormalize back to `from` itself — so this widens the step by one
 * calendar day at a time until it actually lands on a distinct instant.
 */
function previousDistinctLocalMidnight(from: Date): Date {
  let probe = from;
  for (let step = 1; step <= DISTINCT_MIDNIGHT_MAX_STEP; step++) {
    probe = new Date(from);
    probe.setDate(probe.getDate() - step);
    probe.setHours(0, 0, 0, 0);
    if (probe.getTime() !== from.getTime()) return probe;
  }
  return probe;
}

/** The local midnight strictly after `from`; see {@link previousDistinctLocalMidnight}. */
function nextDistinctLocalMidnight(from: Date): Date {
  let probe = from;
  for (let step = 1; step <= DISTINCT_MIDNIGHT_MAX_STEP; step++) {
    probe = new Date(from);
    probe.setDate(probe.getDate() + step);
    probe.setHours(0, 0, 0, 0);
    if (probe.getTime() !== from.getTime()) return probe;
  }
  return probe;
}

/**
 * Build `days + 1` exact local-midnight Unix-second boundaries ending at the
 * start of tomorrow's local day, covering the trailing `days` calendar days
 * (today plus `days - 1` prior days).
 *
 * Walks to each boundary's *distinct* local midnight one civil day at a
 * time (never independent `Date#setDate` offsets from one shared base date,
 * and never `N * 86_400`), so boundaries stay correct across DST
 * transitions — including 30-minute offset zones (e.g. Lord Howe Island),
 * where a "day" is 23.5h or 24.5h — and across a skipped local civil date
 * (e.g. `Pacific/Apia`'s 2011 date-line move), where independently offsetting
 * from one base date would normalize the nonexistent date forward and emit
 * a duplicate boundary. A skipped date instead produces one interval
 * spanning the elapsed real time between the two surviving distinct
 * midnights (which can exceed the ordinary 24h, up to the 48h band
 * `validate_request`'s `MAX_INTERVAL_SECS` (A9) admits) rather than a
 * duplicate. `referenceNow` is injectable for deterministic tests and the
 * midnight-rollover timer (M4).
 */
export function buildLocalDayBoundaries(
  days: UsageWindowDays,
  referenceNow: Date = new Date(),
): number[] {
  const todayMidnight = new Date(referenceNow);
  todayMidnight.setHours(0, 0, 0, 0);

  const tomorrowMidnight = nextDistinctLocalMidnight(todayMidnight);

  // Oldest boundary is `days - 1` distinct local midnights before today's;
  // the window covers today plus the (days - 1) preceding calendar days.
  const priorMidnights: Date[] = [];
  let cursor = todayMidnight;
  for (let i = 0; i < days - 1; i++) {
    cursor = previousDistinctLocalMidnight(cursor);
    priorMidnights.push(cursor);
  }
  priorMidnights.reverse();

  return [...priorMidnights, todayMidnight, tomorrowMidnight].map((d) =>
    Math.floor(d.getTime() / 1_000),
  );
}

/**
 * Milliseconds until the next local midnight after `referenceNow`, for the
 * single-`setTimeout` rollover (M4). Recompute and reschedule each time the
 * timer fires — never use `setInterval`, which drifts across DST.
 */
export function msUntilNextLocalMidnight(
  referenceNow: Date = new Date(),
): number {
  const nextMidnight = new Date(referenceNow);
  nextMidnight.setHours(24, 0, 0, 0);
  return nextMidnight.getTime() - referenceNow.getTime();
}

/**
 * Local midnight opening the civil day named by a `YYYY-MM-DD` string.
 * Parsed field-wise into the local zone — never `new Date("YYYY-MM-DD")`,
 * which JS parses as *UTC* midnight and so lands on the previous civil day
 * for every negative-offset zone.
 *
 * Returns `null` for a malformed string or a field triple that isn't a real
 * calendar date (e.g. `2026-02-30`), which `Date` would silently roll forward.
 */
export function parseLocalDate(value: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (match === null) return null;
  const [year, month, day] = [
    Number(match[1]),
    Number(match[2]),
    Number(match[3]),
  ];
  const parsed = new Date(year, month - 1, day);
  parsed.setHours(0, 0, 0, 0);
  // Reject a rolled-forward nonexistent date. A civil date genuinely skipped
  // by a date-line move still normalizes to a different day-of-month, so it
  // is rejected here too rather than silently querying the wrong day.
  if (
    parsed.getFullYear() !== year ||
    parsed.getMonth() !== month - 1 ||
    parsed.getDate() !== day
  ) {
    return null;
  }
  return parsed;
}

/** Local `YYYY-MM-DD` for a date, for round-tripping through the picker's `<input type="date">`. */
export function formatLocalDate(date: Date): string {
  const year = String(date.getFullYear()).padStart(4, "0");
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

/**
 * Number of distinct local civil days in the inclusive `[startDate, endDate]`
 * range, or `null` if either date is malformed or the range is inverted.
 * Counts by walking distinct local midnights, so it agrees exactly with the
 * boundary count {@link buildRangeBoundaries} produces across DST and skipped
 * civil dates. Stops counting past {@link MAX_RANGE_DAYS} so an absurd range
 * can't spin — callers treat an over-cap result as a validation failure.
 */
export function countRangeDays(
  startDate: string,
  endDate: string,
): number | null {
  const start = parseLocalDate(startDate);
  const end = parseLocalDate(endDate);
  if (start === null || end === null) return null;
  if (start.getTime() > end.getTime()) return null;

  let days = 1;
  let cursor = start;
  while (cursor.getTime() < end.getTime()) {
    cursor = nextDistinctLocalMidnight(cursor);
    days += 1;
    if (days > MAX_RANGE_DAYS) return days;
  }
  return days;
}

/**
 * Validation result for a custom range, carrying the human-facing reason so
 * the picker can surface it instead of letting a rejected request surface a
 * raw Rust error string.
 */
export type RangeValidation =
  | { ok: true; days: number }
  | { ok: false; message: string };

/** Validate a custom range against the picker's contract: real dates, ordered, within one year. */
export function validateCustomRange(
  startDate: string,
  endDate: string,
): RangeValidation {
  const start = parseLocalDate(startDate);
  const end = parseLocalDate(endDate);
  if (start === null || end === null) {
    return { ok: false, message: "Enter both a start and an end date." };
  }
  if (start.getTime() > end.getTime()) {
    return { ok: false, message: "Start date must be on or before end date." };
  }
  const days = countRangeDays(startDate, endDate);
  if (days === null) {
    return { ok: false, message: "Enter both a start and an end date." };
  }
  if (days > MAX_RANGE_DAYS) {
    return {
      ok: false,
      message: `Pick a range of ${MAX_RANGE_DAYS} days or fewer.`,
    };
  }
  return { ok: true, days };
}

/**
 * Local-midnight boundaries covering the inclusive civil-day range
 * `[startDate, endDate]` — `days + 1` entries, ending at the midnight that
 * closes `endDate`. Walks distinct local midnights exactly like
 * {@link buildLocalDayBoundaries}, so DST transitions, 30-minute-offset
 * zones, and skipped civil dates behave identically.
 *
 * Returns `[]` for a range that fails {@link validateCustomRange}, so a
 * malformed or over-cap range yields no query rather than a rejected one.
 */
export function buildCustomDayBoundaries(
  startDate: string,
  endDate: string,
): number[] {
  const validation = validateCustomRange(startDate, endDate);
  if (!validation.ok) return [];

  const start = parseLocalDate(startDate);
  const end = parseLocalDate(endDate);
  if (start === null || end === null) return [];

  const midnights: Date[] = [start];
  let cursor = start;
  while (cursor.getTime() < end.getTime()) {
    cursor = nextDistinctLocalMidnight(cursor);
    midnights.push(cursor);
  }
  // Close the final civil day so the last bucket is end-exclusive.
  midnights.push(nextDistinctLocalMidnight(cursor));

  return midnights.map((d) => Math.floor(d.getTime() / 1_000));
}

/**
 * Boundaries for any {@link UsageRange}. The single entry point the query
 * layer uses, so presets and custom ranges cannot diverge in how civil days
 * are constructed.
 */
export function buildRangeBoundaries(
  range: UsageRange,
  referenceNow: Date = new Date(),
): number[] {
  return range.kind === "preset"
    ? buildLocalDayBoundaries(range.days, referenceNow)
    : buildCustomDayBoundaries(range.startDate, range.endDate);
}

/**
 * Human-facing label for the window, used in empty-state and a11y copy.
 * Phrased to read after "for" — "for the last 7 days", "for Jan 1, 2026 –
 * Feb 1, 2026" — so both range kinds fit the same sentence.
 */
export function describeRange(range: UsageRange): string {
  if (range.kind === "preset") {
    return range.days === 1 ? "the last day" : `the last ${range.days} days`;
  }
  return `${formatRangeEndpoint(range.startDate)} – ${formatRangeEndpoint(range.endDate)}`;
}

/** Short, year-bearing display for a custom endpoint; falls back to the raw string if unparseable. */
function formatRangeEndpoint(value: string): string {
  const parsed = parseLocalDate(value);
  if (parsed === null) return value;
  return parsed.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

// ── Bigint-safe token parsing/formatting ─────────────────────────────────────

/**
 * Parse a decimal token-count string to `bigint`, fail-closed. The wire
 * sends token counters as decimal strings specifically so the full valid
 * `u64` range survives the Tauri boundary — never round-trip through
 * `Number(...)`, which loses precision above 2^53.
 *
 * Returns `null` for a null/missing value or a string that isn't a plain
 * non-negative decimal integer (defensive: malformed wire data becomes
 * "unknown", not a thrown parse error that would crash the panel).
 */
export function parseTokenCount(value: string | null): bigint | null {
  if (value === null) return null;
  if (!/^\d+$/.test(value)) return null;
  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

/** Compact display, e.g. `1234` -> "1.2K", `1_000_000` -> "1M". Never lossy for exact copy — use `formatTokenCountExact` for that. */
export function formatTokenCountCompact(value: bigint): string {
  const abs = value < 0n ? -value : value;
  const units: Array<[bigint, string]> = [
    [1_000_000_000n, "B"],
    [1_000_000n, "M"],
    [1_000n, "K"],
  ];
  for (const [threshold, suffix] of units) {
    if (abs >= threshold) {
      // One decimal place, computed in bigint math to stay exact until the
      // final float division (bounded to a single small ratio, not the
      // original magnitude, so no precision loss that matters visually).
      const scaled = Number((value * 10n) / threshold) / 10;
      return `${scaled}${suffix}`;
    }
  }
  return value.toString();
}

/** Exact grouped display, e.g. `1234567` -> "1,234,567". Safe for arbitrary `bigint` magnitude. */
export function formatTokenCountExact(value: bigint): string {
  return value.toLocaleString("en-US");
}

/** Exact USD display, e.g. `1.5` -> "$1.50". `null` callers should render "Estimated" copy elsewhere, never "$0.00". */
export function formatEstimatedCostUsd(value: number): string {
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
  }).format(value);
}

/** Short coverage-date display, e.g. `1737849600` -> "Jan 25". `null` renders "unknown". */
export function formatCoverageDate(unixSeconds: number | null): string {
  if (unixSeconds === null) return "unknown";
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/**
 * Bigint-safe ratio in `[0, 1]` for a relative bar, e.g. `part` tokens against
 * `whole` tokens. Never converts the full magnitude through `Number(...)`;
 * only the final small ratio is a float. Returns `0` when `whole` is zero or
 * negative (guards a divide-by-zero, not a real data case).
 */
export function bigintRatio(part: bigint, whole: bigint): number {
  if (whole <= 0n) return 0;
  const clampedPart = part < 0n ? 0n : part > whole ? whole : part;
  // Scale into an integer permille before the single float division so the
  // division only ever operates on bounded small integers.
  const permille = (clampedPart * 1000n) / whole;
  return Number(permille) / 1000;
}

// ── Display total derivation (A2 presentation layer) ─────────────────────────

/**
 * A provenance-bearing display total for the usage UI. Only one of three
 * states is ever active:
 *
 * - `exact`: `totalTokens.value` is present and parsed. `partial` mirrors the
 *   wire field's `incomplete` flag.
 * - `approximate`: `totalTokens.value` is absent and BOTH `inputTokens` and
 *   `outputTokens` are known; `value` is their bigint-safe sum. `partial` is
 *   `inputTokens.incomplete || outputTokens.incomplete`. Callers MUST render
 *   `≈` to distinguish this from a provider total. A missing category is
 *   unknown, not zero — one-sided i/o yields `unknown`, not `approximate`.
 * - `unknown`: no token counts are available at all; `value` is `null`.
 *
 * This is a *display* value only — it is NEVER written to the wire or stored.
 * NIP-AM's "MUST NOT derive total = input + output" governs published/stored
 * data; this label lives entirely in the presentation layer.
 */
export type DisplayTotal =
  | { kind: "exact"; value: bigint; partial: boolean }
  | { kind: "approximate"; value: bigint; partial: boolean }
  | { kind: "unknown"; value: null; partial: false };

export function deriveDisplayTotal(usage: {
  inputTokens: UsageField;
  outputTokens: UsageField;
  totalTokens: UsageField;
}): DisplayTotal {
  const exact = parseTokenCount(usage.totalTokens.value);
  if (exact !== null) {
    return {
      kind: "exact",
      value: exact,
      partial: isPartialField(usage.totalTokens),
    };
  }
  const input = parseTokenCount(usage.inputTokens.value);
  const output = parseTokenCount(usage.outputTokens.value);
  if (input !== null && output !== null) {
    return {
      kind: "approximate",
      value: input + output,
      partial:
        isPartialField(usage.inputTokens) || isPartialField(usage.outputTokens),
    };
  }
  return { kind: "unknown", value: null, partial: false };
}

// ── Ranking (A2: rank by display total — exact > approximate > unknown) ──────

type DisplayTierKey = 0 | 1 | 2; // 0 = exact, 1 = approximate, 2 = unknown

type RankedWithDisplay<T> = {
  item: T;
  displayTotal: DisplayTotal;
  tierKey: DisplayTierKey;
};

function tierOf(dt: DisplayTotal): DisplayTierKey {
  if (dt.kind === "exact") return 0;
  if (dt.kind === "approximate") return 1;
  return 2;
}

/**
 * Sort items by their display total:
 * 1. Exact totals rank first, descending by value.
 * 2. Approximate totals (≈ in+out) rank next, descending by value.
 * 3. Unknown totals rank last, unordered beyond the tiebreak.
 * Within the same tier and value, `tiebreak` resolves the order.
 */
function rankByDisplayTotal<T>(
  items: readonly T[],
  getUsage: (item: T) => {
    inputTokens: UsageField;
    outputTokens: UsageField;
    totalTokens: UsageField;
  },
  tiebreak: (a: T, b: T) => number,
): T[] {
  const withDisplay: RankedWithDisplay<T>[] = items.map((item) => {
    const dt = deriveDisplayTotal(getUsage(item));
    return { item, displayTotal: dt, tierKey: tierOf(dt) };
  });

  return withDisplay
    .sort((a, b) => {
      if (a.tierKey !== b.tierKey) return a.tierKey - b.tierKey;
      // Same tier — for exact/approximate, sort descending by value.
      if (a.displayTotal.value !== null && b.displayTotal.value !== null) {
        if (a.displayTotal.value !== b.displayTotal.value) {
          return a.displayTotal.value > b.displayTotal.value ? -1 : 1;
        }
      }
      return tiebreak(a.item, b.item);
    })
    .map((ranked) => ranked.item);
}

/** Agents sort by display total (exact → approximate → unknown), descending by value within tier, then normalized pubkey. */
export function sortAgentsByDisplayTotal(
  agents: readonly AgentUsage[],
): AgentUsage[] {
  return rankByDisplayTotal(
    agents,
    (agent) => agent.usage,
    (a, b) => a.agentPubkey.localeCompare(b.agentPubkey),
  );
}

/** Model rows use the same display-total ranking, tiebroken by harness name
 * (null harness sorts last), then by model name (null model sorts last).
 * Ordinal (`<`/`>`) comparators are used so ordering is locale-independent
 * and matches the Rust backend's `String::cmp` byte order. */
export function sortModelsByDisplayTotal(
  models: readonly AgentUsageModel[],
): AgentUsageModel[] {
  return rankByDisplayTotal(
    models,
    (model) => model.usage,
    (a, b) => {
      const harnessCmp =
        a.harness === b.harness
          ? 0
          : a.harness === null
            ? 1
            : b.harness === null
              ? -1
              : a.harness < b.harness
                ? -1
                : 1;
      if (harnessCmp !== 0) return harnessCmp;
      if (a.model === b.model) return 0;
      if (a.model === null) return 1;
      if (b.model === null) return -1;
      return a.model < b.model ? -1 : 1;
    },
  );
}

// ── Coverage / partial-state copy helpers ────────────────────────────────────

/** A field is a "Partial" lower bound when it has a known value that is flagged incomplete. Distinct from fully unknown (`value === null`), which renders as an omitted/unknown state, never zero. */
export function isPartialField(field: UsageField | CostField): boolean {
  return field.value !== null && field.incomplete;
}

/** True when a field has no known value at all — omit from totals/bars, never render as zero. */
export function isUnknownField(field: UsageField | CostField): boolean {
  return field.value === null;
}

/**
 * Compact per-model input breakdown for the focused view: the cache-read,
 * cache-write, and fresh-input subsets of input, each shown only when known
 * (`value !== null`) and never as zero. Returns `null` when no subset is
 * known at all, so the caller omits the line entirely rather than printing
 * three unknowns. A trailing ` · Partial` marks that at least one shown field
 * is a known-but-incomplete lower bound — matching the row's Partial badge and
 * {@link deriveUsageIngressTrailing}'s text convention. Absent (`null`) fields
 * are simply omitted, never marked Partial (they carry no known lower bound).
 */
export function formatModelCacheBreakdown(
  model: AgentUsageModel,
): string | null {
  const { cacheReadTokens, cacheWriteTokens, freshInputTokens } = model.usage;
  const parts: string[] = [];
  const push = (label: string, field: UsageField) => {
    const parsed = parseTokenCount(field.value);
    if (parsed !== null) {
      parts.push(`${label} ${formatTokenCountCompact(parsed)}`);
    }
  };
  push("Cache read", cacheReadTokens);
  push("Cache write", cacheWriteTokens);
  push("Fresh", freshInputTokens);
  if (parts.length === 0) return null;
  const partial =
    isPartialField(cacheReadTokens) ||
    isPartialField(cacheWriteTokens) ||
    isPartialField(freshInputTokens);
  return partial ? `${parts.join(" · ")} · Partial` : parts.join(" · ");
}

/**
 * True when a usage scope has any known cache-read, cache-write, or
 * fresh-input value — the gate for showing the focused view's input-breakdown
 * subsection. When every cache subset is unknown (old-harness data that never
 * reported cache tokens), the subsection is omitted rather than rendering a
 * row of "—", which keeps absence honest without visual noise.
 */
export function hasKnownCacheData(usage: {
  cacheReadTokens: UsageField;
  cacheWriteTokens: UsageField;
  freshInputTokens: UsageField;
}): boolean {
  return (
    !isUnknownField(usage.cacheReadTokens) ||
    !isUnknownField(usage.cacheWriteTokens) ||
    !isUnknownField(usage.freshInputTokens)
  );
}

/**
 * Truthful trailing summary for the profile Info-tab Usage ingress row
 * (plan:328): the viewer's own agent's 7-day known total, `Partial` when
 * incomplete, `Input/output reported` when only those fields are known,
 * or `No recent data` when nothing in the window is known. Never renders
 * the placeholder `"View"` the ingress row used to show unconditionally.
 */
export function deriveUsageIngressTrailing(series: AgentUsageSeries): string {
  if (!series.collectionEnabled) return "Collection off";

  const agent = series.agents[0];
  if (agent === undefined) return "No recent data";

  const { inputTokens, outputTokens, totalTokens } = agent.usage;
  const knownTotal = parseTokenCount(totalTokens.value);
  if (knownTotal !== null) {
    const compact = formatTokenCountCompact(knownTotal);
    return isPartialField(totalTokens) ? `${compact} · Partial` : compact;
  }
  if (
    parseTokenCount(inputTokens.value) !== null ||
    parseTokenCount(outputTokens.value) !== null
  ) {
    const ioPartial =
      isPartialField(inputTokens) || isPartialField(outputTokens);
    return ioPartial
      ? "Input/output reported · Partial"
      : "Input/output reported";
  }
  return "No recent data";
}

/**
 * Aggregate the per-bucket display totals across a daily series into a single
 * provenance-bearing `DisplayTotal` for the overview/focused-view header.
 *
 * Aggregation rules:
 * - `exact`: every report-bearing bucket contributed an exact display total.
 * - `approximate`: at least one bucket contributed an approximate value;
 *   `partial` is the union of contributing buckets' `DisplayTotal.partial`.
 *   Unknown-bucket peers set `partial = true` but do NOT erase the known sum —
 *   the result surfaces a labeled lower bound rather than hiding measured data.
 * - `unknown`: NO report-bearing bucket has any display value at all.
 * - Empty window (no report-bearing buckets): `{ kind: "unknown", value: null, partial: false }`.
 *
 * `partial` reflects i/o and total completeness of contributing buckets.
 * An approximate aggregate with complete i/o and no exact totals carries
 * `partial: false` — total absence alone does NOT trigger partial.
 *
 * The returned value is a *display* value only — never stored or wired.
 */
export function sumKnownBucketTotals(
  buckets: readonly AgentUsageSeriesBucket[],
): DisplayTotal {
  let sumValue = 0n;
  let sawAny = false; // any report-bearing bucket processed
  let anyApprox = false; // at least one approximate bucket contributed a value
  let anyWithValue = false; // at least one bucket contributed a numeric value
  let partial = false;

  for (const bucket of buckets) {
    if (bucket.reportCount === 0) continue;
    sawAny = true;
    const dt = deriveDisplayTotal(bucket.usage);
    if (dt.kind === "exact" || dt.kind === "approximate") {
      sumValue += dt.value;
      anyWithValue = true;
      if (dt.partial) partial = true;
      if (dt.kind === "approximate") anyApprox = true;
    } else {
      // Report-bearing bucket with no display value — sets partial but does NOT
      // erase the sum already accumulated from sibling buckets.
      partial = true;
    }
  }

  if (!sawAny) {
    // Truly empty window — no report-bearing buckets at all.
    return { kind: "unknown", value: null, partial: false };
  }
  if (!anyWithValue) {
    // Report-bearing buckets exist but none had any display value.
    return { kind: "unknown", value: null, partial: false };
  }
  if (anyApprox) {
    return { kind: "approximate", value: sumValue, partial };
  }
  return { kind: "exact", value: sumValue, partial };
}
