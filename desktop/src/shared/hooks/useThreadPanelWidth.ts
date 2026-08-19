import * as React from "react";

import {
  AUXILIARY_PANEL_DEFAULT_WIDTH_PX,
  AUXILIARY_PANEL_MIN_WIDTH_PX,
  getAuxiliaryPanelMaxWidth,
} from "@/shared/layout/AuxiliaryPanel";

const THREAD_PANEL_WIDTH_SESSION_KEY = "buzz.desktop.thread-panel-width";

type ThreadPanelWidthOptions = {
  defaultWidthPx?: number;
  minWidthPx?: number;
  sessionKey?: string;
};

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
function clampThreadPanelWidth(width: number, minWidthPx: number): number {
  return Math.max(
    minWidthPx,
    Math.min(getAuxiliaryPanelMaxWidth(getViewportWidth()), width),
  );
}

function getInitialThreadPanelWidth({
  defaultWidthPx,
  minWidthPx,
  sessionKey,
}: Required<ThreadPanelWidthOptions>): number {
  if (typeof window === "undefined") {
    return defaultWidthPx;
  }

  try {
    const raw = window.sessionStorage.getItem(sessionKey);
    if (!raw) {
      return defaultWidthPx;
    }

    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) {
      return defaultWidthPx;
    }

    return clampThreadPanelWidth(parsed, minWidthPx);
  } catch {
    return defaultWidthPx;
  }
}

export function useThreadPanelWidth(
  availableWidthPx?: number,
  options: ThreadPanelWidthOptions = {},
) {
  const defaultWidthPx =
    options.defaultWidthPx ?? AUXILIARY_PANEL_DEFAULT_WIDTH_PX;
  const minWidthPx = options.minWidthPx ?? AUXILIARY_PANEL_MIN_WIDTH_PX;
  const sessionKey = options.sessionKey ?? THREAD_PANEL_WIDTH_SESSION_KEY;
  const getAvailableWidth = React.useCallback(
    () => availableWidthPx ?? getViewportWidth(),
    [availableWidthPx],
  );
  const [widthPx, setWidthPx] = React.useState<number>(() =>
    getInitialThreadPanelWidth({ defaultWidthPx, minWidthPx, sessionKey }),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    try {
      window.sessionStorage.setItem(sessionKey, String(widthPx));
    } catch {
      // Ignore storage failures and keep in-memory width for this session.
    }
  }, [sessionKey, widthPx]);

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
        const nextWidth = Math.max(
          minWidthPx,
          Math.min(
            getAuxiliaryPanelMaxWidth(getAvailableWidth()),
            startWidth + deltaX,
          ),
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
    [getAvailableWidth, minWidthPx, widthPx],
  );

  const onResetWidth = React.useCallback(() => {
    setWidthPx(defaultWidthPx);
  }, [defaultWidthPx]);

  return {
    canReset: widthPx !== defaultWidthPx,
    onResetWidth,
    onResizeStart,
    widthPx,
  };
}
