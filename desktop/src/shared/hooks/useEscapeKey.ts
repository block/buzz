import * as React from "react";

/**
 * Calls `onEscape` when the Escape key is pressed, unless the event
 * was already handled (`defaultPrevented`).
 *
 * Pass `enabled: false` to skip registering the listener entirely.
 */
export function useEscapeKey(
  onEscape: () => void,
  enabled: boolean = true,
  options: { capture?: boolean } = {},
) {
  const capture = options.capture ?? false;

  React.useEffect(() => {
    if (!enabled) return;
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape" && !event.defaultPrevented) {
        event.preventDefault();
        onEscape();
      }
    }
    window.addEventListener("keydown", handleKeyDown, { capture });
    return () =>
      window.removeEventListener("keydown", handleKeyDown, { capture });
  }, [capture, enabled, onEscape]);
}
