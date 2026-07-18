import type { RelayEvent } from "@/shared/api/types";

/**
 * Observable connection state for the relay singleton.
 *
 * - `idle`         — never tried to connect yet (post-init, pre-community).
 * - `connecting`   — initial socket + AUTH handshake in flight.
 * - `connected`    — socket open and AUTH'd.
 * - `reconnecting` — socket dropped, waiting for the backoff timer.
 * - `stalled`      — socket is *open* per the WS layer but no inbound frames
 *                    for a long time (half-open / Warp split-brain). We
 *                    surface this so the UI can warn even though tungstenite
 *                    hasn't reported anything wrong yet.
 * - `disconnected` — final/terminal disconnect (auth rejected, community
 *                    switch, etc.) — no auto-reconnect scheduled.
 */
export type ConnectionState =
  | "idle"
  | "connecting"
  | "connected"
  | "reconnecting"
  | "stalled"
  | "disconnected";

/** True when the UI should surface a "connection lost" indicator. */
export function isRelayConnectionDegraded(state: ConnectionState): boolean {
  return (
    state === "reconnecting" || state === "stalled" || state === "disconnected"
  );
}

export type RelaySubscriptionFilter = {
  ids?: string[];
  kinds: number[];
  limit: number;
  authors?: string[];
  since?: number;
  until?: number;
} & Partial<Record<`#${string}`, string[]>>;

type HistorySubscription = {
  mode: "history";
  events: RelayEvent[];
  resolve: (events: RelayEvent[]) => void;
  reject: (error: Error) => void;
  timeout: number;
};

type LiveSubscription = {
  mode: "live";
  filter: RelaySubscriptionFilter;
  onEvent: (event: RelayEvent) => void;
  resolveReady?: () => void;
  lastSeenCreatedAt?: number;
  closedRetryAttempt?: number;
  closedRetryTimeout?: number;
};

export type PendingEvent = {
  event: RelayEvent;
  resolve: (event: RelayEvent) => void;
  reject: (error: Error) => void;
  timeout: number;
};

export function handleSubMessage(
  subscriptions: Map<string, RelaySubscription>,
  type: unknown,
  values: unknown[],
  closeSubscription: (subId: string) => void,
): boolean {
  const subId = values[0];
  if (typeof subId !== "string") {
    return false;
  }
  if (type === "EOSE") {
    const subscription = subscriptions.get(subId);
    if (!subscription) return true;
    if (subscription.mode === "live") {
      subscription.resolveReady?.();
      subscription.resolveReady = undefined;
      return true;
    }
    window.clearTimeout(subscription.timeout);
    subscriptions.delete(subId);
    closeSubscription(subId);
    subscription.resolve(sortEvents(subscription.events));
    return true;
  }
  if (type === "CLOSED") {
    rejectClosedSubscription(
      subscriptions,
      subId,
      typeof values[1] === "string" ? values[1] : "",
    );
    return true;
  }
  return false;
}

/**
 * Handle a relay-terminated subscription. History subscriptions settle exactly
 * once and are removed; live subscriptions settle readiness but stay registered
 * so the existing reconnect path can replay them.
 */
export function rejectClosedSubscription(
  subscriptions: Map<string, RelaySubscription>,
  subId: string,
  message: string,
): boolean {
  const subscription = subscriptions.get(subId);
  if (!subscription) {
    return false;
  }

  if (subscription.mode === "history") {
    subscriptions.delete(subId);
    window.clearTimeout(subscription.timeout);
    subscription.reject(
      new Error(message || "Relay closed the history subscription."),
    );
  } else {
    subscription.resolveReady?.();
    subscription.resolveReady = undefined;
  }

  return true;
}

export type RelaySubscription = HistorySubscription | LiveSubscription;

export function sortEvents(events: RelayEvent[]) {
  return [...events].sort((left, right) => {
    if (left.created_at !== right.created_at) {
      return left.created_at - right.created_at;
    }
    // Same (created_at, id) tiebreak as the cache sort (sortMessages) so a
    // history REQ resolves same-second events in a stable, relay-matching
    // order. Currently every consumer re-sorts downstream, but keeping the
    // two sorts on one invariant avoids a latent ordering drift.
    return left.id < right.id ? -1 : left.id > right.id ? 1 : 0;
  });
}

export function getTextPayload(message: unknown) {
  if (typeof message === "string") {
    return message;
  }

  if (
    typeof message === "object" &&
    message !== null &&
    "type" in message &&
    message.type === "Text" &&
    "data" in message &&
    typeof message.data === "string"
  ) {
    return message.data;
  }

  if (
    typeof message === "object" &&
    message !== null &&
    "Text" in message &&
    typeof message.Text === "string"
  ) {
    return message.Text;
  }

  return null;
}
