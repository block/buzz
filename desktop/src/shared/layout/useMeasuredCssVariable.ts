import * as React from "react";

import {
  observeElementBlockSize,
  observeElementInlineSize,
} from "./observeElementBlockSize";

type UseMeasuredCssVariableArgs = {
  cssVariable: string;
  /** Which box dimension to observe. Defaults to `"block"` (height). */
  dimension?: "block" | "inline";
  enabled?: boolean;
  resetKey?: unknown;
  resetValue: string;
  targetRef: React.RefObject<HTMLElement | null>;
};

/**
 * Observes an element's block (or inline) size and writes it as a CSS custom
 * property on a target element. Uses `useLayoutEffect` so the first
 * measurement happens before paint.
 *
 * Returns a callback ref for the source element. Attach it instead of a ref
 * object so the measurement re-runs when the source mounts later than this
 * hook's owner — e.g. inside a lazy-loaded subtree.
 */
export function useMeasuredCssVariable({
  targetRef,
  cssVariable,
  dimension = "block",
  resetValue,
  resetKey,
  enabled = true,
}: UseMeasuredCssVariableArgs): React.RefCallback<HTMLElement> {
  const [sourceEl, setSourceEl] = React.useState<HTMLElement | null>(null);

  React.useLayoutEffect(() => {
    void resetKey;

    if (!enabled) {
      return;
    }

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

    const observe =
      dimension === "inline"
        ? observeElementInlineSize
        : observeElementBlockSize;
    const disconnect = observe(sourceEl, applySize);

    return () => {
      disconnect();
      targetEl.style.setProperty(cssVariable, resetValue);
    };
  }, [
    sourceEl,
    targetRef,
    cssVariable,
    dimension,
    resetValue,
    resetKey,
    enabled,
  ]);

  return setSourceEl;
}
