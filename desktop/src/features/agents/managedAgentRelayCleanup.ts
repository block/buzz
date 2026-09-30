import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
import { loadCommunities } from "@/features/communities/communityStorage";
import {
  listManagedAgentRuntimes,
  reconcileManagedAgentRuntimes,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import type { ManagedAgentRuntimeStatus } from "@/shared/api/types";

type Dependencies = {
  list: () => Promise<ManagedAgentRuntimeStatus[]>;
  reconcile: typeof reconcileManagedAgentRuntimes;
  stop: (pubkey: string, relayUrl: string) => Promise<unknown>;
  configuredRelayUrls: () => string[];
};

const defaultDependencies: Dependencies = {
  list: listManagedAgentRuntimes,
  reconcile: reconcileManagedAgentRuntimes,
  stop: stopManagedAgentRuntime,
  configuredRelayUrls: () =>
    loadCommunities().map((community) => community.relayUrl),
};

async function stopLivePairs(
  runtimes: readonly ManagedAgentRuntimeStatus[],
  shouldStop: (canonicalRelay: string | null) => boolean,
  stop: Dependencies["stop"],
): Promise<void> {
  const pairs = runtimes.filter(
    (runtime) =>
      runtime.lifecycle !== "stopped" &&
      shouldStop(canonicalRelayUrl(runtime.relayUrl)),
  );
  const results = await Promise.allSettled(
    pairs.map((pair) => stop(pair.pubkey, pair.relayUrl)),
  );
  for (const result of results) {
    if (result.status === "rejected") {
      console.warn("[managed-agent-runtimes] stop failed:", result.reason);
    }
  }
}

/**
 * Stop every live agent pair on a relay whose community was removed.
 * Best-effort: failures are logged so community removal still completes.
 */
export async function stopManagedAgentPairsOnRelay(
  relayUrl: string,
  dependencies: Dependencies = defaultDependencies,
): Promise<void> {
  const relay = canonicalRelayUrl(relayUrl);
  if (!relay) return;
  try {
    await stopLivePairs(
      await dependencies.list(),
      (candidate) => candidate === relay,
      dependencies.stop,
    );
  } catch (error) {
    console.warn("[managed-agent-runtimes] list failed:", error);
  }
}

/**
 * Reconcile auto-start pairs, then stop any pair the reconcile started on a
 * relay whose community was removed while it was in flight. Rust cannot be
 * cancelled mid-reconcile, so the result is fenced against the saved list.
 */
export async function reconcileConfiguredManagedAgentRuntimes(
  communities: readonly { relayUrl: string }[],
  dependencies: Dependencies = defaultDependencies,
): Promise<ManagedAgentRuntimeStatus[]> {
  const runtimes = await dependencies.reconcile(communities);
  const configured = new Set(
    dependencies.configuredRelayUrls().map(canonicalRelayUrl),
  );
  await stopLivePairs(
    runtimes,
    (relay) => !configured.has(relay),
    dependencies.stop,
  );
  return runtimes;
}
