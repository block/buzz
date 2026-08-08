import * as React from "react";

import {
  AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
  clampAuxiliaryPanelWidth,
} from "@/shared/layout/AuxiliaryPanel";

const THREAD_PANEL_WIDTH_STORAGE_KEY = "buzz.desktop.thread-panel-width";

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

function readStoredThreadPanelWidth(): string | null {
  try {
    const fromLocal = window.localStorage.getItem(THREAD_PANEL_WIDTH_STORAGE_KEY);
    if (fromLocal != null) {
      return fromLocal;
    }

    // One-time migrate: older builds kept this width in sessionStorage only.
    const fromSession = window.sessionStorage.getItem(
      THREAD_PANEL_WIDTH_STORAGE_KEY,
    );
    if (fromSession != null) {
      try {
        window.localStorage.setItem(THREAD_PANEL_WIDTH_STORAGE_KEY, fromSession);
        window.sessionStorage.removeItem(THREAD_PANEL_WIDTH_STORAGE_KEY);
      } catch {
        // Keep reading the session value even if migration write fails.
      }
      return fromSession;
    }

    return null;
  } catch {
    return null;
  }
}

function getInitialThreadPanelWidth(): number {
  if (typeof window === "undefined") {
    return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
  }

  const raw = readStoredThreadPanelWidth();
  if (!raw) {
    return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
  }

  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed)) {
    return AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
  }

  return clampThreadPanelWidth(parsed);
}

export function useThreadPanelWidth(availableWidthPx?: number) {
  const getAvailableWidth = React.useCallback(
    () => availableWidthPx ?? getViewportWidth(),
    [availableWidthPx],
  );
  const [widthPx, setWidthPx] = React.useState<number>(() =>
    getInitialThreadPanelWidth(),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    try {
      window.localStorage.setItem(
        THREAD_PANEL_WIDTH_STORAGE_KEY,
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
        const nextWidth = clampAuxiliaryPanelWidth(
          startWidth + deltaX,
          getAvailableWidth(),
        );
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
    [getAvailableWidth, widthPx],
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
