import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * Toggles the native File > Close Window (⌘W) menu item.
 *
 * Buzz Term claims ⌘W for "close this terminal tab" while it owns the
 * keyboard. macOS resolves the menu key equivalent before the webview sees
 * any key event, so the item must be disabled for the chord to reach the
 * terminal at all — a disabled item does not consume its key equivalent.
 * No-op outside Tauri; the Rust command is itself a no-op off macOS, where
 * the menu is never installed.
 */
export async function setCloseWindowMenuEnabled(
  enabled: boolean,
): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    await invoke("set_close_window_menu_enabled", { enabled });
  } catch {
    // No-op on failure; the item defaults to enabled and a stale menu state
    // is recoverable by toggling terminal ownership again.
  }
}
