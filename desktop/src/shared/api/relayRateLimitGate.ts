/**
 * Module-level rate-limit gate for the relay WebSocket and HTTP bridge.
 *
 * When the relay signals back-pressure via a CLOSED `rate-limited:` message or
 * an HTTP 429 response, callers activate the gate. Operations that must not run
 * while rate-limited call `isRateLimited()` or await `waitForRateLimit()`.
 *
 * The gate is a singleton: one shared expiry covers all concurrent callers so
 * overlapping hints (multiple CLOSED frames) extend to the latest expiry without
 * stacking. This module must be reset on community switch via
 * `resetRateLimitGate()`.
 */

/** Minimum gate duration when the relay provides no `retry in Ns` hint. */
const DEFAULT_RATE_LIMIT_SECONDS = 10;

/**
 * Maximum hint the TS gate will honour from a relay 429 response.
 *
 * Mirrors `MAX_HINT_SECONDS` in `relay_admission.rs` (Rust). The Rust relay
 * layer clamps the hint before embedding it in the error string, so in practice
 * this TS cap is a defence-in-depth guard against any future Rust path that
 * forgets to clamp, keeping both gates on the same documented bound.
 */
export const MAX_HINT_SECONDS = 300;

let expiresAt: number | null = null;
let gateTimer: number | null = null;
let gateResolve: (() => void) | null = null;
let gatePromise: Promise<void> | null = null;

/**
 * Parse `retry in Ns` from a relay message string.
 *
 * Matches the relay's canonical hint format embedded in both CLOSED messages
 * (`rate-limited: quota exceeded; retry in 4s`) and HTTP 429 bodies
 * (`relay rate-limited: retry in 4s`).
 */
export function parseRateLimitHint(msg: string): number | null {
  const match = /retry in (\d+)s/i.exec(msg);
  return match ? parseInt(match[1], 10) : null;
}

/**
 * Activate (or extend) the rate-limit gate.
 *
 * If the gate is already active, the expiry is pushed forward to the maximum of
 * the existing expiry and the new hint — overlapping hints never shrink the
 * window. Non-positive or absent hints use the 10-second default; a 0s gate
 * would resolve immediately and swallow the signal.
 *
 * Note: buzz-acp uses a 5s no-hint default; desktop deliberately uses 10s here
 * for a wider back-off window on degraded connections.
 */
export function activateRateLimit(retryInSeconds: number | null): void {
  const durationMs =
    (retryInSeconds != null && retryInSeconds > 0
      ? Math.min(retryInSeconds, MAX_HINT_SECONDS)
      : DEFAULT_RATE_LIMIT_SECONDS) * 1_000;
  const newExpiry = Date.now() + durationMs;

  if (expiresAt !== null && newExpiry <= expiresAt) {
    // Existing window already covers this hint — nothing to do.
    return;
  }

  expiresAt = newExpiry;

  if (gateTimer !== null) {
    window.clearTimeout(gateTimer);
  }

  if (gatePromise === null) {
    // First activation: create the shared promise that waiters will await.
    gatePromise = new Promise<void>((resolve) => {
      gateResolve = resolve;
    });
  }

  gateTimer = window.setTimeout(() => {
    gateTimer = null;
    expiresAt = null;
    const resolve = gateResolve;
    gateResolve = null;
    gatePromise = null;
    resolve?.();
  }, durationMs);
}

/** Returns `true` when the relay has signalled back-pressure and the gate is active. */
export function isRateLimited(): boolean {
  return expiresAt !== null && Date.now() < expiresAt;
}

/**
 * Resolves when the rate-limit gate expires.
 *
 * Resolves immediately if the gate is not active. Callers that need to wait
 * before issuing a new relay request should await this before proceeding.
 */
export function waitForRateLimit(): Promise<void> {
  if (!isRateLimited() || gatePromise === null) {
    return Promise.resolve();
  }
  return gatePromise;
}

/**
 * Returns the milliseconds remaining on the active gate, or 0 when inactive.
 *
 * Use this instead of re-deriving the hint from the message so that a shorter
 * relay hint arriving under a longer active gate never schedules a premature
 * retry.
 */
export function rateLimitRemainingMs(): number {
  if (expiresAt === null) return 0;
  return Math.max(0, expiresAt - Date.now());
}

/**
 * Reset all gate state. Must be called on community switch so a rate-limit hint
 * from the old relay does not bleed into the new community's session.
 *
 * Any in-flight `waitForRateLimit()` awaiters are resolved immediately so they
 * do not leak into the new session.
 */
export function resetRateLimitGate(): void {
  if (gateTimer !== null) {
    window.clearTimeout(gateTimer);
    gateTimer = null;
  }
  expiresAt = null;
  const resolve = gateResolve;
  gateResolve = null;
  gatePromise = null;
  resolve?.();
}
