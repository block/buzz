/**
 * Layout constants for the focus-mode thread drawer.
 *
 * Focus mode overlays the channel content area with a right-anchored drawer
 * rather than splitting the row into two resizable panes.
 */

/**
 * Width of the channel sliver left visible to the left of the focus drawer.
 *
 * Wide enough to read a truncated `‹ #channel` label and to be a comfortable,
 * full-height click target back to the channel, but narrow enough that the
 * drawer still reads as the primary surface. The sliver keeps showing the real,
 * still-mounted channel timeline (dimmed by the scrim) so the user never loses
 * their place.
 */
export const THREAD_FOCUS_SLIVER_WIDTH_PX = 72;

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
 * Horizontal distance the focus drawer travels on enter/exit.
 *
 * Deliberately a fraction of the drawer's own width rather than a true slide
 * from off-screen: opening a thread is a high-frequency act — threads are chat
 * sessions and get flipped between constantly — and full-width travel turns a
 * routine move into ceremony. Short travel keeps it light and repeatable.
 *
 * The floor matters as much as the ceiling: the shared 24px side-panel nudge is
 * only ~3% of this drawer's width, which reads as no movement at all, leaving
 * the opacity fade as the only perceptible change. This is large enough for the
 * eye to track a direction and for the ease to have somewhere to decelerate.
 */
export const THREAD_FOCUS_DRAWER_TRAVEL_PX = 120;

/**
 * `AnimatePresence` key shared by both thread layouts.
 *
 * The split pane and the focus drawer are two containers for one thread, so
 * presence is a property of the thread, not of either container. Keying them
 * apart would make every view-mode switch read as a close followed by an open.
 */
export const THREAD_SURFACE_KEY = "message-thread-surface";
