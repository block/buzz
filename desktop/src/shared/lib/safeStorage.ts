/**
 * Throw-safe localStorage accessors.
 *
 * WKWebView throws `SecurityError` from `localStorage.getItem` (not just
 * `setItem`) when storage access is denied for the origin — e.g. the user
 * disabled website data, or the Tauri webview runs under a restricted
 * storage policy. A raw `window.localStorage.getItem(...)` executed inside
 * a React `useState`/`useMemo` initializer or provider render propagates
 * that throw up the reconcile path and unmounts the whole tree (there is
 * no `ErrorBoundary` in `desktop/src`), leaving a dead window.
 *
 * These helpers make reads fail-closed (return the fallback / `null`) and
 * writes fail-silently-with-a-warning, preserving the app's ability to start
 * and degrade to in-memory state instead of crashing to a blank screen.
 *
 * Issue contexts: block/buzz#5078.
 */

// Keep the console noise down on repeated reads — one warn per (action, key)
// pair per process is enough. This also keeps unit tests deterministic.
const WARNED_KEYS = new Set<string>();

function warnOnce(action: "read" | "write", key: string, error: unknown): void {
  if (WARNED_KEYS.has(`${action}:${key}`)) return;
  WARNED_KEYS.add(`${action}:${key}`);
  console.warn(
    `[safeStorage] localStorage.${action === "read" ? "getItem" : "setItem"} threw for key "${key}" (storage access denied?):`,
    error,
  );
}

/** Reset the warn-once dedup — test-only helper. */
export function __resetSafeStorageWarningsForTests(): void {
  WARNED_KEYS.clear();
}

/**
 * Read a localStorage entry, resolving `fallback` when the underlying call
 * throws (e.g. WebKit `SecurityError` under a restricted storage policy) or
 * storage is unavailable. Never throws.
 */
export function getStorageItem(
  key: string,
  fallback: string | null = null,
): string | null {
  try {
    return window.localStorage.getItem(key) ?? fallback;
  } catch (error) {
    warnOnce("read", key, error);
    return fallback;
  }
}

/**
 * Write a localStorage entry; returns `false` when the underlying call throws
 * (quota exceeded or storage denied) instead of propagating. Prefer
 * `setLocalStorageItemWithRecovery` from `./localStorageQuota` for writes that
 * need cache-eviction recovery — this wrapper exists for call sites that only
 * need the throw converted to a boolean result.
 */
export function setStorageItem(key: string, value: string): boolean {
  try {
    window.localStorage.setItem(key, value);
    return true;
  } catch (error) {
    warnOnce("write", key, error);
    return false;
  }
}

/**
 * Remove a localStorage entry; never throws. Returns `false` when removal
 * threw (treated as best-effort, matching `localStorage.removeItem` semantics
 * callers assume).
 */
export function removeStorageItem(key: string): boolean {
  try {
    window.localStorage.removeItem(key);
    return true;
  } catch (error) {
    // Reads/writes share the SecurityError class; log under the read bucket
    // because a denied-storage origin will fail all three the same way.
    warnOnce("read", key, error);
    return false;
  }
}
