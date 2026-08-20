import * as React from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { isMacPlatform } from "@/shared/lib/platform";

type CloseWindowChord = Pick<
  KeyboardEvent,
  | "altKey"
  | "code"
  | "ctrlKey"
  | "defaultPrevented"
  | "isComposing"
  | "metaKey"
  | "repeat"
  | "shiftKey"
>;

export function isCloseWindowShortcut(
  event: CloseWindowChord,
  isMac: boolean,
): boolean {
  return (
    isMac &&
    event.code === "KeyW" &&
    event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey &&
    !event.defaultPrevented &&
    !event.isComposing &&
    !event.repeat
  );
}

/**
 * Restores the standard macOS Cmd+W behavior without reclaiming the native
 * menu accelerator. Buzz Term handles the chord first in capture phase while
 * it owns input; otherwise this bubble-phase listener closes the current
 * window. The main window's Rust close handler turns that into hide-to-tray.
 */
export function useCloseWindowShortcut() {
  React.useEffect(() => {
    if (!isTauri()) return;

    function handleKeyDown(event: KeyboardEvent) {
      if (!isCloseWindowShortcut(event, isMacPlatform())) return;
      event.preventDefault();
      void getCurrentWindow().close();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);
}
