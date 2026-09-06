import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import {
  getLatestAgentMetricSnapshots,
  onAgentMetricsChanged,
  type LatestAgentMetricSnapshot,
} from "@/shared/api/tauriArchive";
import { deriveConfiguredAgentStatuses } from "./agentStatusModel";
import type { AgentMetricSnapshot, AgentStatusViewModel } from "./types";

function parseTokenCount(value: string | null): bigint | null {
  if (value === null || !/^[0-9]+$/.test(value)) return null;
  try {
    return BigInt(value);
  } catch {
    return null;
  }
}

function parseRfc3339Seconds(value: string): number | null {
  const millis = Date.parse(value);
  return Number.isFinite(millis) ? Math.floor(millis / 1_000) : null;
}

/** Convert the exact Tauri wire shape without losing u64 token precision. */
export function decodeLatestAgentMetricSnapshot(
  snapshot: LatestAgentMetricSnapshot,
): AgentMetricSnapshot | null {
  const timestamp = parseRfc3339Seconds(snapshot.timestamp);
  if (timestamp === null) return null;
  return {
    agentPubkey: snapshot.agentPubkey,
    model: snapshot.model,
    harness: snapshot.harness,
    contextUsedTokens: parseTokenCount(snapshot.contextUsedTokens),
    contextLimitTokens: parseTokenCount(snapshot.contextLimitTokens),
    accountUsageWindows: snapshot.accountUsageWindows.map((window) => ({
      label: window.label,
      usedPercent: window.usedPercent,
      resetAt: window.resetAt ? parseRfc3339Seconds(window.resetAt) : null,
    })),
    timestamp,
  };
}

export function useAgentStatusAdapter(
  agents: readonly { agentPubkey: string; id: string; label: string }[],
  snapshots: readonly AgentMetricSnapshot[],
  isLoading = false,
  error: string | null = null,
): AgentStatusViewModel[] {
  return React.useMemo(
    () =>
      deriveConfiguredAgentStatuses({
        agents,
        error,
        isLoading,
        snapshots,
      }),
    [agents, error, isLoading, snapshots],
  );
}

/** Owner-scoped live query for every agent that has published metrics. */
export function usePermanentAgentStatuses(): AgentStatusViewModel[] {
  const query = useQuery({
    queryKey: ["latest-agent-metric-snapshots"],
    queryFn: async () =>
      (await getLatestAgentMetricSnapshots()).flatMap((snapshot) => {
        const decoded = decodeLatestAgentMetricSnapshot(snapshot);
        return decoded ? [decoded] : [];
      }),
    refetchInterval: 5 * 60 * 1_000,
  });

  React.useEffect(
    () => onAgentMetricsChanged(() => void query.refetch()),
    [query.refetch],
  );

  const error = query.error
    ? query.error instanceof Error
      ? query.error.message
      : "Metrics unavailable"
    : null;
  const snapshots = query.data ?? [];
  const pubkeys = React.useMemo(
    () => snapshots.map((snapshot) => snapshot.agentPubkey),
    [snapshots],
  );
  const profiles = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  }).data?.profiles;
  const agents = React.useMemo(
    () =>
      snapshots.map((snapshot) => ({
        id: snapshot.agentPubkey,
        label: resolveUserLabel({
          pubkey: snapshot.agentPubkey,
          profiles,
        }),
        agentPubkey: snapshot.agentPubkey,
      })),
    [profiles, snapshots],
  );

  return useAgentStatusAdapter(agents, snapshots, query.isLoading, error);
}
