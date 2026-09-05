import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import {
  getLatestAgentMetricSnapshots,
  onAgentMetricsChanged,
  type LatestAgentMetricSnapshot,
} from "@/shared/api/tauriArchive";
import { deriveConfiguredAgentStatuses } from "./agentStatusModel";
import type {
  AgentMetricSnapshot,
  AgentStatusViewModel,
  TrackedAgentStatusConfig,
} from "./types";

function configuredPubkey(value: string | undefined): string | undefined {
  const candidate = value?.trim().toLowerCase();
  return candidate && /^[0-9a-f]{64}$/.test(candidate) ? candidate : undefined;
}

/** Stable deployment configuration for the two permanent status slots. */
export const PERMANENT_AGENT_STATUS_CONFIG = [
  {
    id: "victra",
    label: "Victra",
    agentPubkey: configuredPubkey(import.meta.env?.VITE_VICTRA_AGENT_PUBKEY),
  },
  {
    id: "claude",
    label: "Claude",
    agentPubkey: configuredPubkey(import.meta.env?.VITE_CLAUDE_AGENT_PUBKEY),
  },
] satisfies readonly TrackedAgentStatusConfig[];

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
      resetAt: window.resetAt
        ? parseRfc3339Seconds(window.resetAt)
        : null,
    })),
    timestamp,
  };
}

export function useAgentStatusAdapter(
  agents: readonly TrackedAgentStatusConfig[],
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

/** Live native query shared by both permanent desktop/mobile surfaces. */
export function usePermanentAgentStatuses(): AgentStatusViewModel[] {
  const pubkeys = React.useMemo(
    () =>
      PERMANENT_AGENT_STATUS_CONFIG.flatMap((agent) =>
        agent.agentPubkey ? [agent.agentPubkey] : [],
      ),
    [],
  );
  const query = useQuery({
    enabled: pubkeys.length > 0,
    queryKey: ["latest-agent-metric-snapshots", ...pubkeys],
    queryFn: async () =>
      (
        await getLatestAgentMetricSnapshots({ agentPubkeys: pubkeys })
      ).flatMap((snapshot) => {
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
  return useAgentStatusAdapter(
    PERMANENT_AGENT_STATUS_CONFIG,
    query.data ?? [],
    query.isLoading,
    error,
  );
}
