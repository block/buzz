import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * Persistence + backend sync for the "keep Buzz running" setting.
 *
 * The frontend owns the source of truth (localStorage). The Rust side reads a
 * flag set via the `set_close_to_tray` command — it decides, at window-close
 * time, whether to hide the window to the tray instead of quitting. We push the
 * persisted value to the backend on launch and whenever the toggle changes.
 */
const CLOSE_TO_TRAY_KEY = "buzz-close-to-tray";

/** Read the persisted preference. Defaults to keeping Buzz running. */
export function getCloseToTrayPref(): boolean {
  try {
    return window.localStorage.getItem(CLOSE_TO_TRAY_KEY) !== "false";
  } catch {
    return true;
  }
}

/** Persist the preference to localStorage. */
export function setCloseToTrayPref(enabled: boolean): void {
  try {
    window.localStorage.setItem(CLOSE_TO_TRAY_KEY, enabled ? "true" : "false");
  } catch {
    // Best-effort — a storage failure just means the backend keeps the
    // last-applied value for this session.
  }
}

/** Push the current value to the Tauri backend. No-op outside Tauri. */
export async function applyCloseToTray(enabled: boolean): Promise<boolean> {
  if (!isTauri()) {
    return true;
  }
  try {
    await invoke("set_close_to_tray", { enabled });
    return true;
  } catch (err) {
    console.warn("set_close_to_tray command failed:", err);
    return false;
  }
}
