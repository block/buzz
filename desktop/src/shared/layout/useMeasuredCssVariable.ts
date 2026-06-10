import * as React from "react";

import { observeElementBlockSize } from "./observeElementBlockSize";

type UseMeasuredCssVariableArgs = {
  cssVariable: string;
  enabled?: boolean;
  resetKey?: unknown;
  resetValue: string;
  sourceRef: React.RefObject<HTMLElement | null>;
  targetRef: React.RefObject<HTMLElement | null>;
};

/**
 * Observes an element's block size and writes it as a CSS custom property on a
 * target element. Uses `useLayoutEffect` so the first measurement happens
 * before paint.
 */
export function useMeasuredCssVariable({
  sourceRef,
  targetRef,
  cssVariable,
  resetValue,
  resetKey,
  enabled = true,
}: UseMeasuredCssVariableArgs) {
  React.useLayoutEffect(() => {
    void resetKey;

    if (!enabled) {
      return;
    }

    const sourceEl = sourceRef.current;
    const targetEl = targetRef.current;

    if (!sourceEl || !targetEl) {
      return;
    }

    let lastValue: number | null = null;

    const applySize = (height: number) => {
      const px = Math.ceil(height);
      if (lastValue !== null && Math.abs(px - lastValue) <= 1) {
        return;
      }

      lastValue = px;
      targetEl.style.setProperty(cssVariable, `${px}px`);
    };

    const disconnect = observeElementBlockSize(sourceEl, applySize);

    return () => {
      disconnect();
      targetEl.style.setProperty(cssVariable, resetValue);
    };
  }, [sourceRef, targetRef, cssVariable, resetValue, resetKey, enabled]);
}
