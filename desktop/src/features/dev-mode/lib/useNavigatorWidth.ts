import * as React from "react";

const STORAGE_KEY = "buzz.devMode.channelNavigatorWidth";
const MIN_PX = 200;
const MAX_PX = 480;
const DEFAULT_PX = 288;

function clamp(raw: number): number {
  return Math.min(MAX_PX, Math.max(MIN_PX, raw));
}

function readStoredWidth(): number {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    const value = raw === null || raw === undefined ? Number.NaN : Number(raw);
    if (Number.isFinite(value)) {
      return clamp(value);
    }
  } catch {
    // Fall through to the default width.
  }
  return DEFAULT_PX;
}

function persistWidth(width: number) {
  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, String(width));
  } catch {
    // Persistence is best-effort.
  }
}

/**
 * Persisted, pointer-draggable width for the channel navigator (clamped
 * 200–480px). Spread `dividerProps` on a vertical separator at the
 * navigator's right edge; Left/Right arrows nudge the width when focused.
 */
export function useNavigatorWidth() {
  const [width, setWidth] = React.useState(readStoredWidth);
  const [dragging, setDragging] = React.useState(false);
  const dragStart = React.useRef<{ x: number; width: number } | null>(null);

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragStart.current = { x: event.clientX, width };
    setDragging(true);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const start = dragStart.current;
    if (!dragging || !start) return;
    setWidth(clamp(start.width + (event.clientX - start.x)));
  };

  const handlePointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    setDragging(false);
    dragStart.current = null;
    setWidth((current) => {
      persistWidth(current);
      return current;
    });
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const delta = event.key === "ArrowLeft" ? -16 : 16;
    setWidth((current) => {
      const next = clamp(current + delta);
      persistWidth(next);
      return next;
    });
  };

  return {
    width,
    dragging,
    dividerProps: {
      onPointerDown: handlePointerDown,
      onPointerMove: handlePointerMove,
      onPointerUp: handlePointerUp,
      onKeyDown: handleKeyDown,
    },
  };
}
