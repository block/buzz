import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const DEFAULT_ZOOM_FACTOR = 1;
const MIN_ZOOM_FACTOR = 0.2;
const MAX_ZOOM_FACTOR = 10;
const ZOOM_STEP = 0.2;

type ZoomAction = "increase" | "decrease" | "reset";

function getZoomAction(event: KeyboardEvent): ZoomAction | null {
  if (!hasPrimaryShortcutModifier(event) || event.altKey) {
    return null;
  }

  if (
    event.key === "+" ||
    event.key === "=" ||
    event.code === "Equal" ||
    event.code === "NumpadAdd"
  ) {
    return "increase";
  }

  if (
    !event.shiftKey &&
    (event.key === "-" ||
      event.code === "Minus" ||
      event.code === "NumpadSubtract")
  ) {
    return "decrease";
  }

  if (
    !event.shiftKey &&
    (event.key === "0" || event.code === "Digit0" || event.code === "Numpad0")
  ) {
    return "reset";
  }

  return null;
}

function getNextZoomFactor(action: ZoomAction, zoomFactor: number) {
  if (action === "reset") {
    return DEFAULT_ZOOM_FACTOR;
  }

  if (action === "increase") {
    return Math.min(zoomFactor + ZOOM_STEP, MAX_ZOOM_FACTOR);
  }

  return Math.max(zoomFactor - ZOOM_STEP, MIN_ZOOM_FACTOR);
}

export function useWebviewZoomShortcuts() {
  const zoomFactorRef = React.useRef(DEFAULT_ZOOM_FACTOR);

  React.useLayoutEffect(() => {
    const webview = getCurrentWebview();

    function handleKeyDown(event: KeyboardEvent) {
      const action = getZoomAction(event);
      if (!action) {
        return;
      }

      event.preventDefault();

      const previousZoomFactor = zoomFactorRef.current;
      const nextZoomFactor = getNextZoomFactor(action, previousZoomFactor);

      if (nextZoomFactor === previousZoomFactor) {
        return;
      }

      zoomFactorRef.current = nextZoomFactor;

      void webview.setZoom(nextZoomFactor).catch((error) => {
        zoomFactorRef.current = previousZoomFactor;
        console.error("Failed to update webview zoom", error);
      });
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}
