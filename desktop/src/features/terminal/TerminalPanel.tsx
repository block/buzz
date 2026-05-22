import * as React from "react";

import { TerminalInstance } from "./TerminalInstance";

const TERMINAL_MIN_HEIGHT_PX = 150;
const TERMINAL_MAX_HEIGHT_PX = 600;
const TERMINAL_DEFAULT_HEIGHT_PX = 280;
const TERMINAL_HEIGHT_SESSION_KEY = "sprout.desktop.terminal-panel-height";

function clampTerminalHeight(height: number): number {
  return Math.max(
    TERMINAL_MIN_HEIGHT_PX,
    Math.min(TERMINAL_MAX_HEIGHT_PX, height),
  );
}

function getInitialTerminalHeight(): number {
  if (typeof window === "undefined") return TERMINAL_DEFAULT_HEIGHT_PX;
  try {
    const raw = window.sessionStorage.getItem(TERMINAL_HEIGHT_SESSION_KEY);
    if (!raw) return TERMINAL_DEFAULT_HEIGHT_PX;
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isFinite(parsed)) return TERMINAL_DEFAULT_HEIGHT_PX;
    return clampTerminalHeight(parsed);
  } catch {
    return TERMINAL_DEFAULT_HEIGHT_PX;
  }
}

type TerminalPanelProps = {
  channelId: string;
  isOpen: boolean;
};

export function TerminalPanel({ channelId, isOpen }: TerminalPanelProps) {
  const [heightPx, setHeightPx] = React.useState(getInitialTerminalHeight);

  // Persist height to session storage.
  React.useEffect(() => {
    try {
      window.sessionStorage.setItem(
        TERMINAL_HEIGHT_SESSION_KEY,
        String(heightPx),
      );
    } catch {
      // Ignore.
    }
  }, [heightPx]);

  // Resize via pointer drag on the top edge.
  const handleResizeStart = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      // Only trigger on the top 6px of the container.
      const rect = event.currentTarget.getBoundingClientRect();
      if (event.clientY - rect.top > 6) return;

      event.preventDefault();
      const startY = event.clientY;
      const startHeight = heightPx;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      document.body.style.cursor = "row-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        const deltaY = startY - moveEvent.clientY;
        setHeightPx(clampTerminalHeight(startHeight + deltaY));
      };

      const handlePointerUp = () => {
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        window.removeEventListener("pointermove", handlePointerMove);
      };

      window.addEventListener("pointermove", handlePointerMove);
      window.addEventListener("pointerup", handlePointerUp, { once: true });
    },
    [heightPx],
  );

  if (!isOpen) return null;

  return (
    <div className="px-4 pb-4 pt-0">
      <div
        className="pointer-events-auto relative flex flex-col overflow-hidden rounded-2xl border border-border/50 shadow-[0_4px_24px_rgba(0,0,0,0.08)] dark:shadow-[0_4px_24px_rgba(0,0,0,0.35)]"
        style={{ height: `${heightPx}px` }}
        onPointerDown={handleResizeStart}
      >
        {/* Terminal content */}
        <div className="min-h-0 flex-1 bg-[#1a1b26] p-2">
          <TerminalInstance channelId={channelId} isVisible={isOpen} />
        </div>
      </div>
    </div>
  );
}
