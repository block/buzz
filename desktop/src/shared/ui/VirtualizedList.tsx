import { type Virtualizer, useVirtualizer } from "@tanstack/react-virtual";
import * as React from "react";

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
 * (a) Optional non-virtualized sticky-header slot rendered above the virtual
 *     rows inside the scroll container (for PulseView's sticky composer, etc).
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
  /** Optional non-virtualized content rendered above the virtual rows (sticky headers, etc). */
  stickyHeader?: React.ReactNode;
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

  React.useEffect(() => {
    onVirtualizer?.(virtualizer);
  }, [onVirtualizer, virtualizer]);

  const content = (
    <>
      {stickyHeader}
      <div
        className={cn("relative w-full", innerClassName)}
        ref={spacerRef}
        style={{ height: `${virtualizer.getTotalSize()}px` }}
      >
        {virtualizer.getVirtualItems().map((virtualRow) => (
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
