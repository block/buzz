/**
 * Best-effort time-based cleanup for disposable localStorage caches.
 *
 * Only explicitly whitelisted cache namespaces are eligible. Durable state
 * such as identities, communities, read positions, onboarding, and preferences
 * must never be added here.
 */

const DAY_MS = 24 * 60 * 60 * 1_000;
const SWEEP_INTERVAL_MS = 60 * 60 * 1_000;
/** Minimum time after app boot before the first sweep begins. */
export const BOOT_SWEEP_FLOOR_MS = 30_000;
/**
 * Generous idle-callback ceiling. Unlike the original 1 500 ms value this is
 * not a forcing timeout that lands during busy startup — the browser schedules
 * the callback when the main thread is genuinely idle.
 */
export const SWEEP_IDLE_TIMEOUT_MS = 60_000;
/** Per-slice setTimeout delay in environments without requestIdleCallback. */
export const SWEEP_FALLBACK_TIMER_MS = 250;
/**
 * Maximum number of localStorage entries to parse per chunk when an
 * IdleDeadline is unavailable (rIC-free fallback path).
 */
export const SWEEP_CHUNK_ENTRY_BUDGET = 20;
/**
 * Values larger than this threshold are deferred to the next slice when the
 * idle deadline has already expired (timeRemaining() <= 0) and the key has
 * not been previously deferred. Each key is deferred at most once, guaranteeing
 * convergence: every slice either parses ≥1 entry or grows deferredKeys by
 * exactly one (a set bounded by the snapshot size). Both quantities are finite,
 * so the sweep always terminates.
 */
export const SWEEP_LARGE_VALUE_DEFER_BYTES = 128 * 1024;

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

/** Returns true when the entry is older than its rule TTL at `now`. */
function isStale(
  updatedAt: number | null,
  rule: LocalStorageSweepRule,
  now: number,
): boolean {
  return updatedAt !== null && updatedAt <= now - rule.maxAgeMs;
}

/**
 * Removes whitelisted cache entries older than their configured TTL.
 * Entries without a trustworthy `updatedAt` are left alone rather than guessed
 * stale. Storage and parse failures never escape into app startup.
 *
 * This synchronous form is preserved for direct test use. The scheduler calls
 * sweepChunked instead, which splits the same work across idle-time slices.
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
      if (isStale(updatedAt, rule, now)) {
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

type SliceCallback = (deadline?: { timeRemaining(): number }) => void;

/**
 * Snapshots matching key names synchronously (cheap — no parsing), then
 * processes entries across successive idle-time slices. Keys removed between
 * the snapshot and their slice are silently skipped via a re-getItem check.
 *
 * `isAlive` is checked at the start of each slice; returning false aborts
 * further processing without removing accumulated stale keys.
 *
 * Returns a cancel function that cancels any pending scheduled-slice handle.
 * The `isAlive` flag remains the correctness backstop even after cancellation.
 */
