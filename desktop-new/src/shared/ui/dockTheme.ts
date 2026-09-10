import type { DockviewTheme } from "dockview-react";

/**
 * The workspace's own dockview theme, one per colour scheme.
 *
 * Dockview's shipped themes are vendor appearance. `DockWorkspace` used to pass
 * `themeLight` in both modes, which painted an opaque white sheet behind the
 * panels in dark mode — the defect documented in `product.css`. A theme is
 * mode-dependent, so it is authored per mode here for the same reason a colour
 * role is.
 *
 * We keep dockview's vendor class as the base layer so every `--dv-*` variable
 * we do not override still resolves to something sane, and `.buzz-dockview` in
 * `product.css` restates the ones that must speak our language.
 *
 * `gap` is the real workspace gutter. Dockview feeds it to the splitview as a
 * margin, so the panels get air *and* the resize seam lives in that air. It
 * replaces the padding each group used to carry, which faked the same look
 * without a seam to grab.
 */
const BASE = {
  edgeGroupCollapsedSize: 44,
  /* Where the drop overlay is mounted in the DOM.

     `'absolute'` puts it in a shared container at the component root, which is
     how a target can be drawn across the gutter between two panels rather than
     clipped inside one rounded panel. */
  dndOverlayMounting: "absolute",
  /* Which box the drop overlay is drawn into.

     `'group'` mounts it in the group element's *parent*, so it can cover the
     header as well as the content. That is right for a zero-gap dock where the
     parent and the panel are the same box — and wrong here. With a gutter the
     parent is the grid cell, which is larger than the panel and offset from it,
     so the highlight was drawn in one coordinate space while the drop resolved
     in another: a target would appear over the right-hand side of a panel and
     the panel would then not move. That is the "I drag it to the highlighted
     spot and it doesn't go there" defect.

     `'content'` keeps the overlay in the box whose quadrants actually decide the
     drop. The overlay no longer covers the header, which is the honest picture:
     the header is the handle, the content is the target. */
  dndPanelOverlay: "content",
  /* A spaced layout has no shared tab strip for a fill to sit in, so an
     insertion line reads more precisely than a highlighted half-tab. */
  dndTabIndicator: "line",
  /* Tab reorder animation. Dockview's `smooth` transitions tab positions
     during the drag, which DESIGN.md § Motion rules out: direct manipulation
     follows the pointer with no easing. */
  tabAnimation: "default",
} as const satisfies Partial<DockviewTheme>;

/**
 * Resolve the gutter from the token rather than restating it as a number.
 *
 * Dockview needs pixels, and `--space-panel-gap` is authored in rem, so this is
 * a genuine unit boundary rather than a duplicated value. Reading the computed
 * value keeps `tokens.css` the one place the gutter is decided; hardcoding `8`
 * here would create a second answer that silently drifts.
 */
export function readPanelGap(element: Element): number {
  const value = getComputedStyle(element).getPropertyValue("--space-panel-gap");
  const resolved = Number.parseFloat(value);
  if (Number.isFinite(resolved)) {
    return value.trim().endsWith("rem")
      ? resolved *
          Number.parseFloat(getComputedStyle(document.documentElement).fontSize)
      : resolved;
  }
  return 0;
}

export function dockTheme(
  scheme: "light" | "dark",
  gap: number,
): DockviewTheme {
  return {
    ...BASE,
    name: `buzz-${scheme}`,
    className: `dockview-theme-${scheme}`,
    colorScheme: scheme,
    gap,
  };
}
