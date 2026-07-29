import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import * as React from "react";

import { useWindowDecorationsVisible } from "@/shared/lib/windowDecorationsPreference";

/** Applies the persisted native window-frame preference to the Tauri window. */
export function useWindowDecorationsPreference() {
  const decorationsVisible = useWindowDecorationsVisible();

  React.useEffect(() => {
    if (!isTauri()) {
      return;
    }

    void getCurrentWindow()
      .setDecorations(decorationsVisible)
      .catch((error) => {
        console.warn("Unable to update native window decorations:", error);
      });
  }, [decorationsVisible]);
}
