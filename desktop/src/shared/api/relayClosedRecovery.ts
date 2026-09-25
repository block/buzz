import { classifyRelayClosed } from "@/shared/api/relayClosedPolicy";
import { isQueryDeadlineError } from "@/shared/lib/relayError";
import {
  activateRateLimit,
  parseRateLimitHint,
  rateLimitRemainingMs,
} from "@/shared/api/relayRateLimitGate";
import {
  sortEvents,
  type RelaySubscription,
  type RelaySubscriptionFilter,
  type SubscriptionEventBufferItem,
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
  closeSubscription,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  message: string;
  sendReq: (subId: string, filter: RelaySubscriptionFilter) => Promise<void>;
  closeSubscription?: (subId: string) => Promise<void>;
}) {
  const subscription = subscriptions.get(subId);
  if (!subscription) return;
  if (subscription.mode !== "live") {
    // Classify before acting so a `rate-limited:` CLOSED arms the gate for
    // concurrent ops regardless of whether this specific sub can be retried.
    const closedClass = classifyRelayClosed(message);
    if (closedClass === "rate-limited") {
      const hintSeconds = parseRateLimitHint(message);
      activateRateLimit(hintSeconds);

      // History subs hold a promise the caller is awaiting. Rather than
      // rejecting immediately, defer and re-issue the REQ after the
      // rate-limit window so the caller transparently receives their result.
      // Bounded to 3 attempts — if the relay keeps refusing, fall through
      // to the permanent reject so the caller's promise resolves with an error
      // rather than waiting forever.
      if (subscription.mode === "history") {
        const attempt = subscription.closedRetryAttempt ?? 0;
        if (attempt < 3) {
          subscription.closedRetryAttempt = attempt + 1;
          // The same cancellable timeout owns admission waiting and, only once
          // dispatched, the response budget. A later hint can extend the gate.
          window.clearTimeout(subscription.timeout);
          const newSubId = `history-${crypto.randomUUID()}`;
          subscriptions.delete(subId);
          subscriptions.set(newSubId, subscription);
          const isOwned = () => subscriptions.get(newSubId) === subscription;
          const retryWhenAdmitted = () => {
            if (!isOwned()) return;
            const remainingMs = rateLimitRemainingMs();
            if (remainingMs > 0) {
              subscription.timeout = window.setTimeout(
                retryWhenAdmitted,
                remainingMs,
              );
              return;
            }
            // Arm before transport: EOSE/CLOSED may arrive before send settles.
            subscription.timeout = window.setTimeout(() => {
              if (!isOwned()) return;
              subscriptions.delete(newSubId);
              closeSubscription?.(newSubId)?.catch(() => {});
              subscription.reject(
                new Error("Relay closed the history subscription."),
              );
            }, subscription.timeoutMs);
            void sendReq(newSubId, subscription.filter).catch(() => {
              if (!isOwned()) return;
              window.clearTimeout(subscription.timeout);
              subscriptions.delete(newSubId);
              subscription.reject(
                new Error(message || "Relay closed the history subscription."),
              );
            });
          };
          subscription.timeout = window.setTimeout(
            retryWhenAdmitted,
            rateLimitRemainingMs(),
          );
          return;
        }
      }
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
  subscription.resolveReady?.("closed");
  subscription.resolveReady = undefined;

  // A live sub's backfill is `since: now`, so a deadline there is transient;
  // back off and resubscribe rather than silently dropping live updates.
  const closedClass = isQueryDeadlineError(message)
    ? "retryable"
    : classifyRelayClosed(message);

  if (closedClass === "terminal") {
    // Auth/access/filter failure — permanently remove the subscription so it
    // doesn't silently loop.
    subscriptions.delete(subId);
    clearClosedRetry(subscription);
    subscription.onRemoved?.();
    return;
  }

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
  const retryWhenAdmitted = () => {
    subscription.closedRetryTimeout = undefined;
    if (subscriptions.get(subId) !== subscription) return;
    // A later quota hint can extend the shared gate after this retry was
    // scheduled. Keep the existing cancellable timer owner while waiting;
    // postponement is not another failed attempt and must not grow backoff.
    const remainingMs = rateLimitRemainingMs();
    if (remainingMs > 0) {
      subscription.closedRetryTimeout = window.setTimeout(
        retryWhenAdmitted,
        remainingMs,
      );
      return;
    }
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
  };
  subscription.closedRetryTimeout = window.setTimeout(
    retryWhenAdmitted,
    delayMs,
  );
}

export function prepareSubscriptionEvent(
  subscription: RelaySubscription,
  event: RelayEvent,
) {
  if (subscription.mode === "history") {
    subscription.events.push(event);
    return false;
  }
  if (subscription.mode === "first") {
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

export function shouldDispatchSubscriptionEvent(
  subscription: Extract<RelaySubscription, { mode: "live" }>,
  event: RelayEvent,
) {
  const replay = subscription.reconnectReplay;
  if (replay?.seenEventIds.has(event.id)) return false;
  replay?.seenEventIds.add(event.id);
  return true;
}

export function flushEvents(
  buffer: SubscriptionEventBufferItem[],
  subscriptions: Map<string, RelaySubscription>,
  generation: number,
) {
  for (const item of buffer) {
    const subscription = subscriptions.get(item.subId);
    if (
      subscription?.mode === "live" &&
      item.generation === generation &&
      shouldDispatchSubscriptionEvent(subscription, item.event)
    ) {
      subscription.onEvent(item.event);
    }
  }
}

export function markReconnectLiveEose(
  subscription: Extract<RelaySubscription, { mode: "live" }>,
  generation: number,
) {
  const replay = subscription.reconnectReplay;
  if (!replay || replay.generation !== generation) return;
  replay.liveEose = true;
  if (replay.repairDone) subscription.reconnectReplay = undefined;
}

export function markReconnectRepairDone(
  subscription: Extract<RelaySubscription, { mode: "live" }>,
  generation: number,
) {
  const replay = subscription.reconnectReplay;
  if (!replay || replay.generation !== generation) return;
  replay.repairDone = true;
  if (replay.liveEose) subscription.reconnectReplay = undefined;
}

export function handleSubscriptionEose({
  subscriptions,
  subId,
  closeSubscription,
  generation,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  closeSubscription: (subId: string) => Promise<void>;
  generation?: number;
}) {
  const subscription = subscriptions.get(subId);
  if (!subscription) return;
  if (subscription.mode === "live") {
    if (generation !== undefined)
      markReconnectLiveEose(subscription, generation);
    subscription.resolveReady?.("eose");
    subscription.resolveReady = undefined;
    subscription.closedRetryAttempt = 0;
    clearClosedRetry(subscription);
    return;
  }
  window.clearTimeout(subscription.timeout);
  subscriptions.delete(subId);
  void closeSubscription(subId);
  if (subscription.mode === "first") {
    subscription.resolve(null);
  } else {
    subscription.resolve(sortEvents(subscription.events));
  }
}
