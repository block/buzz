const CENTERED_ROW_TOLERANCE_PX = 2;

function resolveCssLength(value: string) {
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed)) return 0;
  return value.trim().endsWith("rem")
    ? parsed *
        Number.parseFloat(getComputedStyle(document.documentElement).fontSize)
    : parsed;
}

function getUsableViewportBounds(container: HTMLDivElement) {
  const containerRect = container.getBoundingClientRect();
  const styles = getComputedStyle(container);
  return {
    bottom:
      containerRect.bottom -
      resolveCssLength(styles.getPropertyValue("--composer-overlay-height")),
    top:
      containerRect.top +
      resolveCssLength(styles.getPropertyValue("--channel-top-chrome-height")),
  };
}

export function getTargetRowCenterOffset(
  row: Element,
  container: HTMLDivElement,
) {
  const rowRect = row.getBoundingClientRect();
  const viewport = getUsableViewportBounds(container);
  return (
    (rowRect.top + rowRect.bottom) / 2 - (viewport.top + viewport.bottom) / 2
  );
}

/**
 * A virtualized jump is complete only when the row's midpoint reaches the
 * viewport midpoint. Two pixels absorb fractional layout and Virtua's rounded
 * scroll offsets. Boundary rows are the intentional exceptions: the list
 * clamps the oldest row to the physical ceiling and the newest row to the
 * physical floor, where exact centering is impossible.
 */
export function isTargetRowCentered(
  row: Element,
  container: HTMLDivElement,
  boundary: "none" | "top" | "bottom",
  isAtBottom: (container: HTMLDivElement) => boolean,
) {
  const rowRect = row.getBoundingClientRect();
  if (rowRect.bottom - rowRect.top <= 0) return false;
  if (
    Math.abs(getTargetRowCenterOffset(row, container)) <=
    CENTERED_ROW_TOLERANCE_PX
  ) {
    return true;
  }
  const viewport = getUsableViewportBounds(container);
  const rowIsVisible =
    rowRect.bottom > viewport.top && rowRect.top < viewport.bottom;
  if (boundary === "top") return rowIsVisible && container.scrollTop <= 0;
  return boundary === "bottom" && rowIsVisible && isAtBottom(container);
}

export function targetRowNeedsCenterCorrection(offset: number) {
  return Math.abs(offset) > CENTERED_ROW_TOLERANCE_PX;
}
