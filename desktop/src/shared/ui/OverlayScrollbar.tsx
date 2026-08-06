import * as React from "react";

import { useOverlayScrollbar } from "@/shared/hooks/useOverlayScrollbar";

type OverlayScrollbarProps = {
  composerRef: React.RefObject<HTMLElement | null>;
  resetKey?: unknown;
  scrollElement?: HTMLElement | null;
  scrollRef?: React.RefObject<HTMLElement | null>;
};

export function OverlayScrollbar({
  composerRef,
  resetKey,
  scrollElement,
  scrollRef,
}: OverlayScrollbarProps) {
  const thumbRef = React.useRef<HTMLDivElement>(null);
  const scrollElementRef = React.useMemo(
    () => ({ current: scrollElement ?? null }),
    [scrollElement],
  );
  useOverlayScrollbar({
    composerRef,
    resetKey,
    scrollRef: scrollRef ?? scrollElementRef,
    thumbRef,
  });

  return (
    <div
      aria-hidden="true"
      className="pointer-events-none absolute right-[3px] top-0 z-50 w-2 touch-none rounded-full bg-border/80 opacity-0 transition-opacity duration-200"
      data-buzz-overlay-scrollbar
      ref={thumbRef}
    />
  );
}
