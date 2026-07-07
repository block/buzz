/**
 * Reactive detection of a community timeout from a send rejection.
 *
 * The relay refuses writes from a timed-out member with an `OK false` message
 * of the exact form (ingest.rs, load-bearing parse contract):
 *
 *     restricted: you are timed out until <unix_seconds>
 *
 * There is no proactive self-restriction read in v1, so the composer learns it
 * is timed out only by attempting a send and inspecting the rejection. This is
 * the Option-A (reactive) ruling.
 */

const TIMEOUT_PREFIX = "restricted: you are timed out until";

export type TimeoutRejection = {
  /**
   * Timeout expiry in epoch milliseconds, or `null` when the relay's message
   * carried an unparseable timestamp. A `null` expiry still means "timed out" —
   * the caller shows the chip without a countdown rather than pretending the
   * member can send.
   */
  expiresAtMs: number | null;
};

/**
 * Parse a relay send-rejection message. Returns a {@link TimeoutRejection} when
 * the message is a timeout refusal, or `null` for any other rejection (which
 * the caller surfaces through its normal error path, untouched).
 *
 * Defensive by contract: the prefix match is what identifies a timeout; the
 * timestamp is best-effort. A malformed or out-of-range trailing value yields
 * `expiresAtMs: null`, never a throw and never a false negative on the prefix.
 */
export function parseTimeoutRejection(
  message: string | null | undefined,
): TimeoutRejection | null {
  if (!message) {
    return null;
  }
  const trimmed = message.trim();
  if (!trimmed.startsWith(TIMEOUT_PREFIX)) {
    return null;
  }
  const rest = trimmed.slice(TIMEOUT_PREFIX.length).trim();
  const seconds = Number.parseInt(rest, 10);
  if (!Number.isSafeInteger(seconds) || seconds <= 0) {
    return { expiresAtMs: null };
  }
  return { expiresAtMs: seconds * 1000 };
}

/**
 * True when a known timeout expiry is still in the future relative to `nowMs`.
 * A `null` expiry (unknown) is treated as still-active — fail closed, since the
 * member was demonstrably timed out at their last send attempt.
 */
export function isTimeoutActive(
  expiresAtMs: number | null,
  nowMs: number = Date.now(),
): boolean {
  if (expiresAtMs === null) {
    return true;
  }
  return expiresAtMs > nowMs;
}
