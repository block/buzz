import * as React from "react";

/** Distance from the bottom (px) within which the view stays pinned. */
const PIN_THRESHOLD = 48;

/**
 * Keep a scroll container pinned to the bottom while its content grows
 * (live agent output, replies loading in), unless the user scrolled up.
 * A resetKey change (channel or thread switch) re-pins the view.
 */
export function usePinnedScroll(resetKey: string) {
  const scrollRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  const pinnedRef = React.useRef(true);

  const handleScroll = React.useCallback(() => {
    const node = scrollRef.current;
    if (!node) return;
    pinnedRef.current =
      node.scrollHeight - node.scrollTop - node.clientHeight < PIN_THRESHOLD;
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — a resetKey change re-pins the view to the bottom
  React.useLayoutEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
    pinnedRef.current = true;
  }, [resetKey]);

  React.useEffect(() => {
    const content = contentRef.current;
    const scroller = scrollRef.current;
    if (!content || !scroller) return;
    const observer = new ResizeObserver(() => {
      if (pinnedRef.current) {
        scroller.scrollTop = scroller.scrollHeight;
      }
    });
    observer.observe(content);
    // The scroller itself resizes when the composer grows (newlines, drag)
    // or the split moves — a pinned view must stay glued to the bottom.
    observer.observe(scroller);
    return () => observer.disconnect();
  }, []);

  return { scrollRef, contentRef, handleScroll };
}
