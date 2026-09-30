import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
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
};

const defaultDependencies: Dependencies = {
  list: listManagedAgentRuntimes,
  reconcile: reconcileManagedAgentRuntimes,
  stop: stopManagedAgentRuntime,
};

// Per canonical relay URL, how many times its community was removed from this
// device this session. A reconcile compares these before and after its call,
// so only a positive removal — never a storage read — fences its result.
const relayRemovals = new Map<string, number>();

/** Record that the last community on `relayUrl` was removed from this device. */
export function markRelayRemoved(relayUrl: string): void {
  const relay = canonicalRelayUrl(relayUrl);
  if (relay) relayRemovals.set(relay, (relayRemovals.get(relay) ?? 0) + 1);
}

async function stopPairs(
  pairs: readonly ManagedAgentRuntimeStatus[],
  stop: Dependencies["stop"],
): Promise<void> {
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
    const runtimes = await dependencies.list();
    await stopPairs(
      runtimes.filter(
        (runtime) =>
          runtime.lifecycle !== "stopped" &&
          canonicalRelayUrl(runtime.relayUrl) === relay,
      ),
      dependencies.stop,
    );
  } catch (error) {
    console.warn("[managed-agent-runtimes] list failed:", error);
  }
}

/**
 * Reconcile auto-start pairs, then stop any pair the reconcile started on a
 * relay whose community was removed while the call was in flight. Rust cannot
 * be cancelled mid-reconcile, so the result is fenced instead. Returns the
 * relays the fence stopped; they must not be treated as reconciled.
 */
export async function reconcileConfiguredManagedAgentRuntimes(
  communities: readonly { relayUrl: string }[],
  dependencies: Dependencies = defaultDependencies,
): Promise<{
  runtimes: ManagedAgentRuntimeStatus[];
  removedRelays: Set<string>;
}> {
  const before = new Map(relayRemovals);
  const runtimes = await dependencies.reconcile(communities);
  const removedRelays = new Set<string>();
  for (const { relayUrl } of communities) {
    const relay = canonicalRelayUrl(relayUrl);
    if (relay && relayRemovals.get(relay) !== before.get(relay)) {
      removedRelays.add(relay);
    }
  }
  await stopPairs(
    runtimes.filter((runtime) => {
      const relay = canonicalRelayUrl(runtime.relayUrl);
      // `failed` rows have no live child; stopping one only rewrites its
      // record and can fail a concurrent reconcile's `expected_updated_at`.
      return (
        relay !== null &&
        removedRelays.has(relay) &&
        runtime.lifecycle !== "stopped" &&
        runtime.lifecycle !== "failed"
      );
    }),
    dependencies.stop,
  );
  return { runtimes, removedRelays };
}
