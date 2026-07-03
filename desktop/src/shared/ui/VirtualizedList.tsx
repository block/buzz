import { type Virtualizer, useVirtualizer } from "@tanstack/react-virtual";
import * as React from "react";
import { createPortal } from "react-dom";

import { cn } from "@/shared/lib/cn";

export type ListVirtualizer = Virtualizer<HTMLElement, Element>;

/**
 * A headless virtualized list primitive using @tanstack/react-virtual.
 *
 * Migration contract:
 * - Rows must tolerate unmount/remount (no DOM-resident state that can't be
 *   reconstructed from props/data). Surfaces with in-DOM row state (open
 *   `<details>`, drag-and-drop) should use `content-visibility` instead.
 * - Rows may have variable height — the library's `measureElement` handles
 *   dynamic sizing automatically.
 *
 * Supports:
 * (a) Optional floating header fed the topmost visible row index. It is
 *     portaled into a caller-owned container OUTSIDE the scroll element
 *     (`headerOverlayRef`) so it pins to a fixed viewport offset regardless of
 *     scroll position — a `position: sticky` header inside the scroll element
 *     drifts at scrollTop 0, where it reveals its natural flow offset instead
 *     of the pinned offset. The render fn still re-runs with the virtualizer on
 *     every scroll (the absolutely-positioned rows cannot stay `position:
 *     sticky` once they leave the virtual window), but the re-render stays
 *     localized here rather than forcing the caller to re-render per scroll.
 * (b) Optional externally-owned scroll container — pass `scrollRef` when the
 *     caller already owns the scrolling element (a surface that shares its
 *     scroll region with non-row siblings). When omitted, VirtualizedList
 *     renders its own `overflow-y-auto` container.
 */
type VirtualizedListProps<T> = {
  /** The data items to virtualize. */
  items: T[];
  /** Stable key extractor for each item. */
  getItemKey: (item: T, index: number) => string | number;
  /** Render function for each row. Receives the item and its index. */
  renderItem: (item: T, index: number) => React.ReactNode;
  /** Estimated row height in px — used before measurement. */
  estimateSize?: number;
  /**
   * Optional floating header rendered with the index of the topmost visible
   * row (so it can label what's on screen). When `headerOverlayRef` points at a
   * mounted element, the result is portaled there — a non-scrolling container
   * outside the scroll element — so it pins to a fixed offset and cannot drift
   * with scroll position. `null` before the first row is in view.
   */
  stickyHeader?: (topVisibleIndex: number | null) => React.ReactNode;
  /**
   * Portal target for `stickyHeader`. Must sit OUTSIDE the scroll element (a
   * `position: sticky` header inside it drifts at scrollTop 0). The header is
   * portaled here only once the ref attaches.
   */
  headerOverlayRef?: React.RefObject<HTMLElement | null>;
  /**
   * Externally-owned scroll container. When provided, no internal scroll
   * container is rendered — the caller's element scrolls and is measured.
   */
  scrollRef?: React.RefObject<HTMLElement | null>;
  /** Class name for the internal scroll container (ignored when scrollRef is provided). */
  className?: string;
  /** Class name for the inner spacer div that holds the virtual rows. */
  innerClassName?: string;
  /** Overscan — number of items to render outside the visible area. */
  overscan?: number;
  /** Receives the virtualizer instance (for `scrollToIndex`, etc). */
  onVirtualizer?: (virtualizer: ListVirtualizer) => void;
};

export function VirtualizedList<T>({
  items,
  getItemKey,
  renderItem,
  estimateSize = 80,
  stickyHeader,
  headerOverlayRef,
  scrollRef,
  className,
  innerClassName,
  overscan = 5,
  onVirtualizer,
}: VirtualizedListProps<T>) {
  const internalScrollRef = React.useRef<HTMLDivElement>(null);
  const spacerRef = React.useRef<HTMLDivElement>(null);
  const ownsScroll = scrollRef === undefined;
  const resolvedScrollRef = scrollRef ?? internalScrollRef;
  // Read the element lazily inside the callback so the virtualizer picks it up
  // once the ref attaches — capturing `ref.current` at render time would freeze
  // it at the first-render `null`.
  const getScrollElement = React.useCallback(
    () => resolvedScrollRef.current,
    [resolvedScrollRef],
  );

  // When a sticky header (or any caller content) sits above the rows in the
  // same scroll container, the row spacer no longer starts at scrollTop 0.
  // Feed that offset to the virtualizer as `scrollMargin` so the visible-range
  // math stays aligned; without it the wrong rows render near the top.
  const [scrollMargin, setScrollMargin] = React.useState(0);
  React.useLayoutEffect(() => {
    const scrollEl = resolvedScrollRef.current;
    const spacer = spacerRef.current;
    if (!scrollEl || !spacer) {
      return;
    }
    const offset =
      spacer.getBoundingClientRect().top -
      scrollEl.getBoundingClientRect().top +
      scrollEl.scrollTop;
    setScrollMargin((prev) => (prev === offset ? prev : offset));
  });

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement,
    estimateSize: () => estimateSize,
    getItemKey: (index) => getItemKey(items[index], index),
    overscan,
    scrollMargin,
  });

  // Register in a layout effect, not a passive one: when this list remounts on
  // channel switch the parent's mount-pin runs in a layout effect on the same
  // commit, and child layout effects fire before parent layout effects. A
  // passive effect would publish the fresh virtualizer too late and the parent
  // would pin against the previous channel's stale instance.
  React.useLayoutEffect(() => {
    onVirtualizer?.(virtualizer);
  }, [onVirtualizer, virtualizer]);

  const virtualItems = virtualizer.getVirtualItems();

  // The header portals into `headerOverlayRef`, which the parent mounts before
  // this child — so it is populated on first commit. The dependency-free state
  // flip re-renders once after mount to cover the rare case where the parent
  // attaches the ref in the same commit as our first paint.
  const [headerHost, setHeaderHost] = React.useState<HTMLElement | null>(null);
  React.useEffect(() => {
    setHeaderHost(headerOverlayRef?.current ?? null);
  }, [headerOverlayRef]);

  const header = stickyHeader?.(virtualItems[0]?.index ?? null);

  const content = (
    <>
      {headerHost && header ? createPortal(header, headerHost) : null}
      <div
        className={cn("relative w-full", innerClassName)}
        ref={spacerRef}
        style={{ height: `${virtualizer.getTotalSize()}px` }}
      >
        {virtualItems.map((virtualRow) => (
          <div
            data-index={virtualRow.index}
            key={virtualRow.key}
            ref={virtualizer.measureElement}
            style={{
              position: "absolute",
              top: 0,
              left: 0,
              width: "100%",
              transform: `translateY(${virtualRow.start - scrollMargin}px)`,
            }}
          >
            {renderItem(items[virtualRow.index], virtualRow.index)}
          </div>
        ))}
      </div>
    </>
  );

  if (ownsScroll) {
    return (
      <div className={cn("overflow-y-auto", className)} ref={internalScrollRef}>
        {content}
      </div>
    );
  }

  return content;
}
