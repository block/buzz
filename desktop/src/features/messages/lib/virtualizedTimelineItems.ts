/**
 * Pure item-assembly for the virtualized timeline row stream.
 *
 * Kept free of React rendering (only a type-level ReactNode) so the
 * exact-suffix `shift`-admission invariants are covered by the lib-level
 * `*.test.mjs` suite.
 */

import type * as React from "react";

import { estimateTimelineItemHeight } from "./rowHeightEstimate";
import {
  getTimelineItemKey,
  type TimelineDayGroup,
  type TimelineNonDayItem,
} from "./timelineItems";

export type VirtualizedTimelineItem =
  | {
      kind: "leading-content";
      content: React.ReactNode;
    }
  | { kind: "bottom-spacer" }
  | { kind: "day-divider"; key: string; headingTimestamp: number }
  | {
      kind: "timeline-item";
      item: TimelineNonDayItem;
    };

export function estimateVirtualizedTimelineItemHeight(
  item: VirtualizedTimelineItem,
): number {
  if (item.kind === "bottom-spacer") return 96;
  if (item.kind === "leading-content") return 60;
  if (item.kind === "day-divider") return 32;
  return estimateTimelineItemHeight(item.item);
}

export function virtualizedItemKey(item: VirtualizedTimelineItem): string {
  if (item.kind === "bottom-spacer") return "bottom-spacer";
  if (item.kind === "leading-content") return "leading-content";
  if (item.kind === "day-divider") return item.key;
  return getTimelineItemKey(item.item);
}

/**
 * Flattens day groups into the virtual row stream.
 *
 * Day dividers are standalone items emitted only at PROVEN day boundaries: a
 * boundary is proven when a strictly older loaded day precedes it in the
 * window, or when history is exhausted (the window provably starts at the
 * channel's beginning). The oldest loaded day gets no divider while more
 * history exists — its "start" is just the arbitrary edge of the loaded
 * window, and a divider item there would have to accept older same-day rows
 * prepending BEHIND it, which breaks the exact-suffix key admission that
 * Virtua's `shift` depends on. (The previous in-row divider avoided that key
 * problem but made the day's head row change shape mid-commit when the
 * divider migrated off it, causing a one-frame anchor jump at page merges.)
 * A proven boundary is immutable — any strictly older message belongs to a
 * strictly older day — so its divider only ever enters the stream as part of
 * a prepended prefix and never mutates the existing key suffix.
 */
export function buildVirtualizedItems(
  dayGroups: readonly TimelineDayGroup[],
  leadingContent: React.ReactNode | undefined,
  historyExhausted: boolean,
): VirtualizedTimelineItem[] {
  const timelineItems = dayGroups.flatMap((group, groupIndex) => {
    const boundaryProven = groupIndex > 0 || historyExhausted;
    const divider =
      group.headingTimestamp !== null && boundaryProven
        ? [
            {
              kind: "day-divider" as const,
              // Namespaced so a divider key can never collide with any
              // timeline item key (message ids, `unread-*`, sentinels).
              key: `day-divider:${group.key}`,
              headingTimestamp: group.headingTimestamp,
            },
          ]
        : [];
    return [
      ...divider,
      ...group.items.map((item) => ({
        kind: "timeline-item" as const,
        item,
      })),
    ];
  });

  return [
    ...(leadingContent
      ? [
          {
            kind: "leading-content" as const,
            content: leadingContent,
          },
        ]
      : []),
    ...timelineItems,
    { kind: "bottom-spacer" as const },
  ];
}

export function didPrependVirtualizedTimeline(
  previousKeys: readonly string[],
  keys: readonly string[],
): boolean {
  const prependedCount = keys.length - previousKeys.length;
  // Virtua shifts its positional size cache by the length delta, so enable
  // `shift` only when every previous slot is the exact suffix it expects.
  return (
    prependedCount > 0 &&
    previousKeys.every((key, index) => key === keys[index + prependedCount])
  );
}
