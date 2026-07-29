import * as React from "react";

/**
 * Keeps the keyboard anchored to a text input across window refocus and
 * shell clicks:
 *
 * - Refocusing the window restores whichever text input last had the
 *   keyboard (main composer, side-chat composer, palette search), falling
 *   back to the composer if it unmounted meanwhile.
 * - Clicks on non-interactive chrome must not blur the active input — this
 *   also covers the click that refocuses the window landing on dead space.
 *   Transcript areas opt out (data-allow-text-selection) so message text
 *   stays drag-selectable; the mouseup handler restores focus afterwards
 *   when the click ended without selecting text.
 *
 * While card selection owns the keyboard, nothing puts the caret back in a
 * box.
 */
export function useShellFocusGuards({
  cardSelectionActive,
  focusComposer,
}: {
  cardSelectionActive: boolean;
  focusComposer: () => void;
}) {
  const lastFocusedRef = React.useRef<HTMLElement | null>(null);

  const handleFocusCapture = React.useCallback(
    (event: React.FocusEvent<HTMLDivElement>) => {
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        target.matches("textarea, input, [contenteditable='true']")
      ) {
        lastFocusedRef.current = target;
      }
    },
    [],
  );

  React.useEffect(() => {
    const handleWindowFocus = () => {
      if (cardSelectionActive) return;
      const last = lastFocusedRef.current;
      if (last?.isConnected) {
        last.focus();
      } else {
        focusComposer();
      }
    };
    window.addEventListener("focus", handleWindowFocus);
    return () => window.removeEventListener("focus", handleWindowFocus);
  }, [cardSelectionActive, focusComposer]);

  const handleShellMouseDown = React.useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      const target = event.target;
      if (
        target instanceof HTMLElement &&
        !target.closest(
          "button, a, input, textarea, select, [role='button'], [role='separator'], [contenteditable='true'], [data-allow-text-selection]",
        )
      ) {
        event.preventDefault();
      }
    },
    [],
  );

  const handleShellMouseUp = React.useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      const target = event.target;
      if (
        !(target instanceof HTMLElement) ||
        !target.closest("[data-allow-text-selection]") ||
        target.closest("button, a, input, textarea, [contenteditable='true']")
      ) {
        return;
      }
      if (cardSelectionActive) return;
      requestAnimationFrame(() => {
        const selection = window.getSelection();
        if (selection && !selection.isCollapsed) return;
        const last = lastFocusedRef.current;
        if (last?.isConnected) {
          last.focus();
        } else {
          focusComposer();
        }
      });
    },
    [cardSelectionActive, focusComposer],
  );

  return { handleFocusCapture, handleShellMouseDown, handleShellMouseUp };
}
