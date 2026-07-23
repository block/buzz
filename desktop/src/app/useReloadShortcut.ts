import * as React from "react";

import { closeAllWebSockets } from "@/shared/api/relayWebSocketClose";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const RELOAD_TEARDOWN_TIMEOUT_MS = 500;

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
      await Promise.race([
        closeAllWebSockets(),
        new Promise<void>((resolve) =>
          window.setTimeout(resolve, RELOAD_TEARDOWN_TIMEOUT_MS),
        ),
      ]);
      window.location.reload();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, []);
}
