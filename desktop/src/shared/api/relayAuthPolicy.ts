/**
 * Policy for NIP-42 AUTH `OK` responses (G2 of the CMD+R gap audit).
 *
 * Historically ANY auth `OK false` latched the session terminal — no
 * reconnect until explicit user re-engagement. But the relay sends
 * `OK false` for conditions that are transient from the client's side:
 *
 *  - `auth-required: already authenticated` — a duplicate/late AUTH event on
 *    a connection that is in fact authenticated. The session is usable;
 *    treat it as authenticated.
 *  - `auth-required: verification failed` — covers ±60s clock-skew rejects
 *    and the relay's fail-closed allowlist DB lookup errors, both of which
 *    can clear on retry.
 *
 * Only `restricted:` and `blocked:` rejections (not a relay member / banned)
 * are known permanent. Everything else retries with normal backoff, but
 * latches terminal after `MAX_CONSECUTIVE_AUTH_REJECTIONS` consecutive
 * rejections so a genuinely broken identity (e.g. persistently wrong system
 * clock) still surfaces the terminal error card instead of flapping forever.
 *
 * The rejection streak is preserved across environment-driven resume
 * attempts (focus/online/visibility); only explicit user re-engagement —
 * the reconnect card or a community switch — may reset it.
 */
export type AuthOkDecision = "authenticated" | "retry" | "terminal";

export const MAX_CONSECUTIVE_AUTH_REJECTIONS = 3;

/** Tracks consecutive AUTH rejections across reconnect attempts. */
export class AuthOkTracker {
  private consecutiveRejections = 0;

  /**
   * Record an AUTH `OK` and decide the session's next move.
   * A success — real or "already authenticated" — resets the streak.
   */
  record(success: boolean, message: string): AuthOkDecision {
    const normalized = message.trim().toLowerCase();
    if (
      success ||
      normalized.startsWith("auth-required: already authenticated")
    ) {
      this.consecutiveRejections = 0;
      return "authenticated";
    }

    this.consecutiveRejections++;

    if (
      normalized.startsWith("restricted:") ||
      normalized.startsWith("blocked:")
    ) {
      return "terminal";
    }
    if (this.consecutiveRejections >= MAX_CONSECUTIVE_AUTH_REJECTIONS) {
      return "terminal";
    }
    return "retry";
  }

  /** Called on explicit re-engagement (disconnect / manual preconnect). */
  reset(): void {
    this.consecutiveRejections = 0;
  }
}
