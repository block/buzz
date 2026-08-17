import * as React from "react";

/** Device-level type scale applied throughout the desktop interface. */
export type FontSize = "smaller" | "default" | "larger";

export const FONT_SIZE_STORAGE_KEY = "buzz.appearance.fontSize";
export const DEFAULT_FONT_SIZE: FontSize = "default";

/**
 * Virtual rem sizes used by typography tokens. Keeping the real root at 16px
 * prevents a text preference from also resizing rem-based layout geometry.
 */
const TYPE_REM_SIZE_PX: Record<FontSize, number> = {
  smaller: 13 / 0.875,
  default: 14 / 0.875,
  larger: 15 / 0.875,
};

const TYPE_REM_PROPERTY = "--buzz-type-rem";

const listeners = new Set<() => void>();
let fontSize: FontSize = DEFAULT_FONT_SIZE;
let textZoomFactor = 1;
let listeningForStorageChanges = false;

export function parseFontSize(value: string | null | undefined): FontSize {
  return value === "smaller" || value === "default" || value === "larger"
    ? value
    : DEFAULT_FONT_SIZE;
}

function readStoredFontSize(): FontSize {
  try {
    return parseFontSize(
      globalThis.localStorage?.getItem(FONT_SIZE_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_FONT_SIZE;
  }
}

function typeRemSizePx(size: FontSize): number {
  return (
    Math.round(TYPE_REM_SIZE_PX[size] * textZoomFactor * 1_000_000) / 1_000_000
  );
}

function applyFontSize(size: FontSize): void {
  const root = globalThis.document?.documentElement;
  root?.setAttribute("data-font-size", size);
  root?.style.setProperty(TYPE_REM_PROPERTY, `${typeRemSizePx(size)}px`);
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

function applyStoredFontSize(): void {
  const nextSize = readStoredFontSize();
  const changed = nextSize !== fontSize;
  fontSize = nextSize;
  applyFontSize(nextSize);
  if (changed) notifyListeners();
}

function listenForStorageChanges(): void {
  if (listeningForStorageChanges || !globalThis.window?.addEventListener)
    return;
  globalThis.window.addEventListener("storage", (event) => {
    if (event.key === FONT_SIZE_STORAGE_KEY || event.key === null) {
      applyStoredFontSize();
    }
  });
  listeningForStorageChanges = true;
}

/** Apply the persisted preference before React renders to avoid a layout jump. */
export function initializeFontSizePreference(): void {
  applyStoredFontSize();
  listenForStorageChanges();
}

/** Combine Cmd +/- zoom with the selected app-wide type scale. */
export function applyTextZoomFactor(zoomFactor: number): void {
  if (!Number.isFinite(zoomFactor) || zoomFactor <= 0) return;
  textZoomFactor = zoomFactor;
  applyFontSize(fontSize);
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getFontSize(): FontSize {
  return fontSize;
}

export function setFontSize(size: FontSize): void {
  fontSize = size;
  applyFontSize(size);
  try {
    globalThis.localStorage?.setItem(FONT_SIZE_STORAGE_KEY, size);
  } catch {
    // Persistence is best-effort; the live preference still applies.
  }
  notifyListeners();
}

/** Temporarily apply a size without changing the saved preference. */
export function previewFontSize(size: FontSize | null): void {
  applyFontSize(size ?? fontSize);
}

export function useFontSize(): FontSize {
  return React.useSyncExternalStore(
    subscribe,
    getFontSize,
    () => DEFAULT_FONT_SIZE,
  );
}
