/**
 * Layout constants for the focus-mode thread drawer.
 *
 * Focus mode overlays the channel content area with a right-anchored drawer
 * rather than splitting the row into two resizable panes.
 */

/**
 * Max width of the centered message column inside the focus drawer.
 *
 * The drawer itself spans nearly the whole channel content area, but message
 * text set that wide is unreadable. The list and composer share this max width
 * with auto horizontal margins so the reading measure stays comfortable no
 * matter how wide the window gets.
 */
export const THREAD_FOCUS_COLUMN_MAX_WIDTH_PX = 880;

/**
 * `AnimatePresence` key shared by both thread layouts.
 *
 * The split pane and the focus drawer are two containers for one thread, so
 * presence is a property of the thread, not of either container. Keying them
 * apart would make every view-mode switch read as a close followed by an open.
 */
export const THREAD_SURFACE_KEY = "message-thread-surface";
