/**
 * Widths for the Documents tree pane and right rail.
 *
 * Same shape as `features/home/useResizableInboxListWidth.ts` — pointer events
 * on a drag handle, clamped, persisted in sessionStorage.
 */
import * as React from "react";

const TREE_DEFAULT_WIDTH_PX = 260;
export const TREE_MIN_WIDTH_PX = 180;
const TREE_MAX_WIDTH_PX = 480;
const TREE_WIDTH_SESSION_KEY = "buzz.desktop.documents-tree-width";

const RAIL_DEFAULT_WIDTH_PX = 260;
export const RAIL_MIN_WIDTH_PX = 200;
const RAIL_MAX_WIDTH_PX = 460;
const RAIL_WIDTH_SESSION_KEY = "buzz.desktop.documents-rail-width";

function clamp(width: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, width));
}

function readStoredWidth(key: string, fallback: number): number {
  if (typeof window === "undefined") return fallback;
  try {
    const raw = window.sessionStorage.getItem(key);
    if (!raw) return fallback;
    const parsed = Number.parseInt(raw, 10);
    return Number.isFinite(parsed) ? parsed : fallback;
  } catch {
    return fallback;
  }
}

type PaneConfig = {
  defaultWidth: number;
  /** `-1` when dragging the handle should shrink the pane (right-hand rail). */
  direction: 1 | -1;
  max: number;
  min: number;
  storageKey: string;
};

function useResizablePane({
  defaultWidth,
  direction,
  max,
  min,
  storageKey,
}: PaneConfig) {
  const [widthPx, setWidthPx] = React.useState<number>(() =>
    clamp(readStoredWidth(storageKey, defaultWidth), min, max),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.sessionStorage.setItem(storageKey, String(widthPx));
    } catch {
      // Ignore storage failures and keep the chosen width in memory.
    }
  }, [storageKey, widthPx]);

  const handleResizeStart = React.useCallback(
    (event: React.PointerEvent<HTMLButtonElement>) => {
      event.preventDefault();

      const startX = event.clientX;
      const startWidth = widthPx;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const deltaX = (moveEvent.clientX - startX) * direction;
        setWidthPx(clamp(startWidth + deltaX, min, max));
      };

      const handlePointerUp = () => {
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        window.removeEventListener("pointermove", handlePointerMove);
      };

      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp, { once: true });
    },
    [direction, max, min, widthPx],
  );

  const handleWidthReset = React.useCallback(() => {
    setWidthPx(defaultWidth);
  }, [defaultWidth]);

  return {
    canReset: widthPx !== defaultWidth,
    handleResizeStart,
    handleWidthReset,
    widthPx,
  };
}

export function useResizableDocumentsPanes() {
  const tree = useResizablePane({
    defaultWidth: TREE_DEFAULT_WIDTH_PX,
    direction: 1,
    max: TREE_MAX_WIDTH_PX,
    min: TREE_MIN_WIDTH_PX,
    storageKey: TREE_WIDTH_SESSION_KEY,
  });

  const rail = useResizablePane({
    defaultWidth: RAIL_DEFAULT_WIDTH_PX,
    direction: -1,
    max: RAIL_MAX_WIDTH_PX,
    min: RAIL_MIN_WIDTH_PX,
    storageKey: RAIL_WIDTH_SESSION_KEY,
  });

  return { rail, tree };
}
