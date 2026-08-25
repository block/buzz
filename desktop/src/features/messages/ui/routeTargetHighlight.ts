/**
 * Classes that paint the "you were sent here" tint on a timeline row.
 *
 * The tint is drawn on a `before:` pseudo-element sized to the row's own
 * hover pill (same `rounded-2xl` radius, no margin/padding change), so turning
 * the highlight on or off never alters the row's geometry. In the virtualized
 * timeline a geometry change would rewrap text, change the row's measured
 * height, and make Virtua nudge the scroll position — once on arrival and
 * again when the highlight clears.
 *
 * Hosts must be `relative` and use the `rounded-2xl` hover pill geometry.
 */
export const ROUTE_TARGET_HIGHLIGHT_CLASS =
  "before:pointer-events-none before:absolute before:inset-0 before:rounded-2xl before:animate-[route-target-highlight-fade_2s_ease-out_forwards] before:bg-primary/10 before:content-[''] motion-reduce:before:animate-none";
