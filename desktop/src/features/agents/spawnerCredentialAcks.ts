import type { SpawnerCredentialAck } from "@/shared/api/tauriSpawner";

/**
 * One-shot waiters for spawner credential acks, keyed by spawner pubkey.
 *
 * A `CredentialUpdate` has no world-readable echo to poll (deliberately —
 * credentials never appear in any hash), so confirmation arrives as an
 * encrypted `CredentialAck` frame on the same kind:24201 stream the
 * attestation store already subscribes to. The store delivers decoded acks
 * here; the credential card awaits one with a timeout.
 *
 * Module-level singleton for the same reason as the attestation store — the
 * subscription outlives components — so it is reset in `resetCommunityState()`.
 */

type Waiter = {
  resolve: (ack: SpawnerCredentialAck) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
};

const waiters = new Map<string, Waiter[]>();

/** Deliver a decoded ack to whoever is waiting on this spawner. */
export function deliverSpawnerCredentialAck(
  spawnerPubkey: string,
  ack: SpawnerCredentialAck,
): void {
  const queue = waiters.get(spawnerPubkey);
  const waiter = queue?.shift();
  if (!waiter) return;
  if (queue && queue.length === 0) waiters.delete(spawnerPubkey);
  clearTimeout(waiter.timer);
  waiter.resolve(ack);
}

/** Await the next ack from `spawnerPubkey`, rejecting after `timeoutMs`. */
export function waitForSpawnerCredentialAck(
  spawnerPubkey: string,
  timeoutMs: number,
): Promise<SpawnerCredentialAck> {
  return new Promise((resolve, reject) => {
    const waiter: Waiter = {
      resolve,
      reject,
      timer: setTimeout(() => {
        remove(spawnerPubkey, waiter);
        reject(new Error("The server did not confirm the credential in time."));
      }, timeoutMs),
    };
    const queue = waiters.get(spawnerPubkey) ?? [];
    queue.push(waiter);
    waiters.set(spawnerPubkey, queue);
  });
}

function remove(spawnerPubkey: string, waiter: Waiter): void {
  const queue = waiters.get(spawnerPubkey);
  if (!queue) return;
  const index = queue.indexOf(waiter);
  if (index >= 0) queue.splice(index, 1);
  if (queue.length === 0) waiters.delete(spawnerPubkey);
}

/** Reject and drop every pending waiter. Wired into `resetCommunityState()`. */
export function resetSpawnerCredentialAcks(): void {
  for (const queue of waiters.values()) {
    for (const waiter of queue) {
      clearTimeout(waiter.timer);
      waiter.reject(
        new Error("Community changed before the server confirmed."),
      );
    }
  }
  waiters.clear();
}
