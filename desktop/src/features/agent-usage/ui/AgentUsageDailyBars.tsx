import * as React from "react";

import { cn } from "@/shared/lib/cn";
import type { AgentUsageSeriesBucket } from "@/shared/api/tauriArchive";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  bigintRatio,
  deriveDisplayTotal,
  formatTokenCountCompact,
  formatTokenCountExact,
  isPartialField,
  parseTokenCount,
} from "../lib/agentUsage";

const BAR_TRACK_HEIGHT_PX = 56;
const UNKNOWN_BASELINE_HEIGHT_PX = 10;
const KNOWN_MIN_HEIGHT_PX = 3;

/**
 * Above this bucket count the per-bar value labels and every date tick stop
 * fitting, so values move into the tooltip only and date ticks thin out to
 * first/last plus regular intervals. 31 buckets (the 30d preset) still fits
 * date ticks at an interval; a custom year-long range does not.
 */
const DENSE_BUCKET_THRESHOLD = 14;

// A hatched, non-zero baseline for "activity happened but the total
// couldn't be counted" — deliberately never a zero-height bar, so unknown
// usage is never visually indistinguishable from a day with no activity
// (plan:306/329: "does not encode unknown as zero").
const UNKNOWN_BAR_STYLE: React.CSSProperties = {
  backgroundImage:
    "repeating-linear-gradient(45deg, var(--muted-foreground) 0, var(--muted-foreground) 1px, transparent 1px, transparent 5px)",
  backgroundColor: "transparent",
  height: UNKNOWN_BASELINE_HEIGHT_PX,
  opacity: 0.35,
};

