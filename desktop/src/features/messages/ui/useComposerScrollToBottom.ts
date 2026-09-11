import * as React from "react";

/** Scroll the current composer element after its next layout frame. */
export function useComposerScrollToBottom(
  composerScrollRef: React.RefObject<HTMLDivElement | null>,
) {
  return React.useCallback(() => {
    window.requestAnimationFrame(() => {
      const scrollElement = composerScrollRef.current;
      if (!scrollElement) return;
      scrollElement.scrollTop = scrollElement.scrollHeight;
    });
  }, [composerScrollRef]);
}
