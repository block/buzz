import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getAgentUsageSeries,
  onAgentMetricsChanged,
  type AgentUsageSeries,
} from "@/shared/api/tauriArchive";
import {
  buildLocalDayBoundaries,
  msUntilNextLocalMidnight,
  type UsageWindowDays,
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
 * Local-day boundaries for the given window that recompute automatically at
 * every local midnight (M4) — no `setInterval` (which would drift across
 * DST), just a single scheduled `setTimeout` that reschedules itself each
 * time it fires. Split out of {@link useAgentUsageSeries} so the rollover
 * mechanics are testable without a `QueryClientProvider`.
 */
export function useLocalDayBoundaries(days: UsageWindowDays): number[] {
  // Bumped once per local midnight so `boundaries` below recomputes even
  // though `days` hasn't changed.
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

  // `rolloverTick` intentionally forces a recompute at local midnight even
  // though it carries no boundary data itself.
  // biome-ignore lint/correctness/useExhaustiveDependencies: rolloverTick drives recompute, not boundary data
  return React.useMemo(
    () => buildLocalDayBoundaries(days),
    [days, rolloverTick],
  );
}

/**
 * Local NIP-AM usage series for the Agents overview or a single agent's
 * profile drill-in. Rebuilds boundaries once per local midnight (M4, no
 * polling) and invalidates on `onAgentMetricsChanged` — new archived
 * metrics, or a kind-44200 subscription toggle — instead of a refetch
 * interval.
 */
export function useAgentUsageSeries({
  agentPubkey,
  days,
  enabled = true,
}: {
  agentPubkey?: string;
  days: UsageWindowDays;
  enabled?: boolean;
}) {
  const queryClient = useQueryClient();
  const boundaries = useLocalDayBoundaries(days);

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
    enabled,
    staleTime: 60_000,
    gcTime: 5 * 60_000,
  });
}
