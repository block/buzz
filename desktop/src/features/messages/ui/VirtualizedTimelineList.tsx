import * as React from "react";
import type { Virtualizer } from "@tanstack/react-virtual";

import type { VirtualTimelineRow } from "@/features/messages/lib/buildVirtualTimelineRows";
import { formatDayHeading } from "@/features/messages/lib/dateFormatters";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { DayDivider } from "./DayDivider";

type VirtualizedTimelineListProps = {
  virtualizer: Virtualizer<HTMLDivElement, Element>;
  rows: VirtualTimelineRow[];
  /** Filtered main-timeline entries, indexed by `VirtualMessageRow.messageIndex`. */
  entries: MainTimelineEntry[];
  /**
   * The virtualizer's `scrollMargin` — the list's offset within the scroll
   * container (content above it: sentinel, spinner). `virtualItem.start`
   * is in scroll-element coords (includes this margin), so rows are positioned
   * at `start - scrollMargin` within the spacer, which sits at that offset.
   */
  scrollMargin: number;
  /**
   * Renders one message entry's content. Injected (rather than imported) so the
   * heavy `MessageRow` subtree stays out of this component's concern and the
   * list is testable in isolation. `MessageTimeline` passes the real
   * `renderTimelineEntry` bound to its render context.
   */
  renderEntry: (entry: MainTimelineEntry) => React.ReactNode;
  /** Renders the channel intro header when an intro row is in the virtual list. */
  renderIntro?: () => React.ReactNode;
};

/**
 * Presentational virtualized main timeline. Renders only the rows in view
 * (+overscan) from the flat `rows` list, reusing `DayDivider` and the injected
 * `renderEntry`, so the painted output matches the classic nested list exactly.
 *
 * The virtualizer and its scroll behavior are owned by the parent
 * (`MessageTimeline` + `useVirtualTimelineScroll`); this component just paints
 * the spacer and absolutely-positioned rows the virtualizer reports.
 */
export const VirtualizedTimelineList = React.memo(
  function VirtualizedTimelineList({
    virtualizer,
    rows,
    entries,
    scrollMargin,
    renderEntry,
    renderIntro,
  }: VirtualizedTimelineListProps) {
    const virtualItems = virtualizer.getVirtualItems();

    return (
      <div
        style={{
          height: virtualizer.getTotalSize(),
          width: "100%",
          position: "relative",
        }}
      >
        {virtualItems.map((virtualItem) => {
          const row = rows[virtualItem.index];
          if (!row) {
            return null;
          }
          const entry =
            row.kind === "message" ? entries[row.messageIndex] : undefined;
          return (
            <div
              key={virtualItem.key}
              data-index={virtualItem.index}
              ref={virtualizer.measureElement}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${virtualItem.start - scrollMargin}px)`,
              }}
            >
              {row.kind === "intro" ? (
                renderIntro?.()
              ) : row.kind === "day-divider" ? (
                <DayDivider label={formatDayHeading(row.headingTimestamp)} />
              ) : entry ? (
                renderEntry(entry)
              ) : null}
            </div>
          );
        })}
      </div>
    );
  },
);
