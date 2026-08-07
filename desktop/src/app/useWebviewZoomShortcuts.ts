import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const DEFAULT_ZOOM_FACTOR = 1;
const MIN_ZOOM_FACTOR = 0.75;
const MAX_ZOOM_FACTOR = 1.5;
const ZOOM_STEP = 0.1;
const BASE_FONT_SIZE_PX = 16;
const TEXT_SCALE_STORAGE_KEY = "buzz:text-scale";

type ZoomAction = "increase" | "decrease" | "reset";

function roundZoomFactor(zoomFactor: number) {
  return Math.round(zoomFactor * 10) / 10;
}

function getZoomAction(event: KeyboardEvent): ZoomAction | null {
  if (!hasPrimaryShortcutModifier(event) || event.altKey) {
    return null;
  }

  if (
    event.key === "+" ||
    event.key === "=" ||
    event.code === "Equal" ||
    event.code === "NumpadAdd"
  ) {
    return "increase";
  }

  if (
    !event.shiftKey &&
    (event.key === "-" ||
      event.code === "Minus" ||
      event.code === "NumpadSubtract")
  ) {
    return "decrease";
  }

  if (
    !event.shiftKey &&
    (event.key === "0" || event.code === "Digit0" || event.code === "Numpad0")
  ) {
    return "reset";
  }

  return null;
}

function getNextZoomFactor(action: ZoomAction, zoomFactor: number) {
  if (action === "reset") {
    return DEFAULT_ZOOM_FACTOR;
  }

  if (action === "increase") {
    return Math.min(roundZoomFactor(zoomFactor + ZOOM_STEP), MAX_ZOOM_FACTOR);
  }

  return Math.max(roundZoomFactor(zoomFactor - ZOOM_STEP), MIN_ZOOM_FACTOR);
}

function readStoredZoomFactor() {
  const raw = window.localStorage.getItem(TEXT_SCALE_STORAGE_KEY);
  if (!raw) {
    return DEFAULT_ZOOM_FACTOR;
  }

  const parsed = Number.parseFloat(raw);
  if (!Number.isFinite(parsed)) {
    return DEFAULT_ZOOM_FACTOR;
  }

  return Math.min(Math.max(parsed, MIN_ZOOM_FACTOR), MAX_ZOOM_FACTOR);
}

function applyTextScale(zoomFactor: number) {
  if (zoomFactor === DEFAULT_ZOOM_FACTOR) {
    document.documentElement.style.fontSize = "";
    window.localStorage.removeItem(TEXT_SCALE_STORAGE_KEY);
    return;
  }

  document.documentElement.style.fontSize = `${BASE_FONT_SIZE_PX * zoomFactor}px`;
  window.localStorage.setItem(TEXT_SCALE_STORAGE_KEY, String(zoomFactor));
}

// Reactive store so Settings UI can read and set the zoom factor alongside the
// keyboard shortcuts. Mirrors the threadViewModePreference pattern.
const listeners = new Set<() => void>();
let zoomFactor = readStoredZoomFactor();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): number {
  return zoomFactor;
}

function getServerSnapshot(): number {
  return DEFAULT_ZOOM_FACTOR;
}

function commitZoomFactor(next: number) {
  const clamped = Math.min(
    Math.max(roundZoomFactor(next), MIN_ZOOM_FACTOR),
    MAX_ZOOM_FACTOR,
  );
  if (clamped === zoomFactor) return;
  zoomFactor = clamped;
  applyTextScale(clamped);
  for (const listener of listeners) {
    listener();
  }
}

/** Read the persisted zoom factor outside of React. */
export function getZoomFactor(): number {
  return zoomFactor;
}

/** Set the zoom factor from UI (e.g. Settings dropdown). */
export function setZoomFactor(next: number): void {
  commitZoomFactor(next);
}

/** Reset the zoom factor to the default. */
export function resetZoomFactor(): void {
  commitZoomFactor(DEFAULT_ZOOM_FACTOR);
}

export const ZOOM_FACTOR_MIN = MIN_ZOOM_FACTOR;
export const ZOOM_FACTOR_MAX = MAX_ZOOM_FACTOR;
export const ZOOM_FACTOR_STEP = ZOOM_STEP;
export const ZOOM_FACTOR_DEFAULT = DEFAULT_ZOOM_FACTOR;

/** Reactive zoom factor — re-renders when keyboard shortcuts or UI change it. */
export function useZoomFactor(): number {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}

export function useWebviewZoomShortcuts() {
  React.useLayoutEffect(() => {
    const webview = getCurrentWebview();
    applyTextScale(zoomFactor);

    // Keep the webview coordinate system stable; only text should scale.
    void webview.setZoom(DEFAULT_ZOOM_FACTOR).catch((error) => {
      console.error("Failed to reset webview zoom", error);
    });

    function handleKeyDown(event: KeyboardEvent) {
      const action = getZoomAction(event);
      if (!action) {
        return;
      }

      event.preventDefault();

      const nextZoomFactor = getNextZoomFactor(action, zoomFactor);

      if (nextZoomFactor === zoomFactor) {
        return;
      }

      commitZoomFactor(nextZoomFactor);
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}