function sweepChunked(now: number, isAlive: () => boolean): () => void {
  type SliceHandle =
    | { kind: "idle"; id: number }
    | { kind: "timeout"; id: ReturnType<typeof globalThis.setTimeout> };

  let currentHandle: SliceHandle | null = null;

  function scheduleSlice(fn: SliceCallback): void {
    if ("requestIdleCallback" in window) {
      const id = window.requestIdleCallback(fn as IdleRequestCallback, {
        timeout: SWEEP_IDLE_TIMEOUT_MS,
      });
      currentHandle = { kind: "idle", id };
    } else {
      const id = globalThis.setTimeout(fn, SWEEP_FALLBACK_TIMER_MS);
      currentHandle = { kind: "timeout", id };
    }
  }

  function cancelHandle(): void {
    if (currentHandle === null) return;
    try {
      if (currentHandle.kind === "idle") {
        window.cancelIdleCallback(currentHandle.id);
      } else {
        globalThis.clearTimeout(currentHandle.id);
      }
    } catch {
      // Non-fatal: handle may already be consumed.
    }
    currentHandle = null;
  }

  // Step 1: cheap key-only snapshot — no getItem, no parsing.
  let pendingKeys: string[];
  try {
    const storage = window.localStorage;
    pendingKeys = [];
    for (let i = 0; i < storage.length; i++) {
      const key = storage.key(i);
      if (!key) continue;
      if (LOCAL_STORAGE_SWEEP_RULES.some((r) => key.startsWith(r.keyPrefix))) {
        pendingKeys.push(key);
      }
    }
  } catch (err) {
    console.warn("[localStorageSweep] key snapshot failed:", err);
    return cancelHandle;
  }

  if (!pendingKeys.length) return cancelHandle;

  // Keys judged stale during slices, accumulated for batch removal at the end.
  // If the sweep is stopped mid-flight (alive=false), these are intentionally
  // dropped — the next hourly sweep will re-detect them.
  const staleKeys: string[] = [];
  // Keys that have been deferred once due to large value + zero budget;
  // a key in this set is always parsed on its second encounter regardless of budget.
  const deferredKeys = new Set<string>();
  let pos = 0;
  let warnedThisSweep = false;

  // Step 2: parse entries in idle-time slices; reschedule until all done.
  const processSlice: SliceCallback = (deadline) => {
    // Consume the handle — we are now inside the callback, so it has fired.
    currentHandle = null;

    if (!isAlive()) return;

    try {
      const storage = window.localStorage;
      const sliceStart = pos;

      while (pos < pendingKeys.length) {
        // Always process at least one entry per slice: a timeout-fired idle
        // callback reports timeRemaining() === 0, and breaking before any
        // progress would reschedule forever on a persistently busy thread.
        const processedInSlice = pos - sliceStart;
        if (deadline && processedInSlice > 0 && deadline.timeRemaining() <= 0) {
          break;
        }
        if (!deadline && processedInSlice >= SWEEP_CHUNK_ENTRY_BUDGET) break;

        const key = pendingKeys[pos++];

        try {
          const value = storage.getItem(key);
          if (value === null) continue; // deleted since snapshot — skip

          // Defer-once: on a zero-budget slice, avoid parsing large values
          // that would cause a jank frame. Move the key to the back of the
          // queue and mark it deferred; on second encounter parse regardless.
          // Invariant: each slice either parses ≥1 entry or grows deferredKeys
          // by exactly one — both are bounded by pendingKeys.length, so the
          // sweep always converges.
          if (
            deadline &&
            deadline.timeRemaining() <= 0 &&
            value.length > SWEEP_LARGE_VALUE_DEFER_BYTES &&
            !deferredKeys.has(key)
          ) {
            pendingKeys.push(key); // move to back of queue
            deferredKeys.add(key); // will not be deferred a second time
            break; // reschedule; deferredKeys grew, so progress was made
          }

          const rule = LOCAL_STORAGE_SWEEP_RULES.find((r) =>
            key.startsWith(r.keyPrefix),
          );
          if (!rule) continue;
          const updatedAt = updatedAtFromJson(value);
          if (isStale(updatedAt, rule, now)) {
            staleKeys.push(key);
          }
        } catch (err) {
          // Per-key errors skip the entry; remaining keys are still processed.
          // Log at most once per sweep to avoid flooding on a corrupt store.
          if (!warnedThisSweep) {
            console.warn(
              "[localStorageSweep] key read failed, continuing sweep:",
              err,
            );
            warnedThisSweep = true;
          }
        }
      }

      if (pos < pendingKeys.length) {
        scheduleSlice(processSlice);
        return;
      }

      // All entries processed — batch-remove stale keys.
      // Re-check each key before removing: app code may have rewritten it
      // fresh between its slice judgment and this final step.
      for (const key of staleKeys) {
        try {
          const currentValue = storage.getItem(key);
          if (currentValue === null) continue; // already gone
          const rule = LOCAL_STORAGE_SWEEP_RULES.find((r) =>
            key.startsWith(r.keyPrefix),
          );
          if (!rule) continue;
          const currentUpdatedAt = updatedAtFromJson(currentValue);
          if (!isStale(currentUpdatedAt, rule, now)) continue; // rewritten fresh
          storage.removeItem(key);
        } catch {
          // Non-fatal: key may have been concurrently removed by another tab.
        }
      }
    } catch (err) {
      // Last-resort catch: reached only if localStorage itself becomes
      // unavailable mid-slice (e.g. SecurityError). Stale keys accumulated so
      // far are dropped; the next hourly sweep re-detects them.
      console.warn("[localStorageSweep] slice aborted unexpectedly:", err);
    }
  };

  scheduleSlice(processSlice);

  return cancelHandle;
}

/**
 * Schedules background sweeps:
 * - First sweep is delayed by BOOT_SWEEP_FLOOR_MS (30 s) so it never lands
 *   on the startup critical path. After the floor the browser picks the
 *   actual execution time (no forcing timeout).
 * - Subsequent sweeps run hourly via setInterval.
 * - The hidden→visible trigger has been removed. It stacked the sweep onto
 *   the exact moment focus-refetch storms fire. Hourly + boot-delayed covers
 *   the TTL contract — all rule TTLs are 14 days.
 *
 * Returns a cleanup function for tests or future teardown.
 */
export function startLocalStorageSweep(): () => void {
  let alive = true;
  let intervalId: ReturnType<typeof window.setInterval> | null = null;
  let bootTimeoutId: ReturnType<typeof globalThis.setTimeout> | null = null;
  let cancelCurrentSweep: (() => void) | null = null;

  const runSweep = () => {
    cancelCurrentSweep?.();
    cancelCurrentSweep = sweepChunked(Date.now(), () => alive);
  };

  try {
    bootTimeoutId = globalThis.setTimeout(runSweep, BOOT_SWEEP_FLOOR_MS);
    intervalId = window.setInterval(runSweep, SWEEP_INTERVAL_MS);
  } catch (error) {
    console.warn("[localStorageSweep] scheduler setup failed:", error);
  }

  return () => {
    alive = false;
    try {
      if (bootTimeoutId !== null) globalThis.clearTimeout(bootTimeoutId);
      if (intervalId !== null) window.clearInterval(intervalId);
      cancelCurrentSweep?.();
      cancelCurrentSweep = null;
    } catch (error) {
      console.warn("[localStorageSweep] scheduler cleanup failed:", error);
    }
  };
}
