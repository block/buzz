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
 * drawer). The responsive gutter keeps compact focus drawers usable while
 * preserving generous whitespace at desktop widths. The max-width itself is
 * applied inline since it is a caller-provided value.
 */
export const THREAD_PANEL_COLUMN_CLASS = "mx-auto w-full px-6 sm:px-10";
