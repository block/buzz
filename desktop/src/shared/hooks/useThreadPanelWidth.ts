import * as React from "react";

import {
  AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
  clampAuxiliaryPanelWidth,
} from "@/shared/layout/AuxiliaryPanel";

const THREAD_PANEL_WIDTH_SESSION_KEY = "buzz.desktop.thread-panel-width";

function getViewportWidth(): number {
  return typeof window === "undefined" ? 0 : window.innerWidth;
}

/**
 * Clamp the stored panel width for the current viewport.
 *
 * The upper bound grows with the viewport (see {@link clampAuxiliaryPanelWidth}) so
 * the pane can expand on ultrawide displays. `AuxiliaryPanelShell` additionally
 * clamps the rendered width to `calc(100% - MIN)` at paint time, so a stored width
 * larger than the current viewport never collapses the main pane.
 */
function clampThreadPanelWidth(width: number): number {
  return clampAuxiliaryPanelWidth(width, getViewportWidth());
}

function getInitialThreadPanelWidth(): number {
  if (typeof window === "undefined") {
    return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
  }

  try {
    const raw = window.sessionStorage.getItem(THREAD_PANEL_WIDTH_SESSION_KEY);
    if (!raw) {
      return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
    }

    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) {
      return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
    }

    return clampThreadPanelWidth(parsed);
  } catch {
    return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
  }
}

export function useThreadPanelWidth() {
  const [widthPx, setWidthPx] = React.useState<number>(() =>
    getInitialThreadPanelWidth(),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    try {
      window.sessionStorage.setItem(
        THREAD_PANEL_WIDTH_SESSION_KEY,
        String(widthPx),
      );
    } catch {
      // Ignore storage failures and keep in-memory width for this session.
    }
  }, [widthPx]);

  const onResizeStart = React.useCallback(
    (event: React.PointerEvent<HTMLButtonElement>) => {
      event.preventDefault();

      const startX = event.clientX;
      const startWidth = widthPx;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const deltaX = startX - moveEvent.clientX;
        const nextWidth = clampThreadPanelWidth(startWidth + deltaX);
        setWidthPx(nextWidth);
      };

      const handlePointerUp = () => {
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        window.removeEventListener("pointermove", handlePointerMove);
      };

      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp, { once: true });
    },
    [widthPx],
  );

  const onResetWidth = React.useCallback(() => {
    setWidthPx(AUXILIARY_PANEL_DEFAULT_WIDTH_PX);
  }, []);

  return {
    canReset: widthPx !== AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
    onResetWidth,
    onResizeStart,
    widthPx,
  };
}
