/**
 * Pure helpers for the relay reconnect policy.
 *
 * Extracted from `RelayClient` so the decision rules — when to schedule a
 * reconnect, when to refuse to connect — live in one legible place that
 * unit tests can reach without booting the WS layer.
 *
 * The rules:
 *
 *  - **Terminal sessions never reconnect.** When the relay has explicitly
 *    rejected us (today: kind:22242 AUTH OK=false) the session is dead
 *    until the user re-engages (community switch or explicit preconnect).
 *    This guards the reconnect-timer catch handler — and the retry wrappers
 *    in `publishEvent` / `sendRawWithReconnectRetry` — from racing the
 *    `disconnected` state back to `reconnecting`.
 *
 *  - **No-op when a reconnect is already scheduled or in progress.**
 *    A pending timer or a live `wsId` means we have nothing to do.
 *
 *  - **No reconnect needed when nothing wants the socket.** No live
 *    subscription, no `keepAliveRequested` from `preconnect()` → don't
 *    keep an idle socket.
 */
import {
  BACKOFF_RESET_STABLE_MS,
  RECONNECT_BASE_DELAY_MS,
  RECONNECT_MAX_DELAY_MS,
} from "@/shared/api/relayClientTimings";

export type RelayReconnectInputs = {
  terminal: boolean;
  hasPendingReconnect: boolean;
  hasLiveSocket: boolean;
  keepAliveRequested: boolean;
  hasLiveSubscriptions: boolean;
};

export function shouldScheduleReconnect(inputs: RelayReconnectInputs): boolean {
  if (inputs.terminal) return false;
  if (inputs.hasPendingReconnect) return false;
  if (inputs.hasLiveSocket) return false;
  if (!inputs.keepAliveRequested && !inputs.hasLiveSubscriptions) return false;
  return true;
}

export function shouldWaitForScheduledReconnect(inputs: {
  hasPendingReconnect: boolean;
}): boolean {
  return inputs.hasPendingReconnect;
}

/** Whether `ensureConnected()` should refuse with a terminal error. */
export function shouldRefuseConnect(inputs: { terminal: boolean }): boolean {
  return inputs.terminal;
}

export function isWebSocketClose(
  message: unknown,
): message is { type: "Close"; data?: unknown } {
  return (
    typeof message === "object" &&
    message !== null &&
    "type" in message &&
    message.type === "Close"
  );
}

/**
 * Whether the relay closed the socket because it is going away: 1012 (Service
 * Restart, sent to every client of a pod during a rolling deploy) or 1001
 * (Going Away). Both mean "the whole fleet is about to reconnect".
 */
export function isServiceRestartClose(message: unknown): boolean {
  if (!isWebSocketClose(message)) return false;
  if (!("data" in message)) return false;
  const data = message.data;
  return (
    typeof data === "object" &&
    data !== null &&
    "code" in data &&
    (data.code === 1012 || data.code === 1001)
  );
}

/**
 * Server-side close drain is spread over ~10s; the first reconnect after a
 * restart close spreads arrival across the same window ("full jitter").
 */
export const SERVICE_RESTART_RECONNECT_SPREAD_MS = 10_000;

/**
 * Exponential reconnect backoff with jitter.
 *
 * - After a service-restart close the first attempt waits a uniform random
 *   delay in `[0, SERVICE_RESTART_RECONNECT_SPREAD_MS]` instead of resetting to
 *   the minimum backoff, so a fleet bounced by one deploy does not stampede.
 * - Otherwise the delay gets symmetric +/-25% jitter, hard-capped at `maxMs`
 *   (a retry never waits longer than the cap).
 * - Every attempt, including a spread one, doubles the stored backoff; only
 *   a connection that stays up (`resetWhenStable`) or `reset()` (community
 *   switch) returns it to the minimum. Merely opening a socket never does.
 *
 * `random` is injectable for tests and must return a value in `[0, 1)`.
 */
export class RelayReconnectBackoff {
  private backoffMs: number;
  private readonly baseMs: number;
  private readonly maxMs: number;
  private readonly random: () => number;
  private stableTimer: number | null = null;

  constructor(
    baseMs = RECONNECT_BASE_DELAY_MS,
    maxMs = RECONNECT_MAX_DELAY_MS,
    random = () => Math.random(),
  ) {
    this.baseMs = baseMs;
    this.backoffMs = baseMs;
    this.maxMs = maxMs;
    this.random = random;
  }

  /** Current un-jittered backoff, for diagnostics and tests. */
  get currentMs(): number {
    return this.backoffMs;
  }

  /** Back to the minimum now, dropping any pending stable-connection reset. */
  reset(): void {
    this.cancelStableReset();
    this.backoffMs = this.baseMs;
  }

  /** Reset once the connection that just opened has stayed up `stableMs`. */
  resetWhenStable(stableMs = BACKOFF_RESET_STABLE_MS): void {
    this.cancelStableReset();
    this.stableTimer = window.setTimeout(() => {
      this.stableTimer = null;
      this.backoffMs = this.baseMs;
    }, stableMs);
  }

  /** The connection dropped (or a new attempt began) before it was stable. */
  cancelStableReset(): void {
    if (this.stableTimer === null) return;
    window.clearTimeout(this.stableTimer);
    this.stableTimer = null;
  }

  /** Delay before the next attempt; advances the backoff. */
  next(serviceRestart = false): number {
    const delay = serviceRestart
      ? this.random() * SERVICE_RESTART_RECONNECT_SPREAD_MS
      : Math.min(this.backoffMs * (0.75 + this.random() * 0.5), this.maxMs);
    this.backoffMs = Math.min(this.backoffMs * 2, this.maxMs);
    return delay;
  }
}

/** Whether a WS-layer message is the plugin's `Error` frame. */
export function isWebSocketError(message: unknown): boolean {
  return (
    typeof message === "object" &&
    message !== null &&
    "type" in message &&
    message.type === "Error"
  );
}
