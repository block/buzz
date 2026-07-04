/**
 * Quota-aware localStorage writes with pure-cache eviction recovery.
 *
 * The desktop webview caps localStorage at ~5 MB per origin. Writers of
 * load-bearing state (read-state, workspaces) must not leak QuotaExceededError
 * into React click/render paths. On a failed write this evicts snapshot caches
 * — safe to drop, they repaint from the relay — and retries the write once.
 */

const PURE_CACHE_KEY_PREFIXES = [
  "buzz-channel-messages.v1",
  "buzz-channels.v1",
  "buzz-timeline-skeleton-shape.v1",
];

function evictPureCacheEntries(): number {
  const toRemove: string[] = [];
  for (let i = 0; i < window.localStorage.length; i++) {
    const key = window.localStorage.key(i);
    if (
      key !== null &&
      PURE_CACHE_KEY_PREFIXES.some((prefix) => key.startsWith(prefix))
    ) {
      toRemove.push(key);
    }
  }
  for (const key of toRemove) {
    window.localStorage.removeItem(key);
  }
  return toRemove.length;
}

let warnedPersistentFailure = false;

function notifyStorageFull(): void {
  if (warnedPersistentFailure) return;
  warnedPersistentFailure = true;
  // Dynamic import keeps this module usable from node unit tests.
  import("sonner")
    .then(({ toast }) => {
      toast.error("Local storage is full", {
        description:
          "Buzz could not save some local data — read positions may not persist across restarts.",
      });
    })
    .catch(() => {});
}

/**
 * Writes to localStorage; on failure (quota exceeded), evicts pure snapshot
 * caches and retries once. Returns false when the write still fails — callers
 * keep working from in-memory state.
 */
export function setLocalStorageItemWithRecovery(
  key: string,
  value: string,
): boolean {
  try {
    window.localStorage.setItem(key, value);
    return true;
  } catch (error) {
    try {
      if (evictPureCacheEntries() > 0) {
        window.localStorage.setItem(key, value);
        return true;
      }
    } catch {
      // Fall through to failure reporting.
    }
    console.warn(
      "[localStorageQuota] write failed after cache eviction:",
      key,
      error,
    );
    notifyStorageFull();
    return false;
  }
}
