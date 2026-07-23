import { classifyRelayClosed } from "@/shared/api/relayClosedPolicy";
import {
  activateRateLimit,
  parseRateLimitHint,
  rateLimitRemainingMs,
} from "@/shared/api/relayRateLimitGate";
import {
  sortEvents,
  type RelaySubscription,
  type RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

const RETRY_BASE_DELAY_MS = 1_000;
const RETRY_MAX_DELAY_MS = 30_000;
export const CLOSED_RECOVERY_EOSE_TIMEOUT_MS = 8_000;

type LiveSubscription = Extract<RelaySubscription, { mode: "live" }>;

function setClosedRecoveryState(
  subscription: LiveSubscription,
  recovering: boolean,
) {
  if (Boolean(subscription.closedRecoveryInProgress) === recovering) return;
  if (!recovering && subscription.closedRecoveryTimeout !== undefined) {
    window.clearTimeout(subscription.closedRecoveryTimeout);
    subscription.closedRecoveryTimeout = undefined;
  }
  subscription.closedRecoveryInProgress = recovering || undefined;
  subscription.onClosedRecoveryStateChange?.(recovering);
}

function armClosedRecoveryTimeout({
  subscriptions,
  subId,
  subscription,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  subscription: LiveSubscription;
}) {
  if (subscription.closedRecoveryTimeout !== undefined) return;
  subscription.closedRecoveryTimeout = window.setTimeout(() => {
    subscription.closedRecoveryTimeout = undefined;
    if (
      subscriptions.get(subId) !== subscription ||
      !subscription.closedRecoveryInProgress
    ) {
      return;
    }
    const error = new Error(
      `Timed out waiting for relay EOSE while recovering subscription ${subId}.`,
    );
    if (subscription.onClosedRecoveryTimeout) {
      subscription.onClosedRecoveryTimeout(error);
      return;
    }
    subscriptions.delete(subId);
    releaseLiveSubscription(subscription);
  }, CLOSED_RECOVERY_EOSE_TIMEOUT_MS);
}

export function resolveReconnectEose(subscription: LiveSubscription) {
  const resolve = subscription.resolveReconnectEose;
  subscription.resolveReconnectEose = undefined;
  resolve?.();
}

export function clearClosedRetry(subscription: LiveSubscription) {
  if (subscription.closedRetryTimeout === undefined) return;
  window.clearTimeout(subscription.closedRetryTimeout);
  subscription.closedRetryTimeout = undefined;
}

export function releaseLiveSubscription(subscription: LiveSubscription) {
  subscription.resolveReady?.();
  subscription.resolveReady = undefined;
  resolveReconnectEose(subscription);
  clearClosedRetry(subscription);
  subscription.closedRetryAttempt = 0;
  setClosedRecoveryState(subscription, false);
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
    // Classify before rejecting so a `rate-limited:` history CLOSED arms the
    // gate for concurrent ops. A history sub can't be retried (the caller holds
    // the promise), so we still reject immediately after arming.
    const closedClass = classifyRelayClosed(message);
    if (closedClass === "rate-limited") {
      const hintSeconds = parseRateLimitHint(message);
      activateRateLimit(hintSeconds);
    }
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

  const closedClass = classifyRelayClosed(message);

  if (closedClass === "terminal") {
    // Auth/access/filter failure — permanently remove the subscription so it
    // doesn't silently loop.
    resolveReconnectEose(subscription);
    clearClosedRetry(subscription);
    setClosedRecoveryState(subscription, false);
    subscriptions.delete(subId);
    return;
  }

  setClosedRecoveryState(subscription, true);
  armClosedRecoveryTimeout({ subscriptions, subId, subscription });
  if (subscription.closedRetryTimeout !== undefined) return;

  const attempt = subscription.closedRetryAttempt ?? 0;
  const backoffMs = Math.min(
    RETRY_BASE_DELAY_MS * 2 ** attempt,
    RETRY_MAX_DELAY_MS,
  );

  let delayMs = backoffMs;

  if (closedClass === "rate-limited") {
    // Activate the gate so concurrent operations back off too.
    const hintSeconds = parseRateLimitHint(message);
    activateRateLimit(hintSeconds);
    // Use the gate's actual remaining time so a shorter hint arriving under a
    // longer active gate does not schedule a premature retry that just gets
    // another CLOSED. The fallback covers the gate-inactive edge case
    // (hint * 1000, or 10s default when no hint).
    const fallbackMs = (hintSeconds ?? 10) * 1_000;
    delayMs = Math.max(backoffMs, rateLimitRemainingMs() || fallbackMs);
  }

  subscription.closedRetryAttempt = attempt + 1;
  subscription.closedRetryTimeout = window.setTimeout(() => {
    subscription.closedRetryTimeout = undefined;
    if (subscriptions.get(subId) !== subscription) return;
    void sendReq(subId, subscription.filter).catch((error) => {
      if (
        subscriptions.get(subId) !== subscription ||
        !subscription.closedRecoveryInProgress
      ) {
        return;
      }
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
  beforeLiveRecoveryComplete,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  closeSubscription: (subId: string) => Promise<void>;
  beforeLiveRecoveryComplete?: () => void;
}) {
  const subscription = subscriptions.get(subId);
  if (!subscription) return;
  if (subscription.mode === "live") {
    if (subscription.closedRecoveryInProgress) {
      beforeLiveRecoveryComplete?.();
    }
    resolveReconnectEose(subscription);
    subscription.resolveReady?.();
    subscription.resolveReady = undefined;
    subscription.closedRetryAttempt = 0;
    clearClosedRetry(subscription);
    setClosedRecoveryState(subscription, false);
    return;
  }
  window.clearTimeout(subscription.timeout);
  subscriptions.delete(subId);
  void closeSubscription(subId);
  subscription.resolve(sortEvents(subscription.events));
}
