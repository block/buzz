import * as React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { isWindowDragHandleEvent } from "@/app/AppShell.helpers";
import { performTitleBarDoubleClickAction } from "@/shared/lib/titleBarActions";

export function useTauriWindowDrag() {
  React.useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (
        event.button !== 0 ||
        event.detail > 1 ||
        !isWindowDragHandleEvent(event)
      ) {
        return;
      }

      void getCurrentWindow().startDragging();
    }

    function stopTauriDragRegionHandler(event: MouseEvent) {
      if (event.button !== 0 || !isWindowDragHandleEvent(event)) {
        return;
      }

      // Tauri's injected data-tauri-drag-region listener hardcodes maximize on
      // double-click. Buzz handles drag and double-click itself so macOS title
      // bar preferences can be respected.
      event.stopImmediatePropagation();
    }

    function handleDoubleClick(event: MouseEvent) {
      if (event.button !== 0 || !isWindowDragHandleEvent(event)) {
        return;
      }

      event.preventDefault();
      event.stopImmediatePropagation();
      void performTitleBarDoubleClickAction();
    }

    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("mousedown", stopTauriDragRegionHandler, true);
    window.addEventListener("mouseup", stopTauriDragRegionHandler, true);
    window.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("mousedown", stopTauriDragRegionHandler, true);
      window.removeEventListener("mouseup", stopTauriDragRegionHandler, true);
      window.removeEventListener("dblclick", handleDoubleClick, true);
    };
  }, []);
}
