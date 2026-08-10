import * as React from "react";

export function isDocumentVisible(): boolean {
  if (typeof document === "undefined") return true;
  return (
    document.visibilityState === "visible" &&
    (typeof document.hasFocus !== "function" || document.hasFocus())
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
  window.addEventListener("focus", handleVisibilityChange);
  window.addEventListener("blur", handleVisibilityChange);
  return () => {
    document.removeEventListener("visibilitychange", handleVisibilityChange);
    window.removeEventListener("focus", handleVisibilityChange);
    window.removeEventListener("blur", handleVisibilityChange);
  };
}

export function useDocumentVisible(): boolean {
  const [visible, setVisible] = React.useState(isDocumentVisible);

  React.useEffect(() => subscribeDocumentVisibility(setVisible), []);

  return visible;
}

export function useVisibleRefetchInterval(
  intervalMs: number | false,
): number | false {
  return useDocumentVisible() ? intervalMs : false;
}
