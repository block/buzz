import * as React from "react";

/** Device-level type scale applied throughout the desktop interface. */
export type FontSize = "smaller" | "default" | "larger";

export const FONT_SIZE_STORAGE_KEY = "buzz.appearance.fontSize";
export const DEFAULT_FONT_SIZE: FontSize = "default";

/**
 * Root attribute that selects the type scale. The 13 / 14 / 15px contract and
 * the virtual typography rem it drives live in `styles/globals/typography.css`;
 * this module only records the user's choice. Cmd +/- zoom is a separate dial
 * (`useWebviewZoomShortcuts`) that scales the real root font-size.
 */
const FONT_SIZE_ATTRIBUTE = "data-font-size";

const listeners = new Set<() => void>();
let fontSize: FontSize = DEFAULT_FONT_SIZE;
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

function applyFontSize(size: FontSize): void {
  globalThis.document?.documentElement?.setAttribute(FONT_SIZE_ATTRIBUTE, size);
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
