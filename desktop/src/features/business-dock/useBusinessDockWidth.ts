import * as React from "react";

import {
  formatHorizontalResizeIndicator,
  startHorizontalMouseResize,
} from "@/shared/lib/startHorizontalMouseResize";

export const BUSINESS_DOCK_DEFAULT_WIDTH_PX = 560;
export const BUSINESS_DOCK_MIN_WIDTH_PX = 420;
export const BUSINESS_DOCK_MAX_WIDTH_RATIO = 0.5;
export const BUSINESS_DOCK_OVERLAY_BREAKPOINT_PX = 1000;
export const BUSINESS_DOCK_WIDTH_SESSION_KEY = "buzz.business-dock.width";

export function getBusinessDockMaxWidth(viewportWidth: number): number {
  if (viewportWidth < BUSINESS_DOCK_OVERLAY_BREAKPOINT_PX) {
    return Math.max(0, viewportWidth);
  }
  return Math.floor(viewportWidth * BUSINESS_DOCK_MAX_WIDTH_RATIO);
}

export function clampBusinessDockWidth(
  width: number,
  viewportWidth: number,
): number {
  const maxWidth = Math.max(
    BUSINESS_DOCK_MIN_WIDTH_PX,
    getBusinessDockMaxWidth(viewportWidth),
  );
  return Math.max(BUSINESS_DOCK_MIN_WIDTH_PX, Math.min(maxWidth, width));
}

function getViewportWidth() {
  return typeof window === "undefined" ? 0 : window.innerWidth;
}

type BusinessDockWidthStorage = Pick<Storage, "getItem" | "setItem">;

export function readBusinessDockWidth(
  storage: BusinessDockWidthStorage,
  viewportWidth: number,
): number {
  try {
    const stored = Number.parseInt(
      storage.getItem(BUSINESS_DOCK_WIDTH_SESSION_KEY) ?? "",
      10,
    );
    return Number.isFinite(stored)
      ? clampBusinessDockWidth(stored, viewportWidth)
      : BUSINESS_DOCK_DEFAULT_WIDTH_PX;
  } catch {
    return BUSINESS_DOCK_DEFAULT_WIDTH_PX;
  }
}

export function saveBusinessDockWidth(
  storage: BusinessDockWidthStorage,
  widthPx: number,
): void {
  try {
    storage.setItem(BUSINESS_DOCK_WIDTH_SESSION_KEY, String(widthPx));
  } catch {
    // Keep the in-memory width when storage is unavailable.
  }
}

function getInitialWidth() {
  if (typeof window === "undefined") {
    return BUSINESS_DOCK_DEFAULT_WIDTH_PX;
  }
  return readBusinessDockWidth(window.sessionStorage, getViewportWidth());
}

export function useBusinessDockWidth() {
  const [widthPx, setWidthPx] = React.useState(getInitialWidth);
  const [viewportWidth, setViewportWidth] = React.useState(getViewportWidth);

  React.useEffect(() => {
    const onResize = () => setViewportWidth(getViewportWidth());
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  React.useEffect(() => {
    saveBusinessDockWidth(window.sessionStorage, widthPx);
  }, [widthPx]);

  const onResizeStart = React.useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      // Leave the second press unshielded so double-click reset can complete.
      if (event.detail > 1) return;
      const startX = event.clientX;
      const startWidth = widthPx;
      startHorizontalMouseResize(
        event,
        (clientX) => {
          const currentViewportWidth = getViewportWidth();
          const nextWidth = clampBusinessDockWidth(
            startWidth + startX - clientX,
            currentViewportWidth,
          );
          setWidthPx(nextWidth);
          return formatHorizontalResizeIndicator(
            nextWidth,
            currentViewportWidth,
          );
        },
        {
          indicatorText: formatHorizontalResizeIndicator(
            widthPx,
            getViewportWidth(),
          ),
        },
      );
    },
    [widthPx],
  );

  const onResetWidth = React.useCallback(
    () => setWidthPx(BUSINESS_DOCK_DEFAULT_WIDTH_PX),
    [],
  );

  return {
    canReset: widthPx !== BUSINESS_DOCK_DEFAULT_WIDTH_PX,
    isOverlay: viewportWidth < BUSINESS_DOCK_OVERLAY_BREAKPOINT_PX,
    onResetWidth,
    onResizeStart,
    renderedWidthPx:
      viewportWidth < BUSINESS_DOCK_OVERLAY_BREAKPOINT_PX
        ? Math.min(widthPx, viewportWidth)
        : clampBusinessDockWidth(widthPx, viewportWidth),
    widthPx,
  };
}
