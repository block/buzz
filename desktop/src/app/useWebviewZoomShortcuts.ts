import * as React from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

const DEFAULT_ZOOM_FACTOR = 1.1;
const DEFAULT_WEBVIEW_ZOOM_FACTOR = 1;
const MIN_ZOOM_FACTOR = 0.75;
const MAX_ZOOM_FACTOR = 1.5;
const ZOOM_STEP = 0.1;
const BASE_FONT_SIZE_PX = 16;
const TEXT_SCALE_STORAGE_KEY = "buzz:text-scale";

type ZoomAction = "increase" | "decrease" | "reset";

function roundZoomFactor(zoomFactor: number) {
  return Math.round(zoomFactor * 10) / 10;
}

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
    return Math.min(roundZoomFactor(zoomFactor + ZOOM_STEP), MAX_ZOOM_FACTOR);
  }

  return Math.max(roundZoomFactor(zoomFactor - ZOOM_STEP), MIN_ZOOM_FACTOR);
}

type StoredZoomFactor = {
  zoomFactor: number;
  hasStoredPreference: boolean;
};

type ApplyTextScaleOptions = {
  persistPreference?: boolean;
};

function readStoredZoomFactor(): StoredZoomFactor {
  const raw = window.localStorage.getItem(TEXT_SCALE_STORAGE_KEY);
  if (!raw) {
    return { zoomFactor: DEFAULT_ZOOM_FACTOR, hasStoredPreference: false };
  }

  const parsed = Number.parseFloat(raw);
  if (!Number.isFinite(parsed)) {
    return { zoomFactor: DEFAULT_ZOOM_FACTOR, hasStoredPreference: false };
  }

  return {
    zoomFactor: Math.min(Math.max(parsed, MIN_ZOOM_FACTOR), MAX_ZOOM_FACTOR),
    hasStoredPreference: true,
  };
}

function applyTextScale(
  zoomFactor: number,
  {
    persistPreference = zoomFactor !== DEFAULT_ZOOM_FACTOR,
  }: ApplyTextScaleOptions = {},
) {
  if (zoomFactor === DEFAULT_WEBVIEW_ZOOM_FACTOR) {
    document.documentElement.style.fontSize = "";
  } else {
    document.documentElement.style.fontSize = `${BASE_FONT_SIZE_PX * zoomFactor}px`;
  }

  if (!persistPreference) {
    window.localStorage.removeItem(TEXT_SCALE_STORAGE_KEY);
    return;
  }

  window.localStorage.setItem(TEXT_SCALE_STORAGE_KEY, String(zoomFactor));
}

export function useWebviewZoomShortcuts() {
  const zoomFactorRef = React.useRef(DEFAULT_ZOOM_FACTOR);

  React.useLayoutEffect(() => {
    const webview = getCurrentWebview();
    const { zoomFactor: storedZoomFactor, hasStoredPreference } =
      readStoredZoomFactor();

    zoomFactorRef.current = storedZoomFactor;
    applyTextScale(storedZoomFactor, {
      persistPreference: hasStoredPreference,
    });

    // Keep the webview coordinate system stable; only text should scale.
    void webview.setZoom(DEFAULT_WEBVIEW_ZOOM_FACTOR).catch((error) => {
      console.error("Failed to reset webview zoom", error);
    });

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
      applyTextScale(nextZoomFactor);
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);
}
