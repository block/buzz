import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import {
  adjustTextScale,
  applyCurrentTextScale,
  DEFAULT_TEXT_SCALE,
  type TextScaleAction,
} from "@/shared/lib/textScale";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

function getZoomAction(event: KeyboardEvent): TextScaleAction | null {
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

/**
 * Bootstraps persisted text scale and wires Cmd/Ctrl +/- / 0 shortcuts.
 * Layout zoom stays on the root font-size; native webview zoom is pinned at 1.
 */
export function useWebviewZoomShortcuts() {
  React.useLayoutEffect(() => {
    const webview = getCurrentWebview();

    applyCurrentTextScale();

    // Keep the webview coordinate system stable; only text should scale.
    void webview.setZoom(DEFAULT_TEXT_SCALE).catch((error) => {
      console.error("Failed to reset webview zoom", error);
    });

    function handleKeyDown(event: KeyboardEvent) {
      const action = getZoomAction(event);
      if (!action) {
        return;
      }

      event.preventDefault();
      adjustTextScale(action);
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}
