export const DEPRECATION_BANNER_STORAGE_KEY =
  "buzz.deprecationBanner.dismissedAt";
export const DEPRECATION_BANNER_SNOOZE_MS = 24 * 60 * 60 * 1000;

/** Shown unless dismissed less than a day ago. Evaluated once per mount. */
export function shouldShowDeprecationBanner(
  dismissedAt: string | null,
  now: number,
): boolean {
  const dismissedAtMs = Number(dismissedAt);
  if (!dismissedAt || !Number.isFinite(dismissedAtMs)) {
    return true;
  }
  return now - dismissedAtMs >= DEPRECATION_BANNER_SNOOZE_MS;
}
