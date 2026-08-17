import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const SIDEBAR_KEYBOARD_SHORTCUT = "b";
const SIDEBAR_KEYBOARD_SHORTCUT_ALIAS = "s";

function isEditableKeyboardTarget(target: EventTarget | null) {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  return (
    target.closest("input, textarea, select, [contenteditable='true']") !== null
  );
}

export function isSidebarToggleShortcut(event: KeyboardEvent) {
  if (
    !hasPrimaryShortcutModifier(event) ||
    event.altKey ||
    event.shiftKey ||
    event.repeat ||
    event.defaultPrevented
  ) {
    return false;
  }

  const key = event.key.toLowerCase();
  return (
    key === SIDEBAR_KEYBOARD_SHORTCUT_ALIAS ||
    (key === SIDEBAR_KEYBOARD_SHORTCUT &&
      !isEditableKeyboardTarget(event.target))
  );
}
