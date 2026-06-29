import * as React from "react";

import { AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX } from "@/shared/layout/auxiliaryPanelLayout";

const MOBILE_BREAKPOINT = 768;

/**
 * Returns `true` when the viewport is narrower than `breakpointPx`.
 * Uses `matchMedia` for efficient change detection.
 */
export function useMediaBreakpoint(breakpointPx: number): boolean {
  const [isBelow, setIsBelow] = React.useState<boolean>(() =>
    typeof window !== "undefined" ? window.innerWidth < breakpointPx : false,
  );

  React.useEffect(() => {
    const mql = window.matchMedia(`(max-width: ${breakpointPx - 1}px)`);
    const onChange = () => {
      setIsBelow(window.innerWidth < breakpointPx);
    };
    mql.addEventListener("change", onChange);
    setIsBelow(window.innerWidth < breakpointPx);
    return () => mql.removeEventListener("change", onChange);
  }, [breakpointPx]);

  return isBelow;
}

export function useElementWidthBreakpoint<T extends HTMLElement>(
  breakpointPx: number,
): [React.RefObject<T | null>, boolean] {
  const [ref, widthPx] = useElementWidth<T>();

  return [ref, widthPx > 0 && widthPx < breakpointPx];
}

export function useElementWidth<T extends HTMLElement>(): [
  React.RefObject<T | null>,
  number,
] {
  const ref = React.useRef<T>(null);
  const [widthPx, setWidthPx] = React.useState(0);

  React.useEffect(() => {
    let frameId: number | null = null;
    let cleanup: (() => void) | null = null;

    const attach = () => {
      const element = ref.current;
      if (!element) {
        frameId = window.requestAnimationFrame(attach);
        return;
      }

      const updateWidth = () => {
        setWidthPx(element.getBoundingClientRect().width);
      };

      updateWidth();

      if (typeof ResizeObserver === "undefined") {
        window.addEventListener("resize", updateWidth);
        cleanup = () => window.removeEventListener("resize", updateWidth);
        return;
      }

      const observer = new ResizeObserver(updateWidth);
      observer.observe(element);
      cleanup = () => observer.disconnect();
    };

    attach();

    return () => {
      if (frameId !== null) {
        window.cancelAnimationFrame(frameId);
      }
      cleanup?.();
    };
  }, []);

  return [ref, widthPx];
}

export function useIsMobile() {
  return useMediaBreakpoint(MOBILE_BREAKPOINT);
}

export function useIsAuxiliaryPanelOverlay() {
  return useMediaBreakpoint(AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX);
}

export function useIsThreadPanelOverlay() {
  return useIsAuxiliaryPanelOverlay();
}
