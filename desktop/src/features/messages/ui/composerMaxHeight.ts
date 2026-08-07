/** Matches the previous `max-h-32` hard cap (128px). */
export const DEFAULT_COMPOSER_MAX_HEIGHT_PX = 128;

/** localStorage key — device preference, not per-channel. */
export const COMPOSER_MAX_HEIGHT_STORAGE_KEY = "buzz.composer.maxHeightPx";

/**
 * Drag-up raises the cap; never go below the default, and never above 60% of
 * the channel pane so the timeline stays usable.
 */
export function clampComposerMaxHeight(
  heightPx: number,
  paneHeightPx: number,
): number {
  const upper = Math.max(
    DEFAULT_COMPOSER_MAX_HEIGHT_PX,
    Math.floor(paneHeightPx * 0.6),
  );
  return Math.min(
    upper,
    Math.max(DEFAULT_COMPOSER_MAX_HEIGHT_PX, Math.round(heightPx)),
  );
}

export function readStoredComposerMaxHeight(): number {
  try {
    const raw = globalThis.localStorage?.getItem(
      COMPOSER_MAX_HEIGHT_STORAGE_KEY,
    );
    if (raw == null) return DEFAULT_COMPOSER_MAX_HEIGHT_PX;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) return DEFAULT_COMPOSER_MAX_HEIGHT_PX;
    // Pane height unknown at cold start — only enforce the floor.
    return Math.max(DEFAULT_COMPOSER_MAX_HEIGHT_PX, parsed);
  } catch {
    return DEFAULT_COMPOSER_MAX_HEIGHT_PX;
  }
}

export function writeStoredComposerMaxHeight(heightPx: number): void {
  try {
    globalThis.localStorage?.setItem(
      COMPOSER_MAX_HEIGHT_STORAGE_KEY,
      String(Math.round(heightPx)),
    );
  } catch {
    // Best-effort persistence.
  }
}
