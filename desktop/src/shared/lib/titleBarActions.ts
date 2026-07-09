import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * Runs the window action that matches the OS "double-click a window's title
 * bar to" preference (macOS `AppleActionOnDoubleClick`).
 *
 * The app uses a web-based title-bar drag region, so the OS does not act on a
 * title-bar double-click on its own. Previously this hardcoded
 * `toggleMaximize()`, which fought the native minimize behavior and produced a
 * minimize/restore flicker for users whose system pref was "Minimize". The
 * Rust `title_bar_double_click` command reads the preference and dispatches a
 * single matching action (native `miniaturize:` for minimize, so the window
 * genies into the Dock).
 */
export async function performTitleBarDoubleClickAction(): Promise<void> {
  if (!isTauri()) {
    return;
  }

  try {
    await invoke("title_bar_double_click");
  } catch {
    // No-op on failure — a missed maximize toggle is not worth surfacing.
  }
}
