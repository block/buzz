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

export type UsageWindowDays = 7 | 30;

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

// ── Ranking (A2: known lower-bound totals rank; null totals list after) ─────

type Ranked<T> = { item: T; totalTokens: bigint | null };

function rankByKnownTotal<T>(
  items: readonly T[],
  totalTokens: (item: T) => UsageField,
  tiebreak: (a: T, b: T) => number,
): T[] {
  const withTotals: Ranked<T>[] = items.map((item) => ({
    item,
    totalTokens: parseTokenCount(totalTokens(item).value),
  }));

  return withTotals
    .sort((a, b) => {
      if (a.totalTokens !== null && b.totalTokens !== null) {
        if (a.totalTokens !== b.totalTokens) {
          return a.totalTokens > b.totalTokens ? -1 : 1;
        }
        return tiebreak(a.item, b.item);
      }
      // Known-total rows rank before unknown-total rows; never interleave.
      if (a.totalTokens !== null) return -1;
      if (b.totalTokens !== null) return 1;
      return tiebreak(a.item, b.item);
    })
    .map((ranked) => ranked.item);
}

/** Agents sort by known `totalTokens` descending, then normalized pubkey (A2/plan). Unknown-total agents list after all known-total agents, unranked among themselves beyond the pubkey tiebreak. */
export function sortAgentsByKnownTotal(
  agents: readonly AgentUsage[],
): AgentUsage[] {
  return rankByKnownTotal(
    agents,
    (agent) => agent.usage.totalTokens,
    (a, b) => a.agentPubkey.localeCompare(b.agentPubkey),
  );
}

/** Model rows use the same ranking rule as agents, tiebroken by model name (`null` model sorts last as "Unknown model"). */
export function sortModelsByKnownTotal(
  models: readonly AgentUsageModel[],
): AgentUsageModel[] {
  return rankByKnownTotal(
    models,
    (model) => model.usage.totalTokens,
    (a, b) => {
      if (a.model === b.model) return 0;
      if (a.model === null) return 1;
      if (b.model === null) return -1;
      return a.model.localeCompare(b.model);
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
 * Bigint-safe sum of each bucket's known `totalTokens` across a daily
 * series, for the overview/focused-view header total. `partial` is true
 * when any bucket has a known-but-incomplete total OR any bucket's total is
 * fully unknown (activity happened somewhere in the window that this exact
 * sum cannot include) — never silently reported as a complete figure.
 * Returns `knownTotal: null` only when every bucket has zero reports (a
 * true empty window, not partial data).
 */
export function sumKnownBucketTotals(
  buckets: readonly AgentUsageSeriesBucket[],
): {
  knownTotal: bigint | null;
  partial: boolean;
} {
  let sum = 0n;
  let sawKnown = false;
  let partial = false;

  for (const bucket of buckets) {
    const total = bucket.usage.totalTokens;
    const known = parseTokenCount(total.value);
    if (known !== null) {
      sum += known;
      sawKnown = true;
    }
    if (isPartialField(total) || (bucket.reportCount > 0 && known === null)) {
      partial = true;
    }
  }

  return { knownTotal: sawKnown ? sum : null, partial };
}
