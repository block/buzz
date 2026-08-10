/**
 * Best-effort time-based cleanup for disposable localStorage caches.
 *
 * Only explicitly whitelisted cache namespaces are eligible. Durable state
 * such as identities, communities, read positions, onboarding, and preferences
 * must never be added here.
 */

const DAY_MS = 24 * 60 * 60 * 1_000;
const SWEEP_INTERVAL_MS = 60 * 60 * 1_000;
const SWEEP_DEBOUNCE_MS = 5 * 60 * 1_000;
const INITIAL_SWEEP_FALLBACK_MS = 250;
const INITIAL_SWEEP_IDLE_TIMEOUT_MS = 1_500;

type LocalStorageSweepRule = {
  keyPrefix: string;
  maxAgeMs: number;
};

/** Disposable cache namespaces and their maximum idle age. */
export const LOCAL_STORAGE_SWEEP_RULES: readonly LocalStorageSweepRule[] = [
  { keyPrefix: "buzz-channel-messages.v1:", maxAgeMs: 14 * DAY_MS },
  { keyPrefix: "buzz-channels.v1:", maxAgeMs: 14 * DAY_MS },
  { keyPrefix: "buzz-observed-unread.v1:", maxAgeMs: 14 * DAY_MS },
  { keyPrefix: "buzz-sidebar-skeleton-shape.v1:", maxAgeMs: 14 * DAY_MS },
  { keyPrefix: "buzz-timeline-skeleton-shape.v1:", maxAgeMs: 14 * DAY_MS },
  { keyPrefix: "buzz-user-labels.v1:", maxAgeMs: 14 * DAY_MS },
  // Do not add buzz-self-profile.v1: here. It is the load-bearing offline
  // identity fallback when the relay is unreachable, not a repaintable cache.
];

function updatedAtFromJson(value: string): number | null {
  try {
    const parsed = JSON.parse(value) as unknown;
    if (typeof parsed !== "object" || parsed === null) return null;
    const record = parsed as Record<string, unknown>;
    if (
      typeof record.updatedAt === "number" &&
      Number.isFinite(record.updatedAt)
    ) {
      return record.updatedAt;
    }

    // User-label cache buckets carry freshness per profile instead of at the
    // payload root. Use the newest valid label timestamp so the bucket is only
    // removed once every label in it is stale.
    if (typeof record.profiles !== "object" || record.profiles === null) {
      return null;
    }
    let newestUpdatedAt: number | null = null;
    for (const profile of Object.values(record.profiles)) {
      if (typeof profile !== "object" || profile === null) continue;
      const updatedAt = (profile as Record<string, unknown>).updatedAt;
      if (
        typeof updatedAt === "number" &&
        Number.isFinite(updatedAt) &&
        (newestUpdatedAt === null || updatedAt > newestUpdatedAt)
      ) {
        newestUpdatedAt = updatedAt;
      }
    }
    return newestUpdatedAt;
  } catch {
    return null;
  }
}

/**
 * Removes whitelisted cache entries older than their configured TTL.
 * Entries without a trustworthy `updatedAt` are left alone rather than guessed
 * stale. Storage and parse failures never escape into app startup.
 */
export function sweepStaleLocalStorage(now = Date.now()): number {
  let removed = 0;
  try {
    const storage = window.localStorage;
    const staleKeys: string[] = [];

    for (let i = 0; i < storage.length; i++) {
      const key = storage.key(i);
      if (key === null) continue;
      const rule = LOCAL_STORAGE_SWEEP_RULES.find(({ keyPrefix }) =>
        key.startsWith(keyPrefix),
      );
      if (!rule) continue;

      const value = storage.getItem(key);
      if (value === null) continue;
      const updatedAt = updatedAtFromJson(value);
      if (updatedAt !== null && updatedAt <= now - rule.maxAgeMs) {
        staleKeys.push(key);
      }
    }

    // Collect before mutating because localStorage indexes shift on removal.
    for (const key of staleKeys) {
      storage.removeItem(key);
      removed++;
    }
  } catch (error) {
    console.warn("[localStorageSweep] stale cache cleanup failed:", error);
  }
  return removed;
}

/**
 * Defers the first sweep until the browser is idle (or a short timer fallback),
 * then sweeps hourly while the app remains open and when a hidden app becomes
 * visible. Visibility sweeps are debounced to avoid repeated work from rapid
 * focus changes. Returns a cleanup function for tests or future teardown.
 */
export function startLocalStorageSweep(): () => void {
  let lastSweepAt = Number.NEGATIVE_INFINITY;
  let listening = false;
  let intervalId: ReturnType<typeof window.setInterval> | null = null;
  let idleCallbackId: number | null = null;
  let timeoutId: ReturnType<typeof globalThis.setTimeout> | null = null;
  const runIfDue = () => {
    const now = Date.now();
    if (now - lastSweepAt < SWEEP_DEBOUNCE_MS) return;
    lastSweepAt = now;
    sweepStaleLocalStorage(now);
  };
  const onVisibilityChange = () => {
    if (document.visibilityState === "visible") runIfDue();
  };

  try {
    document.addEventListener("visibilitychange", onVisibilityChange);
    listening = true;
    intervalId = window.setInterval(runIfDue, SWEEP_INTERVAL_MS);
    if ("requestIdleCallback" in window) {
      idleCallbackId = window.requestIdleCallback(runIfDue, {
        timeout: INITIAL_SWEEP_IDLE_TIMEOUT_MS,
      });
    } else {
      timeoutId = globalThis.setTimeout(runIfDue, INITIAL_SWEEP_FALLBACK_MS);
    }
  } catch (error) {
    console.warn("[localStorageSweep] scheduler setup failed:", error);
  }

  return () => {
    try {
      if (listening) {
        document.removeEventListener("visibilitychange", onVisibilityChange);
      }
      if (intervalId !== null) window.clearInterval(intervalId);
      if (idleCallbackId !== null && "cancelIdleCallback" in window) {
        window.cancelIdleCallback(idleCallbackId);
      }
      if (timeoutId !== null) globalThis.clearTimeout(timeoutId);
    } catch (error) {
      console.warn("[localStorageSweep] scheduler cleanup failed:", error);
    }
  };
}
