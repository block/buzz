/** Quiet period after the last new message before an auto-scan fires. */
export const AUTO_SCAN_DEBOUNCE_MS = 10_000;

/**
 * Ceiling on how long debouncing may defer a scan. Without it, a channel that
 * never goes quiet would postpone triage indefinitely.
 */
export const AUTO_SCAN_MAX_WAIT_MS = 45_000;

/**
 * Delay before the next auto-scan, or `null` when nothing is waiting to be
 * triaged. The debounce collapses a burst of arrivals into one scan; the
 * remaining budget shrinks as the oldest pending message ages, so a steady
 * stream still gets scanned within {@link AUTO_SCAN_MAX_WAIT_MS}.
 */
export function computeAutoScanDelay({
  pendingCount,
  waitedMs,
}: {
  pendingCount: number;
  waitedMs: number;
}): number | null {
  if (pendingCount <= 0) {
    return null;
  }

  const remainingBudget = AUTO_SCAN_MAX_WAIT_MS - waitedMs;
  return Math.max(0, Math.min(AUTO_SCAN_DEBOUNCE_MS, remainingBudget));
}
