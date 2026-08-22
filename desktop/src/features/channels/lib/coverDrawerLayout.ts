/**
 * Layout constants shared by the channel's cover drawers.
 *
 * A cover drawer overlays the channel content area with a right-anchored
 * surface rather than splitting the row into two resizable panes. Both the
 * focus-mode thread drawer and the agent activity drawer are the same
 * geometry — only their contents and their open condition differ.
 */

/**
 * Width of the channel sliver left visible to the left of a cover drawer.
 *
 * Wide enough to read a truncated `‹ #channel` label and to be a comfortable,
 * full-height click target back to the channel, but narrow enough that the
 * drawer still reads as the primary surface. The sliver keeps showing the real,
 * still-mounted channel timeline (dimmed by the scrim) so the user never loses
 * their place.
 */
export const COVER_DRAWER_SLIVER_WIDTH_PX = 72;

/**
 * Horizontal distance a cover drawer travels on enter/exit.
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
export const COVER_DRAWER_TRAVEL_PX = 120;
