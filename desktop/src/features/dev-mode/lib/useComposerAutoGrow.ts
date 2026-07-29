import * as React from "react";

/** One text line at `leading-6` — the composer's minimum height. */
const LINE_PX = 24;

/** Cap auto-growth so the transcript always stays visible. */
function maxHeightPx(): number {
  return Math.max(LINE_PX * 4, Math.floor(globalThis.innerHeight * 0.4));
}

function readStoredFloor(storageKey: string): number | null {
  try {
    const raw = globalThis.localStorage?.getItem(storageKey);
    const value = raw === null || raw === undefined ? Number.NaN : Number(raw);
    if (Number.isFinite(value) && value > LINE_PX) {
      return value;
    }
  } catch {
    // Fall through to content-sized.
  }
  return null;
}

/**
 * Auto-grow a composer textarea upward with its content (wrapped lines
 * included, unlike a newline-counting `rows`), capped at 40% of the window.
 *
 * A drag on the returned handle sets a persisted manual *floor*: the box
 * never rests shorter than the floor, while content taller than it still
 * grows the box up to the cap (then scrolls internally).
 */
export function useComposerAutoGrow(value: string, storageKey: string) {
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);
  const [floor, setFloor] = React.useState<number | null>(() =>
    readStoredFloor(storageKey),
  );
  const [dragging, setDragging] = React.useState(false);
  const dragStart = React.useRef<{ y: number; floor: number } | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: value is the intentional resize trigger; the effect measures the DOM, not the string
  React.useLayoutEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "0px";
    const max = maxHeightPx();
    const target = Math.min(
      Math.max(el.scrollHeight, floor ?? 0, LINE_PX),
      max,
    );
    el.style.height = `${target}px`;
    el.style.overflowY = el.scrollHeight > target ? "auto" : "hidden";
  }, [value, floor]);

  const persistFloor = (next: number | null) => {
    try {
      if (next === null) {
        globalThis.localStorage?.removeItem(storageKey);
      } else {
        globalThis.localStorage?.setItem(storageKey, String(next));
      }
    } catch {
      // Persistence is best-effort.
    }
  };

  const clampFloor = (raw: number): number =>
    Math.min(Math.max(raw, LINE_PX), maxHeightPx());

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragStart.current = {
      y: event.clientY,
      floor: floor ?? textareaRef.current?.offsetHeight ?? LINE_PX,
    };
    setDragging(true);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const start = dragStart.current;
    if (!dragging || !start) return;
    // The composer sits at the bottom, so dragging up (smaller Y) grows it.
    setFloor(clampFloor(start.floor + (start.y - event.clientY)));
  };

  const handlePointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!dragging) return;
    event.currentTarget.releasePointerCapture(event.pointerId);
    setDragging(false);
    dragStart.current = null;
    setFloor((current) => {
      persistFloor(current);
      return current;
    });
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;
    event.preventDefault();
    const delta = event.key === "ArrowUp" ? LINE_PX : -LINE_PX;
    setFloor((current) => {
      const base = current ?? textareaRef.current?.offsetHeight ?? LINE_PX;
      const next = clampFloor(base + delta);
      persistFloor(next);
      return next;
    });
  };

  return {
    textareaRef,
    dragging,
    resizeHandleProps: {
      "aria-valuenow": Math.round(floor ?? LINE_PX),
      onPointerDown: handlePointerDown,
      onPointerMove: handlePointerMove,
      onPointerUp: handlePointerUp,
      onKeyDown: handleKeyDown,
    },
  };
}
