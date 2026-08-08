/**
 * Quota-aware localStorage writes with pure-cache eviction recovery.
 *
 * The desktop webview caps localStorage at ~5 MB per origin. Writers of
 * load-bearing state (read-state, communities) must not leak QuotaExceededError
 * into React click/render paths. On a failed write this evicts snapshot caches
 * — safe to drop, they repaint from the relay — and retries the write once.
 */

const PURE_CACHE_KEY_PREFIXES = [
  "buzz-channel-messages.v1:",
  "buzz-channels.v1:",
  "buzz-sidebar-skeleton-shape.v1:",
  "buzz-timeline-skeleton-shape.v1:",
];

const QUOTA_RECOVERY_MARKER_KEY = "buzz-local-storage-quota-recovery.v1";

// Keep disposable snapshots below 2 MiB, leaving roughly 3 MiB of WebKit's
// observed ~5 MiB origin quota for identities, communities, preferences, and
// read state. localStorage strings are UTF-16, so count two bytes per code unit.
const PURE_CACHE_BYTE_BUDGET = 2 * 1024 * 1024;

function isPureCacheKey(key: string): boolean {
  return PURE_CACHE_KEY_PREFIXES.some((prefix) => key.startsWith(prefix));
}

function storageEntryBytes(key: string, value: string): number {
  return (key.length + value.length) * 2;
}

function evictPureCacheEntries(): number {
  const toRemove: string[] = [];
  for (let i = 0; i < window.localStorage.length; i++) {
    const key = window.localStorage.key(i);
    if (key !== null && isPureCacheKey(key)) {
      toRemove.push(key);
    }
  }
  for (const key of toRemove) {
    window.localStorage.removeItem(key);
  }
  return toRemove.length;
}

function pureCacheEntriesExcluding(excludedKey: string): Array<{
  bytes: number;
  key: string;
  updatedAt: number;
}> {
  const entries: Array<{ bytes: number; key: string; updatedAt: number }> = [];
  for (let i = 0; i < window.localStorage.length; i++) {
    const key = window.localStorage.key(i);
    if (key === null || key === excludedKey || !isPureCacheKey(key)) continue;
    const value = window.localStorage.getItem(key);
    if (value === null) continue;

    let updatedAt = 0;
    try {
      const parsed = JSON.parse(value) as { updatedAt?: unknown };
      if (
        typeof parsed.updatedAt === "number" &&
        Number.isFinite(parsed.updatedAt)
      ) {
        updatedAt = parsed.updatedAt;
      }
    } catch {
      // Legacy or malformed cache entries are safe to evict first.
    }
    entries.push({ bytes: storageEntryBytes(key, value), key, updatedAt });
  }
  return entries;
}

function trimPureCacheForWrite(key: string, writeBytes: number): void {
  const entries = pureCacheEntriesExcluding(key);
  let totalBytes = entries.reduce((total, entry) => total + entry.bytes, 0);
  if (totalBytes + writeBytes <= PURE_CACHE_BYTE_BUDGET) return;

  entries.sort((a, b) => a.updatedAt - b.updatedAt);
  for (const entry of entries) {
    window.localStorage.removeItem(entry.key);
    totalBytes -= entry.bytes;
    if (totalBytes + writeBytes <= PURE_CACHE_BYTE_BUDGET) return;
  }
}

function preparePureCacheWrite(key: string, value: string): boolean {
  if (!isPureCacheKey(key)) return true;

  const writeBytes = storageEntryBytes(key, value);
  if (writeBytes > PURE_CACHE_BYTE_BUDGET) {
    window.localStorage.removeItem(key);
    return false;
  }

  trimPureCacheForWrite(key, writeBytes);
  return true;
}

/**
 * Probes storage once at startup so installs that already filled WebKit
 * localStorage recover before app initialization. Healthy installs keep their
 * caches; only a failed marker write triggers disposable-cache eviction. The
 * marker is written after recovery, so an interrupted/unavailable backend
 * retries on the next launch. Load-bearing state is never touched.
 */
export function recoverLocalStorageQuotaOnStartup(): void {
  try {
    if (window.localStorage.getItem(QUOTA_RECOVERY_MARKER_KEY) === "1") return;
    try {
      window.localStorage.setItem(QUOTA_RECOVERY_MARKER_KEY, "1");
      return;
    } catch {
      evictPureCacheEntries();
    }
    window.localStorage.setItem(QUOTA_RECOVERY_MARKER_KEY, "1");
  } catch (error) {
    console.warn("[localStorageQuota] startup cache cleanup failed:", error);
  }
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
    if (!preparePureCacheWrite(key, value)) return false;
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
