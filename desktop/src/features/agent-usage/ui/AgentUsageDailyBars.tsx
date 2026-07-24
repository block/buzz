import * as React from "react";

import { cn } from "@/shared/lib/cn";
import type { AgentUsageSeriesBucket } from "@/shared/api/tauriArchive";
import {
  bigintRatio,
  formatTokenCountCompact,
  isPartialField,
  parseTokenCount,
} from "../lib/agentUsage";

const BAR_TRACK_HEIGHT_PX = 56;
const UNKNOWN_BASELINE_HEIGHT_PX = 10;
const KNOWN_MIN_HEIGHT_PX = 3;

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
 */
function deriveBarState(bucket: AgentUsageSeriesBucket) {
  const total = bucket.usage.totalTokens;
  const known = parseTokenCount(total.value);
  const dateLabel = dateLabelOf(bucket.start);

  if (bucket.reportCount === 0) {
    return {
      accessibleLabel: `${dateLabel} · no usage reported`,
      kind: "empty" as const,
      knownTokens: 0n,
    };
  }
  if (known === null) {
    return {
      accessibleLabel: `${dateLabel} · unknown usage`,
      kind: "unknown" as const,
      knownTokens: null,
    };
  }
  const partial = isPartialField(total);
  return {
    accessibleLabel: `${dateLabel} · ${formatTokenCountCompact(known)} reported tokens${
      partial ? " (partial)" : ""
    }`,
    kind: partial ? ("partial" as const) : ("known" as const),
    knownTokens: known,
  };
}

/**
 * CSS-only daily bar row for a usage series (plan:305-306/329). Columns are
 * equal CSS-grid fractions of the container so the row never causes
 * horizontal overflow regardless of window width or bucket count (8 or 31).
 * Each bar carries an accessible `title`/`aria-label` (`date · reported
 * tokens` or `date · unknown usage`) and a visible textual total beneath
 * it; a day with reported-but-uncountable usage renders a fixed hatched
 * baseline, never a zero-height bar.
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
        return total !== null && total > max ? total : max;
      }, 0n),
    [buckets],
  );

  if (buckets.length === 0) return null;

  return (
    <div
      className="grid items-end gap-1"
      data-testid="agent-usage-daily-bars"
      style={{
        gridTemplateColumns: `repeat(${buckets.length}, minmax(0, 1fr))`,
      }}
    >
      {buckets.map((bucket) => (
        <DailyBar
          bucket={bucket}
          key={bucket.start}
          maxKnownTotal={maxKnownTotal}
        />
      ))}
    </div>
  );
}

function DailyBar({
  bucket,
  maxKnownTotal,
}: {
  bucket: AgentUsageSeriesBucket;
  maxKnownTotal: bigint;
}) {
  const { accessibleLabel, kind, knownTokens } = deriveBarState(bucket);

  const knownHeightPx =
    knownTokens !== null && maxKnownTotal > 0n
      ? Math.max(
          Math.round(
            bigintRatio(knownTokens, maxKnownTotal) * BAR_TRACK_HEIGHT_PX,
          ),
          knownTokens > 0n ? KNOWN_MIN_HEIGHT_PX : 1,
        )
      : KNOWN_MIN_HEIGHT_PX;

  const trailingText =
    kind === "unknown"
      ? "—"
      : kind === "partial"
        ? `≥${formatTokenCountCompact(knownTokens ?? 0n)}`
        : formatTokenCountCompact(knownTokens ?? 0n);

  return (
    <div
      className="flex flex-col items-center gap-1"
      data-testid={`agent-usage-daily-bar-${bucket.start}`}
      title={accessibleLabel}
    >
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
              kind === "partial"
                ? "bg-primary/50"
                : kind === "empty"
                  ? "bg-muted/40"
                  : "bg-primary",
            )}
            role="img"
            style={{ height: knownHeightPx }}
          />
        )}
      </div>
      <span className="max-w-full truncate text-2xs text-muted-foreground">
        {trailingText}
      </span>
    </div>
  );
}
