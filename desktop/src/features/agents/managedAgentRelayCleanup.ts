import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
import {
  listManagedAgentRuntimes,
  reconcileManagedAgentRuntimes,
  stopManagedAgentRuntime,
} from "@/shared/api/tauriManagedAgents";
import {
  readdCommunityRelay,
  removeCommunityRelay,
} from "@/shared/api/tauriWorkspace";
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
  if (!relay) return;
  relayRemovals.set(relay, (relayRemovals.get(relay) ?? 0) + 1);
}

/** Native refusal for a start on a relay removed from this device. */
export const RELAY_REMOVED_ERROR = "relay was removed from this device";

/**
 * A start that a relay removal cancelled: a quiet no-op, not a failure.
 * Accepts a thrown error or a returned `spawnError` message.
 */
export function isRelayRemovedError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : error;
  return message === RELAY_REMOVED_ERROR;
}

/**
 * Capture `relayUrl`'s removal counter before a stop/start sequence awaits.
 * The returned check throws `RELAY_REMOVED_ERROR` if that relay was removed
 * since. Call it immediately before the start, with no await in between, so
 * a removal followed by a re-add cannot readmit the stale continuation.
 */
export function captureRelayRemovals(relayUrl: string): () => void {
  // An unparseable relay is never marked removed, so its check never throws.
  const relay = canonicalRelayUrl(relayUrl) ?? "";
  const before = relayRemovals.get(relay) ?? 0;
  return () => {
    if ((relayRemovals.get(relay) ?? 0) !== before) {
      throw new Error(RELAY_REMOVED_ERROR);
    }
  };
}

// Writes to Rust's per-relay admission record run in order, and a reconcile
// waits for them, so a reconcile issued after a re-add is admitted.
let admissionWrites: Promise<void> = Promise.resolve();

// A failed write rejects for its own caller; the queue itself recovers so
// later writes still run.
function queueAdmissionWrite(write: () => Promise<void>): Promise<void> {
  const result = admissionWrites.then(write);
  admissionWrites = result.catch(() => {});
  return result;
}

/** Refuse local pairs on a relay no saved community uses any more. */
export const refuseRelayAdmission = (relayUrl: string) =>
  queueAdmissionWrite(() => removeCommunityRelay(relayUrl));

/** Admit local pairs on the relay of an explicitly re-added community. */
export const readmitRelay = (relayUrl: string) =>
  queueAdmissionWrite(() => readdCommunityRelay(relayUrl));

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
  await admissionWrites;
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
