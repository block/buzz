import * as React from "react";

/**
 * Persisted Assistant popover size (width + height). Drag handles in the
 * popover chrome write here; defaults are intentionally taller/wider than the
 * old fixed w-80 / 32rem so chat has room above Lists.
 *
 * `maxHeightPx` is the stored field name (compat); it is applied as the
 * popover's explicit height so dragging taller than content leaves empty
 * space in the chat area.
 */
export const BESTIE_POPOVER_SIZE_STORAGE_KEY = "buzz-bestie-popover-size.v1";

/** Default width (px) — was Tailwind w-80 = 320. */
export const BESTIE_POPOVER_DEFAULT_WIDTH_PX = 448;
/** Default height (px) — was 32rem = 512. */
export const BESTIE_POPOVER_DEFAULT_MAX_HEIGHT_PX = 720;

export const BESTIE_POPOVER_MIN_WIDTH_PX = 320;
export const BESTIE_POPOVER_MAX_WIDTH_PX = 720;

/**
 * Absolute floor when Lists are collapsed: header + visible message area +
 * composer must stay on screen.
 */
export const BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX = 360;
/**
 * Floor when Lists are expanded: chat/reminders stay visible — cannot collapse
 * past seeing the Lists section (category rows / reminders).
 */
export const BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX = 520;
/** @deprecated Prefer BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX; kept for older imports/tests. */
export const BESTIE_POPOVER_MIN_MAX_HEIGHT_PX = BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX;
export const BESTIE_POPOVER_MAX_MAX_HEIGHT_PX = 960;
/** Cap persisted/dragged height to this fraction of the viewport. */
export const BESTIE_POPOVER_VIEWPORT_HEIGHT_RATIO = 0.85;
/**
 * Viewport gutter (px) reserved above/below the popover so it never fills the
 * screen on short displays. Match CSS `calc(100vh - 6rem)`.
 */
export const BESTIE_POPOVER_VIEWPORT_GUTTER_PX = 96;

export type BestiePopoverSize = {
  maxHeightPx: number;
  widthPx: number;
};

type StoredSize = Partial<BestiePopoverSize>;

const listeners = new Set<() => void>();

let size: BestiePopoverSize = readStoredSize();

function clamp(n: number, min: number, max: number): number {
  if (!Number.isFinite(n)) return min;
  if (min > max) return max;
  return Math.min(max, Math.max(min, Math.round(n)));
}

/** Min height for the current Lists collapse state. */
export function bestiePopoverMinHeightPx(listsCollapsed: boolean): number {
  return listsCollapsed
    ? BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX
    : BESTIE_POPOVER_MIN_HEIGHT_WITH_LISTS_PX;
}

/**
 * Absolute max height for the current viewport: min(hard cap, 85% vh, vh − gutter).
 * Falls back to the hard cap when no viewport size is available (SSR / tests).
 */
export function bestiePopoverViewportMaxHeightPx(
  viewportHeightPx?: number,
): number {
  const vh =
    viewportHeightPx ??
    (typeof window !== "undefined" && Number.isFinite(window.innerHeight)
      ? window.innerHeight
      : undefined);
  if (vh == null || vh <= 0) {
    return BESTIE_POPOVER_MAX_MAX_HEIGHT_PX;
  }
  const ratioCap = Math.floor(vh * BESTIE_POPOVER_VIEWPORT_HEIGHT_RATIO);
  const gutterCap = Math.floor(vh - BESTIE_POPOVER_VIEWPORT_GUTTER_PX);
  return Math.max(
    1,
    Math.min(BESTIE_POPOVER_MAX_MAX_HEIGHT_PX, ratioCap, gutterCap),
  );
}

function normalizeSize(
  input: StoredSize | null | undefined,
  minHeightPx: number = BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX,
  viewportMaxHeightPx: number = bestiePopoverViewportMaxHeightPx(),
): BestiePopoverSize {
  const heightCeiling = Math.max(1, viewportMaxHeightPx);
  return {
    maxHeightPx: clamp(
      input?.maxHeightPx ?? BESTIE_POPOVER_DEFAULT_MAX_HEIGHT_PX,
      minHeightPx,
      heightCeiling,
    ),
    widthPx: clamp(
      input?.widthPx ?? BESTIE_POPOVER_DEFAULT_WIDTH_PX,
      BESTIE_POPOVER_MIN_WIDTH_PX,
      BESTIE_POPOVER_MAX_WIDTH_PX,
    ),
  };
}

function readStoredSize(): BestiePopoverSize {
  if (typeof window === "undefined") {
    return normalizeSize(null);
  }
  try {
    const raw = window.localStorage.getItem(BESTIE_POPOVER_SIZE_STORAGE_KEY);
    if (!raw) return normalizeSize(null);
    const parsed = JSON.parse(raw) as StoredSize;
    const next = normalizeSize(parsed);
    // Migrate oversized persisted heights (e.g. full-viewport / 960) down to
    // the current 85% viewport + gutter ceiling so drag can shrink from there.
    if (
      typeof parsed.maxHeightPx === "number" &&
      parsed.maxHeightPx !== next.maxHeightPx
    ) {
      try {
        window.localStorage.setItem(
          BESTIE_POPOVER_SIZE_STORAGE_KEY,
          JSON.stringify(next),
        );
      } catch {
        // Persistence is best-effort.
      }
    }
    return next;
  } catch {
    return normalizeSize(null);
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): BestiePopoverSize {
  return size;
}

function getServerSnapshot(): BestiePopoverSize {
  return normalizeSize(null);
}

function emit(): void {
  for (const listener of listeners) listener();
}

/**
 * Replace width and/or height; persists to localStorage.
 * Pass `minHeightPx` while dragging so Lists-open cannot collapse past reminders.
 */
export function setBestiePopoverSize(
  patch: Partial<BestiePopoverSize>,
  options?: { minHeightPx?: number; viewportHeightPx?: number },
): BestiePopoverSize {
  const minHeightPx = options?.minHeightPx ?? BESTIE_POPOVER_MIN_HEIGHT_CHAT_PX;
  const viewportMax = bestiePopoverViewportMaxHeightPx(options?.viewportHeightPx);
  size = normalizeSize({ ...size, ...patch }, minHeightPx, viewportMax);
  try {
    window.localStorage.setItem(
      BESTIE_POPOVER_SIZE_STORAGE_KEY,
      JSON.stringify(size),
    );
  } catch {
    // Persistence is best-effort; in-memory size still applies.
  }
  emit();
  return size;
}

export function useBestiePopoverSize(): BestiePopoverSize {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

/** @internal */
export function __resetBestiePopoverSizeForTests(): void {
  size = normalizeSize(null);
  try {
    window.localStorage.removeItem(BESTIE_POPOVER_SIZE_STORAGE_KEY);
  } catch {
    // ignore
  }
  emit();
}
