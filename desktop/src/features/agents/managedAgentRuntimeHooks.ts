import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import { clearActiveTurnsForAgent } from "@/features/agents/activeAgentTurnsStore";
import {
  loadActiveCommunityId,
  loadCommunities,
} from "@/features/communities/communityStorage";
import {
  listManagedAgentRuntimes,
  reconcileManagedAgentRuntimes,
  startManagedAgentRuntime,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";
import { canonicalRelayUrl } from "./managedAgentRuntimeStatus";

export const managedAgentRuntimesQueryKey = ["managed-agent-runtimes"] as const;

export function mergeManagedAgentRuntimeStatuses(
  baseline: readonly ManagedAgentRuntimeStatus[] | undefined,
  current: readonly ManagedAgentRuntimeStatus[] | undefined,
  reconciled: readonly ManagedAgentRuntimeStatus[],
): ManagedAgentRuntimeStatus[] {
  const baselineByPair = new Map(
    (baseline ?? []).map((runtime) => [runtimePairKey(runtime), runtime]),
  );
  const currentByPair = new Map(
    (current ?? []).map((runtime) => [runtimePairKey(runtime), runtime]),
  );
  const reconciledPairs = new Set<string>();
  const merged = reconciled.map((runtime) => {
    const key = runtimePairKey(runtime);
    reconciledPairs.add(key);
    const currentRuntime = currentByPair.get(key);
    const baselineRuntime = baselineByPair.get(key);
    // A status event or user action may update this pair while startup
    // reconciliation is discovering others. Only preserve cache rows that
    // changed after the reconcile began; otherwise its result is newer.
    return currentRuntime && currentRuntime !== baselineRuntime
      ? { ...runtime, ...currentRuntime }
      : runtime;
  });

  for (const runtime of current ?? []) {
    if (!reconciledPairs.has(runtimePairKey(runtime))) merged.push(runtime);
  }
  return merged;
}

function runtimePairKey(runtime: ManagedAgentRuntimeStatus): string {
  return JSON.stringify([runtime.pubkey, runtime.relayUrl]);
}

export function cacheReconciledManagedAgentRuntimes(
  queryClient: QueryClient,
  baseline: readonly ManagedAgentRuntimeStatus[] | undefined,
  runtimes: readonly ManagedAgentRuntimeStatus[],
): void {
  queryClient.setQueryData<ManagedAgentRuntimeStatus[]>(
    managedAgentRuntimesQueryKey,
    (current) => mergeManagedAgentRuntimeStatuses(baseline, current, runtimes),
  );
}

/**
 * Bootstrap runtime pairs in every configured community (fire-and-forget).
 *
 * Called after an agent create: the create command spawns only the active
 * community's pair, and the startup reconcile won't run again until the next
 * launch or community switch, so without this kick a brand-new agent stays
 * deaf in every other community. Idempotent — live pairs are skipped and
 * missing ones spawn lazily (warm socket, no LLM until first mention).
 */
export function bootstrapManagedAgentRuntimePairs(
  queryClient: QueryClient,
): void {
  const baseline = queryClient.getQueryData<ManagedAgentRuntimeStatus[]>(
    managedAgentRuntimesQueryKey,
  );
  const communities = loadCommunities().map((community) => ({
    relayUrl: community.relayUrl,
  }));
  void reconcileManagedAgentRuntimes(communities)
    .then((runtimes) => {
      cacheReconciledManagedAgentRuntimes(queryClient, baseline, runtimes);
    })
    .catch((error) => {
      console.warn(
        "[managed-agent-runtimes] post-create reconcile failed:",
        error,
      );
    });
}

export function useManagedAgentRuntimesQuery(options?: { enabled?: boolean }) {
  return useQuery({
    enabled: options?.enabled ?? true,
    queryKey: managedAgentRuntimesQueryKey,
    queryFn: listManagedAgentRuntimes,
  });
}

/**
 * Clear the active community's working badges for an agent when Desktop
 * performs an agent-wide stop or restart that does not go through the
 * pair-scoped `useManagedAgentRuntimeAction` mutation.  Applies the same
 * relay-scope gate: only wipes the store when the active community relay
 * matches `relayUrl`, or — for agent-wide operations with no known relay —
 * when any of the agent's configured pairs is in the active community.
 *
 * Pass `relayUrl` when a specific pair relay is known (preferred).  Omit it
 * (pass null/undefined) for agent-wide operations: the function then clears
 * whenever the active community is configured, since the agent-wide stop
 * affects all pairs, including the one in the active community.
 */
export function clearActiveTurnsForAgentOnStop(
  pubkey: string,
  relayUrl?: string | null,
): void {
  const activeId = loadActiveCommunityId();
  if (!activeId) return;
  const activeCommunity = loadCommunities().find((c) => c.id === activeId);
  if (!activeCommunity) return;

  if (relayUrl != null) {
    // Pair-scoped: only clear when the stopped pair's relay matches the active
    // community.  A mismatch means the stop targets a different community's
    // store — leave it alone.
    const activeCanonical = canonicalRelayUrl(activeCommunity.relayUrl);
    const stoppedCanonical = canonicalRelayUrl(relayUrl);
    if (
      activeCanonical === null ||
      stoppedCanonical === null ||
      activeCanonical !== stoppedCanonical
    ) {
      return;
    }
  }
  // Agent-wide (relayUrl omitted): active community is confirmed to exist, so
  // the stop affects the active pair among others — clear.

  clearActiveTurnsForAgent(pubkey);
}

export function useManagedAgentRuntimeAction() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      action,
      pubkey,
      relayUrl,
    }: {
      action: "start" | "stop" | "restart";
      pubkey: string;
      relayUrl: string;
    }) => {
      if (action === "stop") return stopManagedAgentRuntime(pubkey, relayUrl);
      if (action === "restart") {
        // Split the restart into stop → clear → start so the badge clears at
        // the successful-stop boundary, before the new process is spawned.
        // Using the combined restartManagedAgentRuntime command would clear
        // only in onSuccess, after the new process is already running — too
        // late if start fails (badge stays stale) or if the new harness emits
        // frames before the command resolves (genuine turn would be cleared).
        return stopManagedAgentRuntime(pubkey, relayUrl).then(() => {
          clearActiveTurnsForAgentOnStop(pubkey, relayUrl);
          return startManagedAgentRuntime(pubkey, relayUrl);
        });
      }
      return startManagedAgentRuntime(pubkey, relayUrl);
    },
    onSuccess: (runtime, { action }) => {
      // For stop-only: clear stale working badges immediately.  The restart
      // path already clears at the stop-success boundary inside mutationFn.
      if (action === "stop") {
        clearActiveTurnsForAgentOnStop(runtime.pubkey, runtime.relayUrl);
      }

      queryClient.setQueryData<ManagedAgentRuntimeStatus[]>(
        managedAgentRuntimesQueryKey,
        (current = []) => {
          const index = current.findIndex(
            (candidate) =>
              candidate.pubkey === runtime.pubkey &&
              candidate.relayUrl === runtime.relayUrl,
          );
          if (index === -1) return [...current, runtime];
          return current.map((candidate, candidateIndex) =>
            candidateIndex === index ? runtime : candidate,
          );
        },
      );
    },
  });
}
