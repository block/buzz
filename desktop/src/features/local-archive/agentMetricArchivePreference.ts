/**
 * Persists whether the user has made an explicit choice about the
 * agent-turn-metric archive default-on feature.
 *
 * The key is identity-scoped so toggling off on one identity doesn't suppress
 * the default-on for another identity.  The value is:
 *   "1"  → user explicitly enabled (or accepted the default)
 *   "0"  → user explicitly disabled
 *   null → no explicit choice yet (default-on seeding may still fire)
 *
 * Device-level localStorage — intentionally not reset on workspace switch
 * (the archive subscription itself is identity-scoped in SQLite; this flag
 * is just the UI gate that prevents re-seeding after an explicit opt-out).
 */

const KEY_PREFIX = "buzz:agent-metric-archive-default-seeded";

function storageKey(identityPubkey: string): string {
  return `${KEY_PREFIX}:${identityPubkey}`;
}

/**
 * Returns `true` if the user has already made an explicit choice for this
 * identity (either opted in or opted out).  When `false`, the seeding path
 * may fire.
 */
export function hasExplicitAgentMetricArchiveChoice(
  identityPubkey: string,
): boolean {
  if (typeof window === "undefined") return true; // SSR/test: treat as set
  try {
    return window.localStorage.getItem(storageKey(identityPubkey)) !== null;
  } catch {
    return true; // storage error → treat as set, never auto-seed
  }
}

/**
 * Mark that the user has made an explicit choice for this identity.
 * `enabled` should reflect whether the `owner_p` subscription exists after
 * the action (true = seeded/enabled, false = opted out).
 */
export function setExplicitAgentMetricArchiveChoice(
  identityPubkey: string,
  enabled: boolean,
): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      storageKey(identityPubkey),
      enabled ? "1" : "0",
    );
  } catch {
    // Best-effort — the seeding guard will re-fire on next startup if storage
    // is unavailable, but that is safe (create_save_subscription is idempotent).
  }
}

/**
 * Clear the explicit choice for this identity (for testing / reset flows).
 */
export function clearExplicitAgentMetricArchiveChoice(
  identityPubkey: string,
): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(storageKey(identityPubkey));
  } catch {
    // ignore
  }
}
