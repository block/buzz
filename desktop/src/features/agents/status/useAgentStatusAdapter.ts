import * as React from "react";
import { useQuery } from "@tanstack/react-query";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import {
  getLatestAgentMetricSnapshots,
  onAgentMetricsChanged,
  type LatestAgentMetricSnapshot,
  type LatestAgentMetricSnapshotsRequest,
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
    contextTimestamp: snapshot.contextTimestamp
      ? parseRfc3339Seconds(snapshot.contextTimestamp)
      : null,
    accountUsageWindows: snapshot.accountUsageWindows.map((window) => ({
      label: window.label,
      usedPercent: window.usedPercent,
      resetAt: window.resetAt ? parseRfc3339Seconds(window.resetAt) : null,
    })),
    accountUsageWindowsTimestamp: snapshot.accountUsageWindowsTimestamp
      ? parseRfc3339Seconds(snapshot.accountUsageWindowsTimestamp)
      : null,
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

export function selectAgentStatusPubkeys(
  snapshots: readonly AgentMetricSnapshot[],
  requestedPubkey?: string | null,
  configuredPubkeys?: readonly string[],
): string[] {
  const requested = requestedPubkey?.trim().toLowerCase();
  if (requested && /^[0-9a-f]{64}$/.test(requested)) return [requested];
  if (requestedPubkey === null) return [];
  const candidates =
    configuredPubkeys ?? snapshots.map((snapshot) => snapshot.agentPubkey);
  return Array.from(
    new Set(
      candidates
        .map((pubkey) => pubkey.trim().toLowerCase())
        .filter((pubkey) => /^[0-9a-f]{64}$/.test(pubkey)),
    ),
  ).slice(0, 64);
}

/** Owner-scoped live query, optionally with exact context-channel scope. */
export function usePermanentAgentStatuses(
  options: {
    agentPubkey?: string | null;
    channelId?: string;
    threadRootId?: string;
  } = {},
): AgentStatusViewModel[] {
  const { agentPubkey, channelId, threadRootId } = options;
  const managedAgents = useManagedAgentsQuery().data;
  const relayAgents = useRelayAgentsQuery().data;
  const configuredAgentNames = React.useMemo(
    () =>
      new Map(
        [...(managedAgents ?? []), ...(relayAgents ?? [])].map((agent) => [
          agent.pubkey.trim().toLowerCase(),
          agent.name,
        ]),
      ),
    [managedAgents, relayAgents],
  );
  const configuredPubkeys = React.useMemo(
    () => Array.from(configuredAgentNames.keys()),
    [configuredAgentNames],
  );
  const targetPubkeys = React.useMemo(
    () => selectAgentStatusPubkeys([], agentPubkey, configuredPubkeys),
    [agentPubkey, configuredPubkeys],
  );
  const normalizedRequest = React.useMemo<LatestAgentMetricSnapshotsRequest>(
    () => ({
      agentPubkeys: targetPubkeys,
      channelId,
      threadRootId,
    }),
    [channelId, targetPubkeys, threadRootId],
  );
  const query = useQuery({
    queryKey: [
      "latest-agent-metric-snapshots",
      targetPubkeys,
      channelId ?? null,
      threadRootId ?? null,
    ],
    queryFn: async () =>
      targetPubkeys.length === 0
        ? []
        : (await getLatestAgentMetricSnapshots(normalizedRequest)).flatMap(
            (snapshot) => {
              const decoded = decodeLatestAgentMetricSnapshot(snapshot);
              return decoded ? [decoded] : [];
            },
          ),
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
    () => selectAgentStatusPubkeys(snapshots, agentPubkey, configuredPubkeys),
    [agentPubkey, configuredPubkeys, snapshots],
  );
  const profiles = useUsersBatchQuery(pubkeys, {
    enabled: pubkeys.length > 0,
  }).data?.profiles;
  const agents = React.useMemo(
    () =>
      pubkeys.map((pubkey) => ({
        id: pubkey,
        label:
          configuredAgentNames.get(pubkey) ??
          resolveUserLabel({
            pubkey,
            profiles,
          }),
        agentPubkey: pubkey,
      })),
    [configuredAgentNames, profiles, pubkeys],
  );

  return useAgentStatusAdapter(agents, snapshots, query.isLoading, error);
}
