import { getCurrentWindow } from "@tauri-apps/api/window";
import * as React from "react";

import { performTitleBarDoubleClickAction } from "@/shared/lib/titleBarActions";

const WINDOW_DRAG_HANDLE_HEIGHT = 44;
const WINDOW_DRAG_INTERACTIVE_SELECTOR =
  'button, a, input, textarea, select, [role="button"], [contenteditable="true"]';

function isWindowDragHandleEvent(event: MouseEvent | PointerEvent) {
  if (event.clientY > WINDOW_DRAG_HANDLE_HEIGHT) {
    return false;
  }

  const target = event.target;
  return !(
    target instanceof Element &&
    target.closest(WINDOW_DRAG_INTERACTIVE_SELECTOR)
  );
}

export function StartupWindowDragRegion() {
  React.useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (event.button !== 0 || event.detail > 1) {
        return;
      }

      if (!isWindowDragHandleEvent(event)) {
        return;
      }

      void getCurrentWindow().startDragging();
    }

    function handleDoubleClick(event: MouseEvent) {
      if (event.button !== 0 || !isWindowDragHandleEvent(event)) {
        return;
      }

      event.preventDefault();
      void performTitleBarDoubleClickAction();
    }

    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("dblclick", handleDoubleClick, true);
    };
  }, []);

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-x-0 top-0 z-20 h-10 select-none"
      data-tauri-drag-region
    />
  );
}
