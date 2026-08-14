/**
 * CLOSED ends a NIP-01 subscription. Retry failures that may recover without
 * changing the request; authorization and malformed-filter failures require a
 * caller/state change and would otherwise loop forever.
 */

/**
 * Three-way classification of a relay CLOSED message.
 *
 * - `"retryable"` — transient failure; re-send the REQ after a backoff.
 * - `"rate-limited"` — relay back-pressure; activate the rate-limit gate and
 *   retry when the gate expires (subscription survives).
 * - `"terminal"` — auth, access, or filter error; delete the subscription.
 */
export type RelayClosedClass = "retryable" | "rate-limited" | "terminal";

export function classifyRelayClosed(message: string): RelayClosedClass {
  const normalized = message.trim().toLowerCase();
  if (normalized.startsWith("rate-limited:")) {
    return "rate-limited";
  }
  // `auth-required:` is deliberately retryable, NOT terminal: it occurs
  // transiently when a REQ races the AUTH handshake after a reconnect. The
  // backoff retry re-sends the REQ once the session is authenticated. A
  // session that is genuinely unauthenticated latches `terminal` at the
  // connection level (AUTH OK=false), so this cannot loop forever.
  if (
    normalized.startsWith("restricted:") ||
    normalized.startsWith("blocked:") ||
    normalized.startsWith("invalid:") ||
    normalized.startsWith("pow:") ||
    normalized.startsWith("duplicate:") ||
    normalized.startsWith("unsupported:") ||
    normalized.startsWith("error: mixed search") ||
    normalized.startsWith("error: too many subscriptions")
  ) {
    return "terminal";
  }
  return "retryable";
}
