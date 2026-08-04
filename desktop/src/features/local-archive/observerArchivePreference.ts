/**
 * Persists whether the user has made an explicit choice about the
 * observer-frame archive default-on feature.
 *
 * The key is identity-scoped so toggling off on one identity doesn't suppress
 * the default-on for another identity.  The value is:
 *   "1"  → user explicitly enabled (or accepted the default)
 *   "0"  → user explicitly disabled
 *   null → no explicit choice yet (default-on seeding may still fire)
 *
 * Device-level localStorage — intentionally not reset on community switch
 * (the archive subscription itself is identity-scoped in SQLite; this flag
 * is just the UI gate that prevents re-seeding after an explicit opt-out).
 *
 * Storage-error contract: a single read that throws is treated the same as
 * a stored "1" (treat-as-set, fail-closed). This matches the metric-archive
 * path: a storage error must never cause the seeding guard to fire or allow
 * a stored opt-out to be silently overridden.
 */

const KEY_PREFIX = "buzz:observer-archive-default-seeded";

function storageKey(identityPubkey: string): string {
  return `${KEY_PREFIX}:${identityPubkey}`;
}

/**
 * Reads the stored explicit choice for this identity in a single localStorage
 * access.
 *
 * Returns:
 *   `false`    — user explicitly opted out ("0" stored)
 *   `true`     — user explicitly opted in ("1" stored)
 *   `"unset"`  — no choice recorded yet
 *
 * On storage error, returns `true` (fail-closed: treat as already opted in,
 * suppress auto-seeding, and never override a potentially stored opt-out).
 */
export function readExplicitObserverArchiveChoice(
  identityPubkey: string,
): boolean | "unset" {
  if (typeof window === "undefined") return true; // SSR/test: treat as set
  try {
    const raw = window.localStorage.getItem(storageKey(identityPubkey));
    if (raw === null) return "unset";
    return raw !== "0";
  } catch {
    return true; // storage error → treat as set, never auto-seed
  }
}

/**
 * Mark that the user has made an explicit choice for this identity.
 * `enabled` should reflect whether the `owner_p` subscription exists after
 * the action (true = seeded/enabled, false = opted out).
 */
export function setExplicitObserverArchiveChoice(
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
    // is unavailable, but that is safe (merge_save_subscription_kinds is idempotent).
  }
}

/**
 * Clear the explicit choice for this identity (for testing / reset flows).
 */
export function clearExplicitObserverArchiveChoice(
  identityPubkey: string,
): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(storageKey(identityPubkey));
  } catch {
    // ignore
  }
}
