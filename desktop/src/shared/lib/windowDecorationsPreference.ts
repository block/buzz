import * as React from "react";

const STORAGE_KEY = "buzz.window.nativeDecorations";
const DEFAULT_WINDOW_DECORATIONS_VISIBLE = true;

const listeners = new Set<() => void>();

let windowDecorationsVisible = readStoredWindowDecorationsVisible();

function parseWindowDecorationsVisible(
  value: string | null | undefined,
): boolean {
  if (value === "true") return true;
  if (value === "false") return false;
  return DEFAULT_WINDOW_DECORATIONS_VISIBLE;
}

function readStoredWindowDecorationsVisible(): boolean {
  try {
    return parseWindowDecorationsVisible(
      globalThis.localStorage?.getItem(STORAGE_KEY),
    );
  } catch {
    return DEFAULT_WINDOW_DECORATIONS_VISIBLE;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): boolean {
  return windowDecorationsVisible;
}

function getServerSnapshot(): boolean {
  return DEFAULT_WINDOW_DECORATIONS_VISIBLE;
}

/** Read the native window-frame preference outside React. */
export function getWindowDecorationsVisible(): boolean {
  return windowDecorationsVisible;
}

/** Update whether the operating system draws Buzz's native window frame. */
export function setWindowDecorationsVisible(visible: boolean): void {
  windowDecorationsVisible = visible;

  try {
    globalThis.localStorage?.setItem(STORAGE_KEY, String(visible));
  } catch {
    // Persistence is best-effort; the in-memory preference still applies.
  }

  for (const listener of listeners) {
    listener();
  }
}

/** Whether the native title bar and its window controls should be visible. */
export function useWindowDecorationsVisible(): boolean {
  return React.useSyncExternalStore(subscribe, getSnapshot, getServerSnapshot);
}
