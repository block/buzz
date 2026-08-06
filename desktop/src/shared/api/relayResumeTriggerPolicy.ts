/**
 * Policy for event-driven reconnect triggers (G1 of the CMD+R gap audit).
 *
 * The exponential-backoff timer is the only thing driving recovery after an
 * outage — and WKWebView throttles JS timers in occluded/background windows,
 * so at max backoff (30s) a scheduled attempt may not fire until the user
 * focuses the window. These triggers short-circuit the wait the moment the
 * environment signals recovery: network `online`, window focus, and
 * visibility becoming visible. `preconnect()` already clears the pending
 * backoff timer, so a trigger converts "wait up to 30s (or forever, if
 * throttled)" into "reconnect now".
 */
import type { ConnectionState } from "@/shared/api/relayClientShared";

/** Min ms between trigger-driven preconnect attempts. */
export const RESUME_TRIGGER_MIN_INTERVAL_MS = 5_000;

export function shouldTriggerResumeReconnect(inputs: {
  connectionState: ConnectionState;
  lastAttemptAt: number;
  now: number;
  minIntervalMs?: number;
}): boolean {
  const minInterval = inputs.minIntervalMs ?? RESUME_TRIGGER_MIN_INTERVAL_MS;

  // Only degraded-but-recoverable states. `disconnected` is the terminal
  // latch — explicit user re-engagement owns that path, and `idle` /
  // `connecting` / `connected` need no help.
  if (
    inputs.connectionState !== "reconnecting" &&
    inputs.connectionState !== "stalled"
  ) {
    return false;
  }

  // Rate-limit: focus/online events arrive in bursts (e.g. wake fires all
  // three); one attempt per window is enough.
  return inputs.now - inputs.lastAttemptAt >= minInterval;
}
