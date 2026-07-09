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

/**
 * The community-timeout durations offered wherever a moderator picks one — the
 * per-message author cluster (U2) and the report-queue timeout resolution.
 * Kept here as the single source of truth so the two surfaces can never drift.
 */
export const TIMEOUT_PRESETS: ReadonlyArray<{
  label: string;
  seconds: number;
}> = [
  { label: "1 hour", seconds: 60 * 60 },
  { label: "24 hours", seconds: 24 * 60 * 60 },
  { label: "7 days", seconds: 7 * 24 * 60 * 60 },
];

/**
 * Convert a preset duration into the absolute expiry (epoch **seconds**) the
 * timeout command (`useTimeoutMemberMutation`) expects — `now + seconds`. The
 * relay stamps its own authoritative expiry; this is the client's request.
 */
export function timeoutExpiresAt(
  seconds: number,
  nowMs: number = Date.now(),
): number {
  return Math.floor(nowMs / 1000) + seconds;
}

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

/**
 * Format the time left until `expiresAtMs` as a short human string for the
 * composer chip: `"2h 5m"`, `"3m 20s"`, `"12s"`. Returns `null` when there is
 * no countdown to show — either the expiry is unknown or already elapsed.
 */
export function formatTimeoutRemaining(
  expiresAtMs: number | null,
  nowMs: number = Date.now(),
): string | null {
  if (expiresAtMs === null) {
    return null;
  }
  const totalSeconds = Math.ceil((expiresAtMs - nowMs) / 1000);
  if (totalSeconds <= 0) {
    return null;
  }
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }
  if (minutes > 0) {
    return `${minutes}m ${seconds}s`;
  }
  return `${seconds}s`;
}
