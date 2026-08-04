import * as React from "react";

/**
 * Window-level keys while a prompt card is keyboard-selected:
 *
 * - ↑/↓ walk the cards
 * - ⏎ opens the selected card's side chat
 * - `e` edits the selected own prompt in the composer
 * - esc clears the selection
 *
 * Inert while the palette or shortcuts overlay is open, and yields to any
 * focused input (e.g. a click landed in one).
 */
export function useCardSelectionShortcuts({
  active,
  onNavigate,
  onOpenSelected,
  onEditSelected,
  onEscape,
}: {
  active: boolean;
  onNavigate: (direction: -1 | 1) => void;
  onOpenSelected: () => void;
  onEditSelected: () => void;
  onEscape: () => void;
}) {
  React.useEffect(() => {
    if (!active) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      if (
        event.target instanceof HTMLElement &&
        event.target.matches("textarea, input, [contenteditable='true']")
      ) {
        return;
      }
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        onNavigate(event.key === "ArrowUp" ? -1 : 1);
      } else if (event.key === "Enter") {
        event.preventDefault();
        onOpenSelected();
      } else if (event.key === "e") {
        event.preventDefault();
        onEditSelected();
      } else if (event.key === "Escape") {
        event.preventDefault();
        onEscape();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [active, onEditSelected, onEscape, onNavigate, onOpenSelected]);
}
