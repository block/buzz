import * as React from "react";

import { acquireEscapeSurface } from "@/shared/hooks/escapeSurfaces";

/**
 * Calls `onEscape` when the Escape key is pressed, unless the event
 * was already handled (`defaultPrevented`) — so nested controls
 * (autocomplete, edit mode) that claim Escape on the element always win.
 *
 * While enabled, the surface is registered with `escapeSurfaces` so
 * app-level Escape shortcuts (mark channel read) know to yield instead
 * of racing this listener on registration order.
 *
 * Pass `enabled: false` to skip registering the listener entirely.
 */
export function useEscapeKey(onEscape: () => void, enabled: boolean = true) {
  React.useEffect(() => {
    if (!enabled) return;
    const releaseSurface = acquireEscapeSurface();
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        onEscape();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      releaseSurface();
    };
  }, [enabled, onEscape]);
}
