import type { RelaySubscription } from "@/shared/api/relayClientShared";
import {
  STALL_CHECK_INTERVAL_MS,
  STALL_IDLE_TIMEOUT_MS,
} from "@/shared/api/relayClientTimings";

/**
 * Reconnect resume cursor.
 *
 * A subscription that reached EOSE on a connection was complete up to that
 * EOSE and subscribed afterwards until the socket's last inbound frame, so a
 * quiet subscription needs no history older than that on its next reconnect.
 * Without this, a subscription that never received an event replays its
 * ORIGINAL filter (startup `since`, full `limit`) on every reconnect, which
 * multiplies reader load across the fleet during a rolling relay deploy.
 *
 * Subscribed is not delivered: the relay drops fan-out frames for a client
 * whose send buffer is full while the drop count is under its grace limit
 * (`ConnectionState::send` in `crates/buzz-relay/src/connection.rs`), so a
 * quiet subscription can miss its only event and still count as live. The
 * liveness term therefore never resumes later than
 * `RESUME_SILENT_DROP_WINDOW_SECS` ago: a drop within that window is replayed
 * on the next reconnect; an older one is left to the timeline's own refetch.
 */

/**
 * Every liveness-derived resume re-reads at least this much recent history,
 * bounding how long a silently dropped fan-out frame can stay unrepaired.
 */
export const RESUME_SILENT_DROP_WINDOW_SECS = 3_600;

/** Skew below the newest event seen (matches the pre-existing replay skew). */
export const RESUME_LAST_SEEN_SKEW_SECS = 5;

/**
 * The relay rejects events whose `created_at` is more than this far from its
 * clock in either direction (`MAX_TIMESTAMP_DRIFT_SECS` in
 * `crates/buzz-relay/src/handlers/ingest.rs`; the replay test pins the
 * coupling). A row stored after a close can therefore carry a `created_at`
 * this far in the past.
 */
export const RELAY_INGEST_FUTURE_TOLERANCE_SECS = 900;

/** Client-vs-relay clock skew margin on top of the relay's drift window. */
export const RESUME_CLIENT_CLOCK_MARGIN_SECS = 60;

/**
 * Skew below the recorded live-until time: the relay's `created_at` drift
 * window plus a client clock margin. Anything stored after the close has
 * `created_at >= closeTime - 900s` on the relay clock, so a narrower window
 * would skip a backdated event that was stored after the subscription went
 * quiet. Paged channels use the wider channel lookback instead.
 */
export const RESUME_LIVE_UNTIL_SKEW_SECS =
  RELAY_INGEST_FUTURE_TOLERANCE_SECS + RESUME_CLIENT_CLOCK_MARGIN_SECS;

/**
 * Lower bound for a reconnect replay, or `undefined` when the subscription has
 * no cursor at all (never saw an event and never reached EOSE on a connection
 * that has since closed) and must replay its original filter.
 *
 * `max(lastSeen - lastSeenSkew, min(liveUntil - liveUntilSkew, now -
 * silentDropWindow))`: the callers apply the original filter's own `since` as
 * a further floor and lower it with any pinned failed-backfill floor, so a
 * recorded gap always wins.
 */
export function computeResumeSince({
  lastSeenCreatedAt,
  liveUntil,
  lastSeenSkewSecs = RESUME_LAST_SEEN_SKEW_SECS,
  liveUntilSkewSecs = RESUME_LIVE_UNTIL_SKEW_SECS,
  nowSecs = Math.floor(Date.now() / 1_000),
}: {
  lastSeenCreatedAt?: number;
  liveUntil?: number;
  lastSeenSkewSecs?: number;
  liveUntilSkewSecs?: number;
  nowSecs?: number;
}): number | undefined {
  const bounds: number[] = [];
  if (lastSeenCreatedAt !== undefined) {
    bounds.push(lastSeenCreatedAt - lastSeenSkewSecs);
  }
  if (liveUntil !== undefined) {
    bounds.push(
      Math.min(
        liveUntil - liveUntilSkewSecs,
        nowSecs - RESUME_SILENT_DROP_WINDOW_SECS,
      ),
    );
  }
  return bounds.length === 0 ? undefined : Math.max(0, ...bounds);
}

/**
 * Longest silence a running renderer can see before the stall watchdog resets
 * the socket. A frame processed after a longer silence may have been queued
 * through a sleep, since frames are stamped when processed, not received.
 */
export const LIVENESS_MAX_GAP_MS =
  STALL_IDLE_TIMEOUT_MS + STALL_CHECK_INTERVAL_MS;

/**
 * Next liveness credit for a connection after an inbound frame at `nowMs`.
 * Once a gap longer than `LIVENESS_MAX_GAP_MS` is seen, credit stays at the
 * last frame before it for the rest of the connection: whatever the relay
 * sent (or dropped) during the gap was never confirmed.
 */
export function creditInboundLiveness(previousMs: number, nowMs: number) {
  return previousMs === 0 || nowMs - previousMs <= LIVENESS_MAX_GAP_MS
    ? nowMs
    : previousMs;
}

/**
 * Record `liveUntil` for every live subscription that reached EOSE on the
 * connection that is closing. Subscriptions that did not reach EOSE on it keep
 * their previous `liveUntil` (if any): their history was never confirmed
 * delivered on this connection.
 *
 * The stamp is the time of the connection's last inbound peer frame, never
 * the close: after a sleep or a stalled socket, the close is processed long
 * after the subscription last heard from the relay. When that time is unknown
 * (`0`) the previous `liveUntil` is kept rather than guessed from the clock.
 */
export function stampLiveUntilOnClose(
  subscriptions: Map<string, RelaySubscription>,
  generation: number,
  /** Time of the connection's last inbound peer frame, or 0 when unknown. */
  lastInboundMs: number,
  nowMs = Date.now(),
) {
  const liveUntil = Math.floor(Math.min(nowMs, lastInboundMs) / 1_000);
  for (const subscription of subscriptions.values()) {
    if (subscription.mode !== "live") continue;
    if (subscription.eoseGeneration === generation && lastInboundMs > 0) {
      subscription.liveUntil = Math.max(subscription.liveUntil ?? 0, liveUntil);
    }
    subscription.eoseGeneration = undefined;
  }
}
