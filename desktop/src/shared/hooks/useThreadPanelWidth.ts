import * as React from "react";

const THREAD_PANEL_DEFAULT_WIDTH_PX = 380;
const THREAD_PANEL_MIN_WIDTH_PX = 320;
const THREAD_PANEL_MAX_WIDTH_PX = 720;
const THREAD_PANEL_WIDTH_SESSION_KEY = "sprout.desktop.thread-panel-width";

function clampThreadPanelWidth(width: number): number {
  return Math.max(
    THREAD_PANEL_MIN_WIDTH_PX,
    Math.min(THREAD_PANEL_MAX_WIDTH_PX, width),
  );
}

function getInitialThreadPanelWidth(): number {
  if (typeof window === "undefined") {
    return THREAD_PANEL_DEFAULT_WIDTH_PX;
  }

  try {
    const raw = window.sessionStorage.getItem(THREAD_PANEL_WIDTH_SESSION_KEY);
    if (!raw) {
      return THREAD_PANEL_DEFAULT_WIDTH_PX;
    }

    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) {
      return THREAD_PANEL_DEFAULT_WIDTH_PX;
    }

    return clampThreadPanelWidth(parsed);
  } catch {
    return THREAD_PANEL_DEFAULT_WIDTH_PX;
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
    setWidthPx(THREAD_PANEL_DEFAULT_WIDTH_PX);
  }, []);

  return {
    canReset: widthPx !== THREAD_PANEL_DEFAULT_WIDTH_PX,
    onResetWidth,
    onResizeStart,
    widthPx,
  };
}
