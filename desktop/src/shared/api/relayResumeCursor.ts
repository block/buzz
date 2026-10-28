import type { RelaySubscription } from "@/shared/api/relayClientShared";

/**
 * Reconnect resume cursor.
 *
 * A subscription that reached EOSE on a connection was complete up to that
 * EOSE and live afterwards until the socket closed, so a quiet subscription
 * needs no history older than that close on its next reconnect. Without this,
 * a subscription that never received an event replays its ORIGINAL filter
 * (startup `since`, full `limit`) on every reconnect, which multiplies reader
 * load across the fleet during a rolling relay deploy.
 */

/** Skew below the newest event seen (matches the pre-existing replay skew). */
export const RESUME_LAST_SEEN_SKEW_SECS = 5;

/**
 * Skew below the recorded live-until time. Absorbs client/relay clock skew and
 * `created_at` jitter between the last delivered frame and the close.
 */
export const RESUME_LIVE_UNTIL_SKEW_SECS = 60;

/**
 * Lower bound for a reconnect replay, or `undefined` when the subscription has
 * no cursor at all (never saw an event and never reached EOSE on a connection
 * that has since closed) and must replay its original filter.
 *
 * `max(lastSeen - lastSeenSkew, liveUntil - liveUntilSkew)`: the callers
 * apply the original filter's own `since` as a further floor and lower it with
 * any pinned failed-backfill floor, so a recorded gap always wins.
 */
export function computeResumeSince({
  lastSeenCreatedAt,
  liveUntil,
  lastSeenSkewSecs = RESUME_LAST_SEEN_SKEW_SECS,
  liveUntilSkewSecs = RESUME_LIVE_UNTIL_SKEW_SECS,
}: {
  lastSeenCreatedAt?: number;
  liveUntil?: number;
  lastSeenSkewSecs?: number;
  liveUntilSkewSecs?: number;
}): number | undefined {
  const bounds: number[] = [];
  if (lastSeenCreatedAt !== undefined) {
    bounds.push(lastSeenCreatedAt - lastSeenSkewSecs);
  }
  if (liveUntil !== undefined) bounds.push(liveUntil - liveUntilSkewSecs);
  return bounds.length === 0 ? undefined : Math.max(0, ...bounds);
}

/**
 * Record `liveUntil` for every live subscription that reached EOSE on the
 * connection that is closing. Subscriptions that did not reach EOSE on it keep
 * their previous `liveUntil` (if any): their history was never confirmed
 * delivered on this connection.
 *
 * `lastInboundMs` caps the stamp for half-open sockets: when a stall is only
 * detected after a minute of silence, the close time overstates how long the
 * subscription was actually live.
 */
export function stampLiveUntilOnClose(
  subscriptions: Map<string, RelaySubscription>,
  generation: number,
  /** Time of the last inbound frame, or 0 when unknown. */
  lastInboundMs: number,
  nowMs = Date.now(),
) {
  const liveUntil = Math.floor(
    (lastInboundMs ? Math.min(nowMs, lastInboundMs) : nowMs) / 1_000,
  );
  for (const subscription of subscriptions.values()) {
    if (subscription.mode !== "live") continue;
    if (subscription.eoseGeneration === generation) {
      subscription.liveUntil = Math.max(subscription.liveUntil ?? 0, liveUntil);
    }
    subscription.eoseGeneration = undefined;
  }
}
