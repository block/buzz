import * as React from "react";

export function useProjectsScrollIndicator() {
  const scrollIdleTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const scrollIndicatorRef = React.useRef<HTMLDivElement | null>(null);

  // The native scrollbar thumb is permanently transparent (WebKit won't
  // re-resolve ::-webkit-scrollbar styles dynamically), so paint a custom
  // indicator over the gutter and show it only while the area is scrolling.
  const handleContentScroll = React.useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      const element = event.currentTarget;
      const indicator = scrollIndicatorRef.current;
      if (!indicator) return;

      const { clientHeight, scrollHeight, scrollTop } = element;
      if (scrollHeight <= clientHeight) {
        indicator.style.opacity = "0";
        return;
      }

      const thumbHeight = Math.max(
        24,
        (clientHeight / scrollHeight) * clientHeight,
      );
      const maxOffset = clientHeight - thumbHeight;
      const offset = (scrollTop / (scrollHeight - clientHeight)) * maxOffset;
      indicator.style.height = `${thumbHeight}px`;
      indicator.style.transform = `translateY(${offset}px)`;
      indicator.style.opacity = "1";

      if (scrollIdleTimerRef.current !== null) {
        globalThis.clearTimeout(scrollIdleTimerRef.current);
      }
      scrollIdleTimerRef.current = globalThis.setTimeout(() => {
        indicator.style.opacity = "0";
        scrollIdleTimerRef.current = null;
      }, 700);
    },
    [],
  );

  return { handleContentScroll, scrollIndicatorRef };
}
