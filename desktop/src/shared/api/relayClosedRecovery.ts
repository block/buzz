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
  type SubscriptionEventBufferItem,
} from "@/shared/api/relayClientShared";
import {
  beginReconnectRepair,
  buildReconnectReplayFilter,
  reconnectReplaySince,
  repairChannelSubscription,
  shouldPageReconnectReplay,
} from "@/shared/api/relayReconnectReplay";
import type { ChannelReconnectRepairRequest } from "@/shared/api/channelReconnectRepair";
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
  requestRepair,
  generation = 0,
  isActive = () => true,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  message: string;
  sendReq: (subId: string, filter: RelaySubscriptionFilter) => Promise<void>;
  /** Paged channel repair; without it a CLOSED retry resends the cursor filter only. */
  requestRepair?: (
    request: ChannelReconnectRepairRequest,
  ) => Promise<RelayEvent[]>;
  /** Connection generation the CLOSED frame arrived on. */
  generation?: number;
  isActive?: () => boolean;
}) {
  const subscription = subscriptions.get(subId);
  if (!subscription) return;
  if (subscription.mode !== "live") {
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
    requestRepair,
    generation,
    isActive,
  });
}

function recoverLiveSubscriptionFromClosed({
  subscriptions,
  subId,
  subscription,
  message,
  sendReq,
  requestRepair,
  generation = 0,
  isActive = () => true,
}: {
  subscriptions: Map<string, RelaySubscription>;
  subId: string;
  subscription: LiveSubscription;
  message: string;
  sendReq: (subId: string, filter: RelaySubscriptionFilter) => Promise<void>;
  /** Paged channel repair; without it a CLOSED retry resends the cursor filter only. */
  requestRepair?: (
    request: ChannelReconnectRepairRequest,
  ) => Promise<RelayEvent[]>;
  /** Connection generation the CLOSED frame arrived on. */
  generation?: number;
  isActive?: () => boolean;
}) {
  subscription.resolveReady?.("closed");
  subscription.resolveReady = undefined;

  const closedClass = classifyRelayClosed(message);

  if (closedClass === "terminal") {
    // Auth/access/filter failure — permanently remove the subscription so it
    // doesn't silently loop.
    subscriptions.delete(subId);
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
  subscription.closedRetryTimeout = window.setTimeout(() => {
    subscription.closedRetryTimeout = undefined;
    if (subscriptions.get(subId) !== subscription) return;
    // The original filter has no reader-clock `since` and replays only its
    // newest `limit` rows, so resume from the accepted-event cursor instead.
    const channelId = subscription.filter["#h"]?.[0];
    const shouldPageReplay =
      channelId !== undefined &&
      requestRepair !== undefined &&
      shouldPageReconnectReplay(subscription.filter);
    const replaySince = reconnectReplaySince(subscription, shouldPageReplay);
    const willRepair = shouldPageReplay && replaySince !== undefined;
    if (willRepair) beginReconnectRepair(subscription, generation);

    void (async () => {
      await sendReq(
        subId,
        willRepair
          ? subscription.filter
          : buildReconnectReplayFilter(subscription.filter, replaySince),
      );
      if (
        !willRepair ||
        !requestRepair ||
        channelId === undefined ||
        replaySince === undefined
      )
        return;
      await repairChannelSubscription({
        subId,
        subscription,
        subscriptions,
        channelId,
        replaySince,
        generation,
        isActive,
        requestRepair,
      });
    })().catch((error) => {
      if (subscriptions.get(subId) !== subscription) return;
      console.error("Failed to restore closed relay subscription", error);
      recoverLiveSubscriptionFromClosed({
        subscriptions,
        subId,
        subscription,
        message,
        sendReq,
        requestRepair,
        generation,
        isActive,
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
  const flushCallbacks = new Set<() => void>();
  for (const item of buffer) {
    const subscription = subscriptions.get(item.subId);
    if (
      subscription?.mode === "live" &&
      item.generation === generation &&
      shouldDispatchSubscriptionEvent(subscription, item.event)
    ) {
      subscription.onEvent(item.event);
      if (subscription.onFlush) flushCallbacks.add(subscription.onFlush);
    }
  }
  for (const callback of flushCallbacks) callback();
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
