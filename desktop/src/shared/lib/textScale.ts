/**
 * App text / UI scale preference.
 *
 * Scales the root `<html>` font-size so rem-based layout grows or shrinks.
 * Native webview zoom stays pinned at 1 so layout coordinates stay stable
 * (see `useWebviewZoomShortcuts`).
 *
 * Persisted in localStorage under {@link TEXT_SCALE_STORAGE_KEY}. Device-level
 * only — not community-scoped.
 *
 * React hook: {@link useTextScale} in `./useTextScale`.
 */

import { APPEARANCE_SCALE_PRESETS } from "./appearanceScalePresets";

export const TEXT_SCALE_STORAGE_KEY = "buzz:text-scale";

export const DEFAULT_TEXT_SCALE = 1;
export const TEXT_SCALE_PRESETS = APPEARANCE_SCALE_PRESETS;
export const MIN_TEXT_SCALE = TEXT_SCALE_PRESETS[0];
export const MAX_TEXT_SCALE = TEXT_SCALE_PRESETS[TEXT_SCALE_PRESETS.length - 1];

/** @deprecated Prefer stepping via {@link adjustTextScale} preset indices. */
export const TEXT_SCALE_STEP = 0.1;

const BASE_FONT_SIZE_PX = 16;

const listeners = new Set<() => void>();

let textScale = readStoredTextScale();

export function roundTextScale(scale: number): number {
  return Math.round(scale * 10) / 10;
}

export function clampTextScale(scale: number): number {
  return Math.min(Math.max(scale, MIN_TEXT_SCALE), MAX_TEXT_SCALE);
}

/**
 * Snap a raw scale to the nearest supported preset.
 */
export function normalizeTextScale(scale: number): number {
  if (!Number.isFinite(scale)) {
    return DEFAULT_TEXT_SCALE;
  }

  let best: number = TEXT_SCALE_PRESETS[0];
  let bestDistance = Number.POSITIVE_INFINITY;
  for (const preset of TEXT_SCALE_PRESETS) {
    const distance = Math.abs(preset - scale);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = preset;
    }
  }
  return best;
}

function readStoredTextScale(): number {
  try {
    const raw = globalThis.localStorage?.getItem(TEXT_SCALE_STORAGE_KEY);
    if (!raw) {
      return DEFAULT_TEXT_SCALE;
    }
    return normalizeTextScale(Number.parseFloat(raw));
  } catch {
    return DEFAULT_TEXT_SCALE;
  }
}

function applyTextScaleToDocument(scale: number): void {
  if (typeof document === "undefined") {
    return;
  }

  if (scale === DEFAULT_TEXT_SCALE) {
    document.documentElement.style.fontSize = "";
    return;
  }

  document.documentElement.style.fontSize = `${BASE_FONT_SIZE_PX * scale}px`;
}

function persistTextScale(scale: number): void {
  try {
    if (scale === DEFAULT_TEXT_SCALE) {
      globalThis.localStorage?.removeItem(TEXT_SCALE_STORAGE_KEY);
    } else {
      globalThis.localStorage?.setItem(TEXT_SCALE_STORAGE_KEY, String(scale));
    }
  } catch {
    // Persistence is best-effort; the in-memory value still applies.
  }
}

function notify(): void {
  for (const listener of listeners) {
    listener();
  }
}

/** Read the current text scale outside of React. */
export function getTextScale(): number {
  return textScale;
}

/**
 * Set text scale, apply it to the document root, persist, and notify
 * subscribers. Values outside the supported range are clamped to presets.
 */
export function setTextScale(scale: number): void {
  const next = normalizeTextScale(scale);
  if (next === textScale) {
    applyTextScaleToDocument(next);
    return;
  }

  textScale = next;
  applyTextScaleToDocument(next);
  persistTextScale(next);
  notify();
}

export type TextScaleAction = "increase" | "decrease" | "reset";

/** Step text scale by adjacent preset (same path as keyboard zoom). */
export function adjustTextScale(action: TextScaleAction): number {
  if (action === "reset") {
    setTextScale(DEFAULT_TEXT_SCALE);
    return textScale;
  }

  const currentIndex = textScalePresetIndex(textScale);
  const direction = action === "increase" ? 1 : -1;
  const nextIndex = Math.min(
    Math.max(currentIndex + direction, 0),
    TEXT_SCALE_PRESETS.length - 1,
  );
  setTextScale(TEXT_SCALE_PRESETS[nextIndex] ?? DEFAULT_TEXT_SCALE);
  return textScale;
}

/**
 * Apply the current in-memory scale to the document (e.g. on app boot after
 * module load, or after restoring from storage in a new document).
 */
export function applyCurrentTextScale(): void {
  applyTextScaleToDocument(textScale);
}

export function subscribeTextScale(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getTextScaleSnapshot(): number {
  return textScale;
}

export function getTextScaleServerSnapshot(): number {
  return DEFAULT_TEXT_SCALE;
}

/** Format a scale factor as a percentage label (e.g. 1.2 → "120%"). */
export function formatTextScalePercent(scale: number): string {
  return `${Math.round(normalizeTextScale(scale) * 100)}%`;
}

/** Nearest index into {@link TEXT_SCALE_PRESETS} for a scale factor. */
export function textScalePresetIndex(scale: number): number {
  const target = normalizeTextScale(scale);
  const index = TEXT_SCALE_PRESETS.indexOf(
    target as (typeof TEXT_SCALE_PRESETS)[number],
  );
  return index >= 0 ? index : 0;
}
