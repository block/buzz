import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import { requestRendererReload } from "@/shared/lib/reloadRenderer";

/** Reloads the webview after bounded native WebSocket teardown. */
export function useReloadShortcut() {
  React.useEffect(() => {
    async function handleKeyDown(event: KeyboardEvent) {
      if (
        !hasPrimaryShortcutModifier(event) ||
        event.altKey ||
        event.shiftKey ||
        event.key.toLowerCase() !== "r"
      ) {
        return;
      }

      event.preventDefault();
      await requestRendererReload();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);
}
