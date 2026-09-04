/**
 * Gate-aware relay operation boundaries.
 *
 * History REQ operations are issued here rather than inline in RelayClient so
 * the gate-await + op-timeout pattern lives in one place and the op timeout
 * budget starts only after the rate-limit window has cleared.
 */
import { collectWithConcurrency } from "@/shared/api/concurrency";
import { AUX_BACKFILL_CHUNK_SIZE } from "@/shared/api/relayChannelFilters";
import { waitForRateLimit } from "@/shared/api/relayRateLimitGate";
import type {
  RelaySubscription,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

const AUX_BACKFILL_CONCURRENCY = 4;

/**
 * Issue a history REQ on `filter`, waiting for any active rate-limit gate
 * before starting the subscription so the op timeout begins only after
 * back-pressure has cleared.
 */
export async function requestHistoryGated(
  subscriptions: Map<string, RelaySubscription>,
  sendRaw: (payload: unknown[]) => Promise<void>,
  closeSubscription: (subId: string) => Promise<void>,
  filter: RelaySubscriptionFilter,
  historyTimeoutMs: number,
): Promise<RelayEvent[]> {
  // Await the gate before issuing REQ; op timeout starts after the wait.
  await waitForRateLimit();

  return new Promise<RelayEvent[]>((resolve, reject) => {
    const subId = `history-${crypto.randomUUID()}`;
    const timeout = window.setTimeout(() => {
      subscriptions.delete(subId);
      void closeSubscription(subId);
      reject(new Error("Timed out while loading channel history."));
    }, historyTimeoutMs);

    subscriptions.set(subId, {
      mode: "history",
      filter,
      events: [],
      resolve,
      reject,
      timeout,
      timeoutMs: historyTimeoutMs,
    });

    void sendRaw(["REQ", subId, filter]).catch((error) => {
      window.clearTimeout(timeout);
      subscriptions.delete(subId);
      reject(
        error instanceof Error
          ? error
          : new Error("Failed to request channel history."),
      );
    });
  });
}

/**
 * Issue a REQ that resolves as soon as its first matching event arrives.
 *
 * This keeps single-event lookups from waiting for EOSE while still resolving
 * `null` when the relay completes an empty result set.
 */
export async function requestFirstEventGated(
  subscriptions: Map<string, RelaySubscription>,
  sendRaw: (payload: unknown[]) => Promise<void>,
  closeSubscription: (subId: string) => Promise<void>,
  filter: RelaySubscriptionFilter,
  historyTimeoutMs: number,
): Promise<RelayEvent | null> {
  await waitForRateLimit();

  return new Promise<RelayEvent | null>((resolve, reject) => {
    const subId = `first-${crypto.randomUUID()}`;
    const timeout = window.setTimeout(() => {
      subscriptions.delete(subId);
      void closeSubscription(subId);
      reject(new Error("Timed out while loading relay event."));
    }, historyTimeoutMs);

    subscriptions.set(subId, {
      mode: "first",
      onEvent: (event) => {
        window.clearTimeout(timeout);
        subscriptions.delete(subId);
        void closeSubscription(subId);
        resolve(event);
      },
      resolve,
      reject,
      timeout,
    });

    void sendRaw(["REQ", subId, filter]).catch((error) => {
      window.clearTimeout(timeout);
      subscriptions.delete(subId);
      reject(
        error instanceof Error
          ? error
          : new Error("Failed to request relay event."),
      );
    });
  });
}

/**
 * Fetch relay history for event IDs in bounded, concurrent chunks and flatten
 * the responses into one list. Returns an empty list without issuing a request
 * when `eventIds` is empty.
 */
export async function fetchChunkedHistory(
  eventIds: string[],
  buildFilter: (eventIds: string[]) => RelaySubscriptionFilter,
  fetchHistory: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>,
): Promise<RelayEvent[]> {
  if (eventIds.length === 0) return [];

  const chunks: string[][] = [];
  for (let i = 0; i < eventIds.length; i += AUX_BACKFILL_CHUNK_SIZE) {
    chunks.push(eventIds.slice(i, i + AUX_BACKFILL_CHUNK_SIZE));
  }
  const batches = await collectWithConcurrency(
    chunks,
    AUX_BACKFILL_CONCURRENCY,
    (ids) => fetchHistory(buildFilter(ids)),
  );
  return batches.flat();
}
