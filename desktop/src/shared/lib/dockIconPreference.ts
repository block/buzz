import { invoke, isTauri } from "@tauri-apps/api/core";
import * as React from "react";

/**
 * macOS-only preference. When enabled, closing the main window drops Buzz to a
 * menu-bar-only app: the Dock icon and the ⌘-Tab entry go away until Buzz is
 * reopened from its tray menu.
 *
 * The webview owns persistence, so the backend is told the current value at
 * startup and on every change; the close handler in Rust reads it from there.
 */
export const HIDE_DOCK_ICON_STORAGE_KEY = "buzz.appearance.hideDockIconOnClose";
export const DEFAULT_HIDE_DOCK_ICON_ON_CLOSE = false;

const listeners = new Set<() => void>();
let hideDockIconOnClose = readStoredHideDockIconOnClose();

function readStoredHideDockIconOnClose(): boolean {
  try {
    return (
      globalThis.localStorage?.getItem(HIDE_DOCK_ICON_STORAGE_KEY) === "true"
    );
  } catch {
    return DEFAULT_HIDE_DOCK_ICON_ON_CLOSE;
  }
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getHideDockIconOnClose(): boolean {
  return hideDockIconOnClose;
}

export function setHideDockIconOnClose(enabled: boolean): void {
  hideDockIconOnClose = enabled;
  try {
    globalThis.localStorage?.setItem(
      HIDE_DOCK_ICON_STORAGE_KEY,
      String(enabled),
    );
  } catch {
    // Persistence is best-effort; the in-memory preference still applies.
  }
  publishHideDockIconOnClose(enabled);
  for (const listener of listeners) listener();
}

function publishHideDockIconOnClose(enabled: boolean): void {
  if (!isTauri()) {
    return;
  }

  void invoke("set_hide_dock_icon_on_close", { enabled }).catch(
    (error: unknown) => {
      console.error("failed to publish dock icon preference", error);
    },
  );
}

/**
 * Hands the stored preference to the backend during startup. Without this the
 * backend would fall back to its default until the user toggles the setting.
 */
export function syncHideDockIconOnClose(): void {
  publishHideDockIconOnClose(hideDockIconOnClose);
}

export function useHideDockIconOnClose(): boolean {
  return React.useSyncExternalStore(
    subscribe,
    getHideDockIconOnClose,
    () => DEFAULT_HIDE_DOCK_ICON_ON_CLOSE,
  );
}
