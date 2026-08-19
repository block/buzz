import type { RelayEvent } from "@/shared/api/types";

/**
 * Observable connection state for the relay singleton.
 *
 * - `idle`         — never tried to connect yet (post-init, pre-community).
 * - `connecting`   — initial socket + AUTH handshake in flight.
 * - `connected`    — socket open and AUTH'd.
 * - `reconnecting` — socket dropped, waiting for the backoff timer.
 * - `stalled`      — socket is *open* per the WS layer but no inbound frames
 *                    for a long time (half-open socket / VPN split-brain). We
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

type FirstEventSubscription = {
  mode: "first";
  onEvent: (event: RelayEvent) => void;
  resolve: (event: RelayEvent | null) => void;
  reject: (error: Error) => void;
  timeout: number;
};

export type LiveSubscriptionReadiness = "eose" | "closed" | "timeout";

type LiveSubscription = {
  mode: "live";
  filter: RelaySubscriptionFilter;
  onEvent: (event: RelayEvent) => void;
  resolveReady?: (readiness: LiveSubscriptionReadiness) => void;
  lastSeenCreatedAt?: number;
  /**
   * Lower bound of a reconnect backfill window that has not yet completed.
   *
   * Events on the restored live REQ advance `lastSeenCreatedAt` regardless of
   * backfill success, so after an exhausted backfill the cursor alone would
   * make the next reconnect skip the unresolved older window — silent message
   * loss. This floor is pinned when paging starts and cleared only when a
   * backfill pass completes; the next replay starts from
   * `min(pendingReplaySince, cursor window)`.
   */
  pendingReplaySince?: number;
  closedRetryAttempt?: number;
  closedRetryTimeout?: number;
};

export type PendingEvent = {
  event: RelayEvent;
  resolve: (event: RelayEvent) => void;
  reject: (error: Error) => void;
  timeout: number;
};

export type RelaySubscription =
  | HistorySubscription
  | FirstEventSubscription
  | LiveSubscription;

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

/**
 * Splits a native websocket delivery into individual frames.
 *
 * The native plugin coalesces inbound frames into one IPC delivery, so a
 * delivery is an array of frames. Older single-frame deliveries stay valid:
 * `getTextPayload` rejects arrays, so every consumer must unwrap here rather
 * than per-client — an un-unwrapped consumer drops batched frames silently
 * instead of failing.
 */
export function toRelayFrames(message: unknown): unknown[] {
  return Array.isArray(message) ? message : [message];
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
