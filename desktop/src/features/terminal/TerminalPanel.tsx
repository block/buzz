import * as React from "react";
import { X, Minus, GripHorizontal } from "lucide-react";

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
  onClose: () => void;
};

export function TerminalPanel({
  channelId,
  isOpen,
  onClose,
}: TerminalPanelProps) {
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

  const handleResizeStart = React.useCallback(
    (event: React.PointerEvent<HTMLButtonElement>) => {
      event.preventDefault();
      const startY = event.clientY;
      const startHeight = heightPx;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      document.body.style.cursor = "row-resize";
      document.body.style.userSelect = "none";

      const handlePointerMove = (moveEvent: PointerEvent) => {
        // Dragging up = negative deltaY = taller panel.
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
    <div className="px-3 pb-3 pt-1 sm:px-4">
      <div
        className="pointer-events-auto relative flex flex-col overflow-hidden rounded-2xl border border-border/50 shadow-[0_4px_24px_rgba(0,0,0,0.08)] dark:shadow-[0_4px_24px_rgba(0,0,0,0.35)]"
        style={{ height: `${heightPx}px` }}
      >
        {/* Resize handle */}
        <button
          type="button"
          className="group absolute inset-x-0 -top-1 z-20 flex h-3 cursor-row-resize items-center justify-center"
          onPointerDown={handleResizeStart}
          aria-label="Resize terminal"
        >
          <GripHorizontal className="h-3 w-3 text-muted-foreground/40 transition-colors group-hover:text-muted-foreground" />
        </button>

        {/* Header bar */}
        <div className="flex h-8 shrink-0 items-center justify-between border-b border-white/5 bg-[#1a1b26] px-3">
          <span className="text-xs font-medium text-[#7aa2f7]">Terminal</span>
          <div className="flex items-center gap-1">
            <button
              type="button"
              className="rounded p-0.5 text-[#565f89] hover:text-[#c0caf5] transition-colors"
              onClick={onClose}
              aria-label="Minimize terminal"
            >
              <Minus className="h-3.5 w-3.5" />
            </button>
            <button
              type="button"
              className="rounded p-0.5 text-[#565f89] hover:text-[#c0caf5] transition-colors"
              onClick={onClose}
              aria-label="Close terminal"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        {/* Terminal content */}
        <div className="min-h-0 flex-1 bg-[#1a1b26]">
          <TerminalInstance channelId={channelId} isVisible={isOpen} />
        </div>
      </div>
    </div>
  );
}
