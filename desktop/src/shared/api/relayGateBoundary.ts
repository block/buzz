/**
 * Gate-aware relay operation boundaries.
 *
 * History REQ operations are issued here rather than inline in RelayClient so
 * the gate-await + op-timeout pattern lives in one place and the op timeout
 * budget starts only after the rate-limit window has cleared.
 */
import { waitForRateLimit } from "@/shared/api/relayRateLimitGate";
import type {
  RelaySubscription,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

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
      events: [],
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
          : new Error("Failed to request channel history."),
      );
    });
  });
}
