import * as React from "react";

/**
 * User preference for the base font scale of the Buzz Desktop chat UI.
 *
 * Stored as a number in the range [0.85, 1.30] (i.e. 85%–130% of the default
 * size). The value is applied as a CSS `font-size` multiplier on the
 * document root, cascading into all rem-based typography.
 *
 * Persisted in localStorage. This is a device-level accessibility preference,
 * not community-scoped data, so it is intentionally not reset on community
 * switch.
 */

const STORAGE_KEY = "buzz.ui.fontScale";

const DEFAULT_FONT_SCALE = 1;

const MIN_FONT_SCALE = 0.85;
const MAX_FONT_SCALE = 1.3;

/** Snap points for the slider — makes the control feel deterministic. */
export const FONT_SCALE_PRESETS = [0.85, 0.9, 0.95, 1, 1.1, 1.2, 1.3] as const;

const listeners = new Set<() => void>();

let fontScale = readStoredFontScale();

function clampFontScale(value: number): number {
  if (Number.isNaN(value)) return DEFAULT_FONT_SCALE;
  return Math.min(MAX_FONT_SCALE, Math.max(MIN_FONT_SCALE, value));
}

function readStoredFontScale(): number {
  try {
    const raw = globalThis.localStorage?.getItem(STORAGE_KEY);
    if (raw === null) return DEFAULT_FONT_SCALE;
    return clampFontScale(Number.parseFloat(raw));
  } catch {
    return DEFAULT_FONT_SCALE;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): number {
  return fontScale;
}

function getServerSnapshot(): number {
  return DEFAULT_FONT_SCALE;
}

/** Read the persisted font scale preference outside of React. */
export function getFontScale(): number {
  return fontScale;
}

/** Update the font scale preference and notify all subscribed components. */
export function setFontScale(scale: number): void {
  fontScale = clampFontScale(scale);

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, String(fontScale));
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** The base font-size multiplier for the Buzz Desktop UI. */
export function useFontScale(): number {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
