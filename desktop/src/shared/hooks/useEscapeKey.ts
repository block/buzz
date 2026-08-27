import { useIsPresent } from "motion/react";
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
 * A surface animating out under `AnimatePresence` stands down: it stays
 * registered (so background shortcuts keep yielding for the duration) but no
 * longer acts on the key. `AnimatePresence` keeps a replaced surface mounted
 * through its exit, so during a replacement two surfaces listen at once, and
 * the outgoing one registered first — it would mark the press
 * `defaultPrevented` and its successor, respecting exactly that flag, would
 * ignore it. The user sees a swallowed keypress and has to press Escape twice.
 * Outside `AnimatePresence` there is no presence context and this is always
 * true, so surfaces that never animate out are unaffected.
 *
 * Pass `enabled: false` to skip registering the listener entirely.
 */
export function useEscapeKey(onEscape: () => void, enabled: boolean = true) {
  // Read through a ref rather than an effect dependency so entering the exit
  // phase does not release and re-acquire the surface registration.
  const isPresentRef = React.useRef(true);
  isPresentRef.current = useIsPresent();

  React.useEffect(() => {
    if (!enabled) return;
    const releaseSurface = acquireEscapeSurface();
    function handleKeyDown(event: KeyboardEvent) {
      if (!isPresentRef.current) return;
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
