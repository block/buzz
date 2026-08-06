/**
 * Documents preferences, stored per-machine alongside the vault path.
 *
 * Currently one setting: whether to trust live preview with files the
 * round-trip guard flags. Off by default — the guard exists because this
 * editor genuinely cannot represent callouts, footnotes or raw HTML, and
 * autosaving one would rewrite it. (Tables used to head that list; they gained
 * a schema node and now round-trip, which is why the copy names only what is
 * still true.) Turning it on is a considered choice, so it lives in Settings
 * rather than behind a per-file prompt.
 */
import * as React from "react";

export const ALWAYS_LIVE_PREVIEW_KEY = "buzz.documents.alwaysLivePreview.v1";

type Listener = () => void;
const listeners = new Set<Listener>();

function read(): boolean {
  try {
    return window.localStorage.getItem(ALWAYS_LIVE_PREVIEW_KEY) === "1";
  } catch {
    return false;
  }
}

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  const handleStorage = (event: StorageEvent) => {
    if (event.key === ALWAYS_LIVE_PREVIEW_KEY) emit();
  };
  window.addEventListener("storage", handleStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", handleStorage);
  };
}

export function setAlwaysLivePreview(enabled: boolean): void {
  try {
    window.localStorage.setItem(ALWAYS_LIVE_PREVIEW_KEY, enabled ? "1" : "0");
  } catch {
    // Keep the in-memory value usable even when storage is unavailable.
  }
  emit();
}

/**
 * Whether notes should open in live preview regardless of the guard's verdict.
 *
 * Only affects the *default* mode. Source mode remains one click away either
 * way, and the guard still classifies every file — this changes which side of
 * that classification the editor starts on.
 */
export function useAlwaysLivePreview(): boolean {
  return React.useSyncExternalStore(subscribe, read, () => false);
}
