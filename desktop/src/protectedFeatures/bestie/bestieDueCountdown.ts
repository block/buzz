/**
 * Rough due-countdown labels for Bestie Reminders / Jobs rows.
 *
 * - Minutes while remaining > 1m (no live second ticker).
 * - Seconds only when under a minute.
 * - Hidden when farther than the chip horizon (default 60m).
 */

export const DEFAULT_BESTIE_DUE_CHIP_HORIZON_MINUTES = 60;

/** Format a countdown chip label, or null when outside the horizon. */
export function formatBestieDueCountdown(
  dueAtSeconds: number,
  nowSeconds: number,
  horizonSeconds: number,
): string | null {
  if (!Number.isFinite(dueAtSeconds) || !Number.isFinite(nowSeconds)) {
    return null;
  }
  if (!(horizonSeconds > 0)) return null;
  const remaining = Math.floor(dueAtSeconds) - Math.floor(nowSeconds);
  if (remaining > horizonSeconds) return null;
  if (remaining <= 0) return "due";
  if (remaining < 60) return `${remaining}s`;
  // Rough minute bucket — round so ~90s reads "2m", ~7.4m reads "7m".
  const minutes = Math.max(1, Math.round(remaining / 60));
  return `${minutes}m`;
}

/**
 * Next timer delay for refreshing the chip.
 * - Beyond horizon: wake when entering (capped).
 * - Under 1m: 1s ticks.
 * - Minutes: coarse ~15s (not a live second ticker).
 * - Already due / disabled: null (no timer).
 */
export function bestieDueCountdownTickMs(
  dueAtSeconds: number,
  nowSeconds: number,
  horizonSeconds: number,
): number | null {
  if (!(horizonSeconds > 0)) return null;
  const remaining = Math.floor(dueAtSeconds) - Math.floor(nowSeconds);
  if (remaining <= 0) return null;
  if (remaining > horizonSeconds) {
    const untilHorizon = (remaining - horizonSeconds) * 1000;
    return Math.min(Math.max(untilHorizon, 1000), 60_000);
  }
  if (remaining <= 60) return 1000;
  return 15_000;
}
