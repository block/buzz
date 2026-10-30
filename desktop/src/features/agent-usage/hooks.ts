import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getAgentUsageSeries,
  onAgentMetricsChanged,
  type AgentUsageSeries,
} from "@/shared/api/tauriArchive";
import {
  buildRangeBoundaries,
  DEFAULT_USAGE_RANGE,
  deriveUsageIngressTrailing,
  msUntilNextLocalMidnight,
  type UsageRange,
} from "./lib/agentUsage";

/** Root query key for the whole `agent-usage` family — invalidated en masse on any agent-metric change (M4/A13). */
export const agentUsageQueryKeyRoot = ["agent-usage"] as const;

/**
 * Stable key incorporating the exact boundary set (so a midnight rollover or
 * 7d/30d switch produces a new cache entry) and the optional author filter.
 */
export function agentUsageQueryKey(
  boundaries: readonly number[],
  agentPubkey?: string,
) {
  return [
    ...agentUsageQueryKeyRoot,
    boundaries.join(","),
    agentPubkey ?? null,
  ] as const;
}

/**
 * Local-day boundaries for the given range that recompute automatically at
 * every local midnight (M4) — no `setInterval` (which would drift across
 * DST), just a single scheduled `setTimeout` that reschedules itself each
 * time it fires. Split out of {@link useAgentUsageSeries} so the rollover
 * mechanics are testable without a `QueryClientProvider`.
 *
 * A custom range is anchored to explicit dates, so midnight rollover leaves
 * it unchanged; the timer still runs so a later switch back to a preset is
 * immediately correct.
 */
export function useLocalDayBoundaries(range: UsageRange): number[] {
  // Bumped once per local midnight so `boundaries` below recomputes even
  // though `range` hasn't changed.
  const [rolloverTick, setRolloverTick] = React.useState(0);

  // `rolloverTick` is the only intended dependency: each fire reschedules
  // against a freshly computed `Date.now()`, never a fixed interval that
  // would drift across DST.
  // biome-ignore lint/correctness/useExhaustiveDependencies: rolloverTick is read to reschedule, not to avoid a stale closure
  React.useEffect(() => {
    const timeoutId = setTimeout(() => {
      setRolloverTick((tick) => tick + 1);
    }, msUntilNextLocalMidnight());
    return () => clearTimeout(timeoutId);
  }, [rolloverTick]);

  // Depend on the range's fields rather than the object so a caller passing a
  // fresh literal each render doesn't rebuild boundaries (and refetch) every
  // time. `rolloverTick` intentionally forces a recompute at local midnight
  // even though it carries no boundary data itself.
  const rangeKey = serializeRange(range);
  // biome-ignore lint/correctness/useExhaustiveDependencies: rangeKey stands in for `range`; rolloverTick drives recompute, not boundary data
  return React.useMemo(
    () => buildRangeBoundaries(range),
    [rangeKey, rolloverTick],
  );
}

/** Stable string identity for a range, for memo/query keys. */
function serializeRange(range: UsageRange): string {
  return range.kind === "preset"
    ? `preset:${range.days}`
    : `custom:${range.startDate}:${range.endDate}`;
}

/**
 * Local NIP-AM usage series for the Agents overview or a single agent's
 * profile drill-in. Rebuilds boundaries once per local midnight (M4, no
 * polling) and invalidates on `onAgentMetricsChanged` — new archived
 * metrics, or a kind-44200 subscription toggle — instead of a refetch
 * interval.
 *
 * An invalid custom range produces no boundaries; the query stays disabled
 * rather than issuing a request the backend would reject.
 */
export function useAgentUsageSeries({
  agentPubkey,
  range,
  enabled = true,
}: {
  agentPubkey?: string;
  range: UsageRange;
  enabled?: boolean;
}) {
  const queryClient = useQueryClient();
  const boundaries = useLocalDayBoundaries(range);

  React.useEffect(
    () =>
      onAgentMetricsChanged(() => {
        void queryClient.invalidateQueries({
          queryKey: agentUsageQueryKeyRoot,
        });
      }),
    [queryClient],
  );

  return useQuery<AgentUsageSeries>({
    queryKey: agentUsageQueryKey(boundaries, agentPubkey),
    queryFn: () =>
      getAgentUsageSeries({ bucketBoundaries: boundaries, agentPubkey }),
    enabled: enabled && boundaries.length >= 2,
    staleTime: 60_000,
    gcTime: 5 * 60_000,
  });
}

/**
 * Trailing text for the profile Info tab's usage ingress row (plan:328).
 *
 * Owns its own 7-day window deliberately: the row summarises recent usage
 * independently of the focused view's 7d/30d selector, so the two must not
 * share a query. Returns `undefined` while the query is disabled or still
 * loading, which the row renders as no trailing text at all.
 *
 * `enabled` gates the query off entirely when the row won't render.
 */
export function useUsageIngress(
  agentPubkey: string | null,
  enabled: boolean,
): string | undefined {
  const query = useAgentUsageSeries({
    agentPubkey: agentPubkey ?? undefined,
    range: DEFAULT_USAGE_RANGE,
    enabled,
  });
  return query.data ? deriveUsageIngressTrailing(query.data) : undefined;
}