function dateLabelOf(unixSeconds: number): string {
  return new Date(unixSeconds * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

/**
 * One bar's derived render state, computed from backend truth rather than
 * the field's `value` alone: `reportCount === 0` is a genuine zero-activity
 * day (the field is `null` because nothing happened), which is a different
 * state from `hasUnknownUsage` (activity happened but the total could not
 * be fully counted) even though both leave `usage.totalTokens.value` null.
 *
 * When no genuine total is available but i/o counts are known, the bar falls
 * back to an approx i/o sum (`kind: "approx"`) rather than the hatched
 * unknown baseline — the bar height becomes meaningful and is labeled `≈`.
 */
function deriveBarState(bucket: AgentUsageSeriesBucket) {
  const total = bucket.usage.totalTokens;
  const known = parseTokenCount(total.value);
  const dateLabel = dateLabelOf(bucket.start);

  if (bucket.reportCount === 0) {
    return {
      accessibleLabel: `${dateLabel} · no usage reported`,
      dateLabel,
      kind: "empty" as const,
      knownTokens: 0n,
    };
  }
  if (known !== null) {
    const partial = isPartialField(total);
    return {
      accessibleLabel: `${dateLabel} · ${formatTokenCountCompact(known)} reported tokens${
        partial ? " (partial)" : ""
      }`,
      dateLabel,
      kind: partial ? ("partial" as const) : ("known" as const),
      knownTokens: known,
    };
  }
  // Genuine total unknown — derive the display total for the bar.
  const dt = deriveDisplayTotal(bucket.usage);
  if (dt.kind === "approximate") {
    return {
      accessibleLabel: `${dateLabel} · ≈ ${formatTokenCountCompact(dt.value)} tokens${
        dt.partial ? " (partial)" : ""
      }`,
      dateLabel,
      kind: dt.partial ? ("approx-partial" as const) : ("approx" as const),
      knownTokens: dt.value,
    };
  }
  return {
    accessibleLabel: `${dateLabel} · unknown usage`,
    dateLabel,
    kind: "unknown" as const,
    knownTokens: null,
  };
}

/**
 * The compact value rendered directly on the bar. Carries the same
 * provenance markers the header uses: `≈` for an in+out approximation, `≥`
 * for a known-but-incomplete lower bound, a trailing `*` when partial, and
 * `—` when nothing is countable.
 */
function barValueText(
  kind: ReturnType<typeof deriveBarState>["kind"],
  knownTokens: bigint | null,
): string {
  if (kind === "unknown") return "—";
  const compact = formatTokenCountCompact(knownTokens ?? 0n);
  switch (kind) {
    case "partial":
      return `≥${compact}`;
    case "approx-partial":
      return `≈${compact}*`;
    case "approx":
      return `≈${compact}`;
    default:
      return compact;
  }
}

/**
 * Exact per-field breakdown for the hover tooltip, so total/input/output are
 * legible without opening the focused view. Each field is reported
 * independently: a null field renders "unknown", never zero, and the total
 * keeps its exact/≈/unknown provenance rather than being derived from the
 * i/o pair shown beside it.
 */
function barBreakdown(bucket: AgentUsageSeriesBucket): {
  total: string;
  input: string;
  output: string;
} {
  const dt = deriveDisplayTotal(bucket.usage);
  const total =
    dt.kind === "exact"
      ? formatTokenCountExact(dt.value)
      : dt.kind === "approximate"
        ? `≈ ${formatTokenCountExact(dt.value)}`
        : "unknown";
  const totalSuffix = dt.kind !== "unknown" && dt.partial ? " (partial)" : "";

  const field = (f: { value: string | null; incomplete: boolean }): string => {
    const parsed = parseTokenCount(f.value);
    if (parsed === null) return "unknown";
    return `${formatTokenCountExact(parsed)}${isPartialField(f) ? " (partial)" : ""}`;
  };

  return {
    total: `${total}${totalSuffix}`,
    input: field(bucket.usage.inputTokens),
    output: field(bucket.usage.outputTokens),
  };
}

/**
 * CSS-only daily bar chart for a usage series (plan:305-306/329). Columns are
 * equal CSS-grid fractions of the container so the chart never causes
 * horizontal overflow regardless of window width or bucket count (2, 8, 31,
 * or a custom range up to a year).
 *
 * Each bar renders its token value directly on the bar, labels the **date**
 * beneath it on the x-axis, and exposes a hover tooltip with the exact
 * total/input/output breakdown so the split is readable without opening the
 * focused view. Accessible `aria-label`s carry the same `date · value` truth
 * for screen readers. A day with reported-but-uncountable usage renders a
 * fixed hatched baseline, never a zero-height bar.
 *
 * Dense ranges (more than {@link DENSE_BUCKET_THRESHOLD} buckets) drop the
 * on-bar value text, which cannot fit legibly, and thin the date ticks to
 * first, last, and a regular interval. The tooltip still carries every bar's
 * full breakdown, so no information is lost.
 */
export function AgentUsageDailyBars({
  buckets,
}: {
  buckets: AgentUsageSeriesBucket[];
}) {
  const maxKnownTotal = React.useMemo(
    () =>
      buckets.reduce<bigint>((max, bucket) => {
        const total = parseTokenCount(bucket.usage.totalTokens.value);
        if (total !== null) return total > max ? total : max;
        // Fall back to the display total's approximate value so bars scale
        // correctly when no bucket reports a genuine total.
        const dt = deriveDisplayTotal(bucket.usage);
        return dt.kind === "approximate" && dt.value > max ? dt.value : max;
      }, 0n),
    [buckets],
  );

  if (buckets.length === 0) return null;

  const dense = buckets.length > DENSE_BUCKET_THRESHOLD;
  // At most ~7 date ticks in a dense range; first and last are always shown.
  const tickInterval = dense ? Math.ceil(buckets.length / 7) : 1;

  return (
    <div
      className="grid items-end gap-1"
      data-testid="agent-usage-daily-bars"
      style={{
        gridTemplateColumns: `repeat(${buckets.length}, minmax(0, 1fr))`,
      }}
    >
      {buckets.map((bucket, index) => (
        <DailyBar
          bucket={bucket}
          key={bucket.start}
          maxKnownTotal={maxKnownTotal}
          showDateLabel={
            !dense ||
            index === 0 ||
            index === buckets.length - 1 ||
            index % tickInterval === 0
          }
          showValueLabel={!dense}
        />
      ))}
    </div>
  );
}

function DailyBar({
  bucket,
  maxKnownTotal,
  showDateLabel,
  showValueLabel,
}: {
  bucket: AgentUsageSeriesBucket;
  maxKnownTotal: bigint;
  showDateLabel: boolean;
  showValueLabel: boolean;
}) {
  const { accessibleLabel, dateLabel, kind, knownTokens } =
    deriveBarState(bucket);
  const breakdown = barBreakdown(bucket);

  const knownHeightPx =
    knownTokens !== null && maxKnownTotal > 0n
      ? Math.max(
          Math.round(
            bigintRatio(knownTokens, maxKnownTotal) * BAR_TRACK_HEIGHT_PX,
          ),
          knownTokens > 0n ? KNOWN_MIN_HEIGHT_PX : 1,
        )
      : KNOWN_MIN_HEIGHT_PX;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <div
          className="flex cursor-default flex-col items-center gap-1"
          data-testid={`agent-usage-daily-bar-${bucket.start}`}
        >
          {showValueLabel ? (
            <span
              className="max-w-full truncate text-2xs text-muted-foreground"
              data-testid={`agent-usage-daily-bar-value-${bucket.start}`}
            >
              {barValueText(kind, knownTokens)}
            </span>
          ) : null}
          <div
            className="flex w-full items-end justify-center"
            style={{ height: BAR_TRACK_HEIGHT_PX }}
          >
            {kind === "unknown" ? (
              <div
                aria-label={accessibleLabel}
                className="w-full rounded-t-sm"
                role="img"
                style={UNKNOWN_BAR_STYLE}
              />
            ) : (
              <div
                aria-label={accessibleLabel}
                className={cn(
                  "w-full rounded-t-sm",
                  kind === "partial" || kind === "approx-partial"
                    ? "bg-primary/50"
                    : kind === "approx"
                      ? "bg-primary/70"
                      : kind === "empty"
                        ? "bg-muted/40"
                        : "bg-primary",
                )}
                role="img"
                style={{ height: knownHeightPx }}
              />
            )}
          </div>
          {/* Non-breaking space holds the tick row's height when a dense
              range hides this bar's date, so bars stay baseline-aligned. */}
          <span
            className="max-w-full truncate text-2xs text-muted-foreground"
            data-testid={`agent-usage-daily-bar-date-${bucket.start}`}
          >
            {showDateLabel ? dateLabel : "\u00A0"}
          </span>
        </div>
      </TooltipTrigger>
      <TooltipContent
        data-testid={`agent-usage-daily-bar-tooltip-${bucket.start}`}
      >
        <span className="block font-medium">{dateLabel}</span>
        <span className="block">Total: {breakdown.total}</span>
        <span className="block">Input: {breakdown.input}</span>
        <span className="block">Output: {breakdown.output}</span>
      </TooltipContent>
    </Tooltip>
  );
}
