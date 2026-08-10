import * as React from "react";

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

  const handleFocusChange = () => listener(isAppFocused());
  document.addEventListener("visibilitychange", handleFocusChange);
  window.addEventListener("focus", handleFocusChange);
  window.addEventListener("blur", handleFocusChange);
  return () => {
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

export function useAppFocused(): boolean {
  const [focused, setFocused] = React.useState(isAppFocused);

  React.useEffect(() => subscribeAppFocus(setFocused), []);

  return focused;
}

export function useFocusedRefetchInterval(
  intervalMs: number | false,
): number | false {
  return useAppFocused() ? intervalMs : false;
}
