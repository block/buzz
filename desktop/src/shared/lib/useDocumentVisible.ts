import * as React from "react";

import { scheduleAfterForegroundReady } from "@/shared/lib/foregroundReady";

export function isDocumentVisible(): boolean {
  if (typeof document === "undefined") return true;
  return document.visibilityState === "visible";
}

export function isAppFocused(): boolean {
  if (!isDocumentVisible()) return false;
  return (
    typeof document === "undefined" ||
    typeof document.hasFocus !== "function" ||
    document.hasFocus()
  );
}

export function subscribeDocumentVisibility(
  listener: (visible: boolean) => void,
): () => void {
  if (typeof document === "undefined") {
    return () => {};
  }

  const handleVisibilityChange = () => listener(isDocumentVisible());
  document.addEventListener("visibilitychange", handleVisibilityChange);
  return () => {
    document.removeEventListener("visibilitychange", handleVisibilityChange);
  };
}

export function subscribeAppFocus(
  listener: (focused: boolean) => void,
): () => void {
  if (typeof document === "undefined") {
    return () => {};
  }

  let cancelPendingResume: (() => void) | null = null;
  const handleFocusChange = () => {
    cancelPendingResume?.();
    cancelPendingResume = null;
    const focused = isAppFocused();
    if (!focused) {
      listener(false);
      return;
    }
    cancelPendingResume = scheduleAfterForegroundReady(() => {
      cancelPendingResume = null;
      if (isAppFocused()) listener(true);
    });
  };
  document.addEventListener("visibilitychange", handleFocusChange);
  window.addEventListener("focus", handleFocusChange);
  window.addEventListener("blur", handleFocusChange);
  return () => {
    cancelPendingResume?.();
    document.removeEventListener("visibilitychange", handleFocusChange);
    window.removeEventListener("focus", handleFocusChange);
    window.removeEventListener("blur", handleFocusChange);
  };
}

export function useDocumentVisible(): boolean {
  const [visible, setVisible] = React.useState(isDocumentVisible);

  React.useEffect(() => subscribeDocumentVisibility(setVisible), []);

  return visible;
}

const appFocusStoreListeners = new Set<() => void>();
let appFocusStoreSnapshot = isAppFocused();
let stopAppFocusStore: (() => void) | null = null;

function subscribeAppFocusStore(listener: () => void): () => void {
  appFocusStoreListeners.add(listener);
  if (!stopAppFocusStore) {
    appFocusStoreSnapshot = isAppFocused();
    stopAppFocusStore = subscribeAppFocus((focused) => {
      if (appFocusStoreSnapshot === focused) return;
      appFocusStoreSnapshot = focused;
      for (const storeListener of appFocusStoreListeners) storeListener();
    });
  }

  return () => {
    appFocusStoreListeners.delete(listener);
    if (appFocusStoreListeners.size === 0) {
      stopAppFocusStore?.();
      stopAppFocusStore = null;
    }
  };
}

export function useAppFocused(): boolean {
  return React.useSyncExternalStore(
    subscribeAppFocusStore,
    () => appFocusStoreSnapshot,
    () => true,
  );
}

export function useFocusedRefetchInterval(
  intervalMs: number | false,
): number | false {
  return useAppFocused() ? intervalMs : false;
}
