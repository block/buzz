import { useEffect } from "react";

const KEYBOARD_NAVIGATION_KEYS = new Set([
  "Tab",
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "End",
  "Home",
  "PageDown",
  "PageUp",
]);

/**
 * Makes keyboard focus visible without treating a pointer click in a text field
 * as keyboard navigation. Browsers intentionally match :focus-visible for text
 * inputs even after a click, so this app-level modality fact gates the visual
 * treatment consistently for every control.
 */
export function useKeyboardFocusVisibility() {
  useEffect(() => {
    const root = document.documentElement;
    const useKeyboardNavigation = (event: KeyboardEvent) => {
      if (
        KEYBOARD_NAVIGATION_KEYS.has(event.key) &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.altKey
      ) {
        root.dataset.keyboardNavigation = "";
      }
    };
    const usePointerNavigation = () => {
      delete root.dataset.keyboardNavigation;
    };

    window.addEventListener("keydown", useKeyboardNavigation, true);
    window.addEventListener("pointerdown", usePointerNavigation, true);
    window.addEventListener("touchstart", usePointerNavigation, true);
    return () => {
      window.removeEventListener("keydown", useKeyboardNavigation, true);
      window.removeEventListener("pointerdown", usePointerNavigation, true);
      window.removeEventListener("touchstart", usePointerNavigation, true);
      delete root.dataset.keyboardNavigation;
    };
  }, []);
}
