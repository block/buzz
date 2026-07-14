import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

/**
 * Reloads the webview on the platform's reload shortcut (Cmd+R on macOS,
 * Ctrl+R elsewhere), matching browser behavior.
 *
 * `window.location.reload()` is the app's existing reload primitive (see
 * App.tsx, useCommunityInit.ts): it triggers a full reinit that re-reads
 * localStorage and reconnects relays.
 */
export function useReloadShortcut() {
  React.useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (
        !hasPrimaryShortcutModifier(event) ||
        event.altKey ||
        event.shiftKey
      ) {
        return;
      }
      if (event.key.toLowerCase() !== "r") {
        return;
      }

      event.preventDefault();
      window.location.reload();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}
