import React from "react";

export const PAGE_ZOOM_STEPS = [0.8, 0.9, 1, 1.1, 1.25, 1.5] as const;
const STORAGE_KEY = "bizfin.business.pageZoom";

export function stepPageZoom(current: number, direction: -1 | 1) {
  const index = PAGE_ZOOM_STEPS.reduce(
    (closest, value, candidate) =>
      Math.abs(value - current) < Math.abs(PAGE_ZOOM_STEPS[closest] - current)
        ? candidate
        : closest,
    0,
  );
  return PAGE_ZOOM_STEPS[
    Math.min(PAGE_ZOOM_STEPS.length - 1, Math.max(0, index + direction))
  ];
}

function savedZoom() {
  try {
    const value = Number(window.localStorage.getItem(STORAGE_KEY));
    return PAGE_ZOOM_STEPS.includes(value as (typeof PAGE_ZOOM_STEPS)[number])
      ? value
      : 1;
  } catch {
    return 1;
  }
}

export function usePageZoom() {
  const [zoom, setZoom] = React.useState(savedZoom);

  React.useLayoutEffect(() => {
    document.body.style.setProperty("zoom", String(zoom));
    document.body.style.setProperty("--page-zoom-inverse", String(1 / zoom));
    try {
      window.localStorage.setItem(STORAGE_KEY, String(zoom));
    } catch {
      // Storage can be unavailable in privacy-restricted embedded webviews.
    }
  }, [zoom]);

  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((!event.metaKey && !event.ctrlKey) || event.altKey) return;
      if (event.key === "+" || event.key === "=") {
        event.preventDefault();
        setZoom((current) => stepPageZoom(current, 1));
      } else if (event.key === "-") {
        event.preventDefault();
        setZoom((current) => stepPageZoom(current, -1));
      } else if (event.key === "0") {
        event.preventDefault();
        setZoom(1);
      }
    };
    window.addEventListener("keydown", onKeyDown, { capture: true });
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, []);

  return {
    zoom,
    zoomIn: () => setZoom((current) => stepPageZoom(current, 1)),
    zoomOut: () => setZoom((current) => stepPageZoom(current, -1)),
    resetZoom: () => setZoom(1),
  };
}
