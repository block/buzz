import * as React from "react";

/** Device-level type scale applied throughout the desktop interface. */
export type FontSize = "smaller" | "default" | "larger";

export const FONT_SIZE_STORAGE_KEY = "buzz.appearance.fontSize";
export const DEFAULT_FONT_SIZE: FontSize = "default";

const BASE_FONT_SIZE_PX: Record<FontSize, number> = {
  smaller: 15,
  default: 16,
  larger: 17,
};

const listeners = new Set<() => void>();
let fontSize: FontSize = DEFAULT_FONT_SIZE;
let textZoomFactor = 1;

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

function rootFontSizePx(size: FontSize): number {
  return Math.round(BASE_FONT_SIZE_PX[size] * textZoomFactor * 1_000) / 1_000;
}

function applyFontSize(size: FontSize): void {
  const root = globalThis.document?.documentElement;
  root?.setAttribute("data-font-size", size);
  if (root) root.style.fontSize = `${rootFontSizePx(size)}px`;
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

/** Apply the persisted preference before React renders to avoid a layout jump. */
export function initializeFontSizePreference(): void {
  const nextSize = readStoredFontSize();
  const changed = nextSize !== fontSize;
  fontSize = nextSize;
  applyFontSize(nextSize);
  if (changed) notifyListeners();
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
