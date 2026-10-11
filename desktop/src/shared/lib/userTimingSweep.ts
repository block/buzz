/**
 * Periodic sweep of the User Timing measure buffer for development builds.
 *
 * React's development bundle records a `performance.measure()` entry, with a
 * serialized `detail` object, for every component render (its "Components ⚛"
 * DevTools track). WebKit keeps user-timing entries for the life of the page,
 * outside the JavaScript heap, so an idle `tauri dev` session grows by tens of
 * MB per minute until the web process is OOM-killed (about 186k entries after
 * 17 minutes with one channel open). The production bundle never emits them.
 *
 * Nothing in Buzz reads a measure later: the sidebar boot measure in
 * features/channels/hooks.ts reads its own entry synchronously. Marks are left
 * alone; React does not accumulate those.
 */

export const USER_TIMING_SWEEP_INTERVAL_MS = 10_000;

type UserTimingSweepOptions = {
  enabled: boolean;
  intervalMs?: number;
  performance?: Pick<Performance, "clearMeasures">;
  setInterval?: (callback: () => void, ms: number) => number;
  clearInterval?: (id: number) => void;
};

/** Starts the sweep when enabled; returns a function that stops it. */
export function startUserTimingSweep({
  enabled,
  intervalMs = USER_TIMING_SWEEP_INTERVAL_MS,
  performance = globalThis.performance,
  setInterval = globalThis.setInterval,
  clearInterval = globalThis.clearInterval,
}: UserTimingSweepOptions): () => void {
  if (!enabled || typeof performance?.clearMeasures !== "function") {
    return () => {};
  }
  const timer = setInterval(() => {
    performance.clearMeasures();
  }, intervalMs);
  return () => {
    clearInterval(timer);
  };
}
