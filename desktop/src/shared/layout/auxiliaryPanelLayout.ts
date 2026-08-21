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
