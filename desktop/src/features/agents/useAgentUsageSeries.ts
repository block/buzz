import * as React from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getAgentUsageSeries,
  onAgentMetricsChanged,
  type AgentUsageSeries,
  type AgentUsageSeriesRequest,
} from "@/shared/api/tauriArchive";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const agentUsageSeriesQueryKey = (
  agentPubkey: string | null | undefined,
  dayCount: number,
) => ["agent-usage-series", agentPubkey ? normalizePubkey(agentPubkey) : null, dayCount] as const;

/**
 * Build DST-safe local-midnight Unix-second boundaries for `dayCount` buckets.
 * Returns `dayCount + 1` entries (inclusive start / exclusive end per adjacent pair).
 */
export function buildLocalMidnightBucketBoundaries(
  dayCount: number,
  nowMs: number = Date.now(),
): number[] {
  if (!Number.isInteger(dayCount) || dayCount < 1 || dayCount > 30) {
    throw new Error(`dayCount must be 1..30, got ${dayCount}`);
  }
  const end = new Date(nowMs);
  end.setHours(0, 0, 0, 0);
  // Exclusive end is tomorrow's local midnight so "today" is a complete bucket.
  end.setDate(end.getDate() + 1);
  const boundaries: number[] = [];
  for (let i = dayCount; i >= 0; i -= 1) {
    const d = new Date(end);
    d.setDate(end.getDate() - i);
    boundaries.push(Math.floor(d.getTime() / 1000));
  }
  return boundaries;
}

export function useAgentUsageSeries(
  agentPubkey: string | null | undefined,
  options?: { dayCount?: number; enabled?: boolean },
) {
  const dayCount = options?.dayCount ?? 7;
  const enabled = (options?.enabled ?? true) && Boolean(agentPubkey);
  const queryClient = useQueryClient();
  const normalized = agentPubkey ? normalizePubkey(agentPubkey) : null;

  const query = useQuery({
    queryKey: agentUsageSeriesQueryKey(normalized, dayCount),
    enabled,
    queryFn: async (): Promise<AgentUsageSeries> => {
      const request: AgentUsageSeriesRequest = {
        bucketBoundaries: buildLocalMidnightBucketBoundaries(dayCount),
        agentPubkey: normalized ?? undefined,
      };
      return getAgentUsageSeries(request);
    },
    staleTime: 30_000,
  });

  React.useEffect(() => {
    if (!enabled) return;
    return onAgentMetricsChanged(() => {
      void queryClient.invalidateQueries({
        queryKey: ["agent-usage-series"],
      });
    });
  }, [enabled, queryClient]);

  return query;
}
