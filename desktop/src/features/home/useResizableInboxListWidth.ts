import * as React from "react";

const INBOX_LIST_DEFAULT_WIDTH_PX = 365;
export const INBOX_COLUMN_MIN_WIDTH_PX = 300;
export const INBOX_SINGLE_COLUMN_BREAKPOINT_PX = INBOX_COLUMN_MIN_WIDTH_PX * 2;
const INBOX_LIST_MAX_WIDTH_PX = 520;
const INBOX_LIST_WIDTH_SESSION_KEY = "buzz.desktop.home-inbox-list-width";

function clampInboxListWidth(width: number): number {
  return Math.max(
    INBOX_COLUMN_MIN_WIDTH_PX,
    Math.min(INBOX_LIST_MAX_WIDTH_PX, width),
  );
}

function getInitialInboxListWidth(): number {
  if (typeof window === "undefined") {
    return INBOX_LIST_DEFAULT_WIDTH_PX;
  }

  try {
    const raw = window.sessionStorage.getItem(INBOX_LIST_WIDTH_SESSION_KEY);
    if (!raw) {
      return INBOX_LIST_DEFAULT_WIDTH_PX;
    }

    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) {
      return INBOX_LIST_DEFAULT_WIDTH_PX;
    }

    return clampInboxListWidth(parsed);
  } catch {
    return INBOX_LIST_DEFAULT_WIDTH_PX;
  }
}

export function useResizableInboxListWidth() {
  const [inboxListWidthPx, setInboxListWidthPx] = React.useState<number>(() =>
    getInitialInboxListWidth(),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      return;
    }

    try {
      window.sessionStorage.setItem(
        INBOX_LIST_WIDTH_SESSION_KEY,
        String(inboxListWidthPx),
      );
    } catch {
      // Ignore storage failures and keep the chosen width in memory.
    }
  }, [inboxListWidthPx]);

  const handleInboxListResizeStart = React.useCallback(
    (event: React.PointerEvent<HTMLButtonElement>) => {
      event.preventDefault();

      const startX = event.clientX;
      const startWidth = inboxListWidthPx;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const deltaX = moveEvent.clientX - startX;
        setInboxListWidthPx(clampInboxListWidth(startWidth + deltaX));
      };

      const handlePointerUp = () => {
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        window.removeEventListener("pointermove", handlePointerMove);
      };

      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp, { once: true });
    },
    [inboxListWidthPx],
  );

  const handleInboxListWidthReset = React.useCallback(() => {
    setInboxListWidthPx(INBOX_LIST_DEFAULT_WIDTH_PX);
  }, []);

  return {
    canResetInboxListWidth: inboxListWidthPx !== INBOX_LIST_DEFAULT_WIDTH_PX,
    handleInboxListResizeStart,
    handleInboxListWidthReset,
    inboxListWidthPx,
  };
}
