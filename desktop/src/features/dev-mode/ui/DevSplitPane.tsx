import * as React from "react";

import { cn } from "@/shared/lib/cn";

const STORAGE_KEY = "buzz.devMode.splitPct";
const MIN_PCT = 25;
const MAX_PCT = 75;

function readStoredPct(): number {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    const value = raw === null || raw === undefined ? Number.NaN : Number(raw);
    if (Number.isFinite(value)) {
      return Math.min(MAX_PCT, Math.max(MIN_PCT, value));
    }
  } catch {
    // Fall through to the default split.
  }
  return 50;
}

/**
 * Channel transcript / side-chat split. The divider is pointer-draggable
 * (clamped 25–75%, persisted per device) and the inactive pane dims so it is
 * obvious which composer owns the keyboard.
 */
export function DevSplitPane({
  main,
  side,
  activePane,
}: {
  main: React.ReactNode;
  side: React.ReactNode;
  activePane: "main" | "thread";
}) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const [splitPct, setSplitPct] = React.useState(readStoredPct);
  const [dragging, setDragging] = React.useState(false);

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(true);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    const container = containerRef.current;
    if (!container) return;
    const bounds = container.getBoundingClientRect();
    if (bounds.width === 0) return;
    const pct = ((event.clientX - bounds.left) / bounds.width) * 100;
    setSplitPct(Math.min(MAX_PCT, Math.max(MIN_PCT, pct)));
  };

  const handlePointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    setDragging(false);
    try {
      globalThis.localStorage?.setItem(STORAGE_KEY, String(splitPct));
    } catch {
      // Persistence is best-effort.
    }
  };

  return (
    <div ref={containerRef} className="flex min-h-0 min-w-0 flex-1">
      <div
        className={cn(
          "flex min-h-0 min-w-0 flex-col transition-opacity",
          activePane === "thread" && "opacity-55",
        )}
        style={{ width: `${splitPct}%` }}
      >
        {main}
      </div>
      {/* biome-ignore lint/a11y/useSemanticElements: <hr> cannot host the drag/keyboard resize handlers of a movable separator */}
      <div
        className={cn(
          "w-1 shrink-0 cursor-col-resize bg-border/60 outline-none hover:bg-primary/60 focus-visible:bg-primary/60",
          dragging && "bg-primary",
        )}
        data-testid="dev-mode-split-divider"
        onKeyDown={(event) => {
          if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
          event.preventDefault();
          const delta = event.key === "ArrowLeft" ? -2 : 2;
          setSplitPct((current) => {
            const next = Math.min(MAX_PCT, Math.max(MIN_PCT, current + delta));
            try {
              globalThis.localStorage?.setItem(STORAGE_KEY, String(next));
            } catch {
              // Persistence is best-effort.
            }
            return next;
          });
        }}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        role="separator"
        aria-orientation="vertical"
        aria-valuenow={Math.round(splitPct)}
        tabIndex={0}
      />
      <div
        className={cn(
          "flex min-h-0 min-w-0 flex-1 flex-col transition-opacity",
          activePane === "main" && "opacity-55",
        )}
      >
        {side}
      </div>
    </div>
  );
}
