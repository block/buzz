export const AUXILIARY_PANEL_DEFAULT_WIDTH_PX = 380;
export const AUXILIARY_PANEL_MIN_WIDTH_PX = 300;
export const AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX =
  AUXILIARY_PANEL_MIN_WIDTH_PX * 2;
export const AUXILIARY_PANEL_MAX_WIDTH_PX = 720;

/**
 * Upper bound for the auxiliary panel width clamp, given the current viewport width.
 *
 * On ultrawide displays the static {@link AUXILIARY_PANEL_MAX_WIDTH_PX} is too small,
 * so the panel is allowed to grow with the viewport while always reserving at least
 * {@link AUXILIARY_PANEL_MIN_WIDTH_PX} for the main pane. The static cap acts as a
 * floor, so narrow viewports keep their existing behavior.
 */
export function getAuxiliaryPanelMaxWidth(viewportWidth: number): number {
  return Math.max(
    AUXILIARY_PANEL_MAX_WIDTH_PX,
    viewportWidth - AUXILIARY_PANEL_MIN_WIDTH_PX,
  );
}

/** Clamp a stored panel width into the allowed range for the current viewport. */
export function clampAuxiliaryPanelWidth(
  width: number,
  viewportWidth: number,
): number {
  return Math.max(
    AUXILIARY_PANEL_MIN_WIDTH_PX,
    Math.min(getAuxiliaryPanelMaxWidth(viewportWidth), width),
  );
}

/** Resolve the CSS width without reserving main-pane space for overlays. */
export function resolveAuxiliaryPanelWidth({
  floatingOverlay,
  singlePanelView,
  splitPaneClamp,
  widthPx,
}: {
  floatingOverlay: boolean;
  singlePanelView: boolean;
  splitPaneClamp: boolean;
  widthPx: number;
}): string {
  if (singlePanelView) return "100%";
  if (floatingOverlay || !splitPaneClamp) return `${widthPx}px`;
  return `min(${widthPx}px, calc(100% - ${AUXILIARY_PANEL_MIN_WIDTH_PX}px))`;
}
