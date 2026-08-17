import type * as React from "react";

const SIDEBAR_CONTEXT_ICON_SLOT_CLASS =
  "flex h-4 w-4 shrink-0 items-center justify-center";

/**
 * Run a menu action on the next tick. Radix closes the menu on select and
 * restores focus; deferring the action avoids opening a modal dialog while
 * the menu is still tearing down (which can leave `pointer-events: none`
 * stuck on <body> and freeze the app).
 */
export function deferMenuAction(action: () => void) {
  globalThis.setTimeout(action, 0);
}

/**
 * A fixed-width leading slot for context-menu rows so that labels stay
 * left-aligned whether or not a given row has an icon. Pass no children for
 * an intentionally blank (but still aligned) slot.
 */
export function ContextMenuIconSlot({
  children,
}: {
  children?: React.ReactNode;
}) {
  return (
    <span
      aria-hidden="true"
      className={SIDEBAR_CONTEXT_ICON_SLOT_CLASS}
      data-sidebar-context-icon-slot
    >
      {children}
    </span>
  );
}
