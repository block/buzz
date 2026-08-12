/**
 * Shared inline-gutter classes for the thread panel, so the real panel and its
 * loading skeleton stay pixel-aligned as content swaps in.
 */

/** Inline gutter around thread message rows. */
export const THREAD_PANEL_MESSAGE_GUTTER_CLASS = "px-4";

/** Inline gutter around the thread composer and its activity row. */
export const THREAD_PANEL_COMPOSER_GUTTER_CLASS = "px-5";

/**
 * Centers the reading column when a `columnMaxWidthPx` is supplied (focus-mode
 * drawer). The responsive inline gutter keeps compact overlays usable while
 * retaining the calmer 40px edge at desktop widths; the max-width itself is
 * applied inline since it is a caller-provided pixel value.
 */
export const THREAD_PANEL_COLUMN_CLASS = "mx-auto w-full px-6 sm:px-10";
