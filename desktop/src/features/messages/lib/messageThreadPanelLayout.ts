/**
 * Shared inline-gutter classes for the thread panel, so the real panel and its
 * loading skeleton stay pixel-aligned as content swaps in.
 */

/** Inline gutter around thread message rows. */
export const THREAD_PANEL_MESSAGE_GUTTER_CLASS = "px-2";

/** Inline gutter around the thread composer and its activity row. */
export const THREAD_PANEL_COMPOSER_GUTTER_CLASS = "px-5";

/**
 * Centers the reading column when a `columnMaxWidthPx` is supplied (focus-mode
 * drawer). `px-10` (40px) is the inline gutter between the column and the drawer
 * edges; the max-width itself is applied inline since it is a caller-provided
 * pixel value.
 */
export const THREAD_PANEL_COLUMN_CLASS = "mx-auto w-full px-10";
