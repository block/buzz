import { isRetryableRelayClosed } from "@/shared/api/relayClosedPolicy";
import {
  sortEvents,
  type RelaySubscription,
  type RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

const RETRY_BASE_DELAY_MS = 1_000;
const RETRY_MAX_DELAY_MS = 30_000;

type LiveSubscription = Extract<RelaySubscription, { mode: "live" }>;

export function clearClosedRetry(subscription: LiveSubscription) {
  if (subscription.closedRetryTimeout === undefined) return;
  window.clearTimeout(subscription.closedRetryTimeout);
  subscription.closedRetryTimeout = undefined;
}

export function handleRelayClosed({
  subscriptions,
  subId,
  message,
  sendReq,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  message: string;
  sendReq: (subId: string, filter: RelaySubscriptionFilter) => Promise<void>;
}) {
  const subscription = subscriptions.get(subId);
  if (!subscription) return;
  if (subscription.mode === "history") {
    window.clearTimeout(subscription.timeout);
    subscriptions.delete(subId);
    subscription.reject(
      new Error(message || "Relay closed the history subscription."),
    );
    return;
  }
  recoverLiveSubscriptionFromClosed({
    subscriptions,
    subId,
    subscription,
    message,
    sendReq,
  });
}

function recoverLiveSubscriptionFromClosed({
  subscriptions,
  subId,
  subscription,
  message,
  sendReq,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  subscription: LiveSubscription;
  message: string;
  sendReq: (subId: string, filter: RelaySubscriptionFilter) => Promise<void>;
}) {
  subscription.resolveReady?.();
  subscription.resolveReady = undefined;
  if (!isRetryableRelayClosed(message)) {
    subscriptions.delete(subId);
    return;
  }
  if (subscription.closedRetryTimeout !== undefined) return;

  const attempt = subscription.closedRetryAttempt ?? 0;
  const delayMs = Math.min(
    RETRY_BASE_DELAY_MS * 2 ** attempt,
    RETRY_MAX_DELAY_MS,
  );
  subscription.closedRetryAttempt = attempt + 1;
  subscription.closedRetryTimeout = window.setTimeout(() => {
    subscription.closedRetryTimeout = undefined;
    if (subscriptions.get(subId) !== subscription) return;
    void sendReq(subId, subscription.filter).catch((error) => {
      if (subscriptions.get(subId) !== subscription) return;
      console.error("Failed to restore closed relay subscription", error);
      recoverLiveSubscriptionFromClosed({
        subscriptions,
        subId,
        subscription,
        message,
        sendReq,
      });
    });
  }, delayMs);
}

export function prepareSubscriptionEvent(
  subscription: RelaySubscription,
  event: RelayEvent,
) {
  if (subscription.mode === "history") {
    subscription.events.push(event);
    return false;
  }
  subscription.closedRetryAttempt = 0;
  clearClosedRetry(subscription);
  subscription.lastSeenCreatedAt = Math.max(
    subscription.lastSeenCreatedAt ?? 0,
    event.created_at,
  );
  return true;
}

export function handleSubscriptionEose({
  subscriptions,
  subId,
  closeSubscription,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  closeSubscription: (subId: string) => Promise<void>;
}) {
  const subscription = subscriptions.get(subId);
  if (!subscription) return;
  if (subscription.mode === "live") {
    subscription.resolveReady?.();
    subscription.resolveReady = undefined;
    subscription.closedRetryAttempt = 0;
    clearClosedRetry(subscription);
    return;
  }
  window.clearTimeout(subscription.timeout);
  subscriptions.delete(subId);
  void closeSubscription(subId);
  subscription.resolve(sortEvents(subscription.events));
}
