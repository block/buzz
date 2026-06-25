import * as React from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { isWindowDragHandleEvent } from "@/app/AppShell.helpers";

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

    function handleDoubleClick(event: MouseEvent) {
      if (event.button !== 0 || !isWindowDragHandleEvent(event)) {
        return;
      }

      event.preventDefault();
      void getCurrentWindow().toggleMaximize();
    }

    window.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("dblclick", handleDoubleClick, true);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("dblclick", handleDoubleClick, true);
    };
  }, []);
}
