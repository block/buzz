import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import { loadCommunities } from "@/features/communities/communityStorage";
import {
  listManagedAgentRuntimes,
  reconcileManagedAgentRuntimes,
  restartManagedAgentRuntime,
  startManagedAgentRuntime,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

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
        return restartManagedAgentRuntime(pubkey, relayUrl);
      }
      return startManagedAgentRuntime(pubkey, relayUrl);
    },
    onSuccess: (runtime) => {
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
