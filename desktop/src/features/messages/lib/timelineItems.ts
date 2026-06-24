/**
 * Flattens the heterogeneous day-grouped timeline tree into a flat
 * discriminated-union item stream that a virtualizer can window over, and
 * builds the `messageId -> itemIndex` map every DOM-query scroll path now
 * resolves against instead of `querySelector`.
 *
 * Kept pure (no React, no DOM) so it is covered by the lib-level `*.test.mjs`
 * suite. The list and the index map are produced together from the SAME walk,
 * so they can never drift: a stale map would scroll deep-links to the wrong
 * row, the exact failure virtualization risks.
 */

import {
  buildDayGroupBoundaries,
  type DayGroupBoundary,
} from "@/features/messages/lib/timelineSnapshot";
import { shouldRenderUnreadDivider } from "@/features/messages/lib/threadPanel";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

/**
 * One renderable row in the flattened timeline. Dividers carry no message and
 * never appear in the index map; the three message-bearing kinds do.
 */
export type TimelineItem =
  // `headingTimestamp` (not a prebaked label) so the render still resolves
  // "Today"/"Yesterday" relative to the current clock, not to build time.
  | { kind: "day-divider"; key: string; headingTimestamp: number }
  | { kind: "unread-divider"; key: string }
  | { kind: "system"; key: string; entry: MainTimelineEntry }
  | { kind: "message"; key: string; entry: MainTimelineEntry };

export type TimelineItemsResult = {
  items: TimelineItem[];
  /** Maps a top-level message id to its index in `items`. */
  indexByMessageId: Map<string, number>;
};

/** Stable per-item key, unique across the flattened stream. */
export function getTimelineItemKey(item: TimelineItem): string {
  return item.key;
}

/**
 * The `headingTimestamp` of the day the topmost visible row belongs to: the
 * nearest day-divider at or above `topVisibleIndex`. The floating active-day
 * header reads this so it always shows the current day even when that day's
 * in-stream divider has scrolled out of the virtual window. Returns `null`
 * before any row is in view or when no divider precedes the visible row (an
 * empty stream).
 */
export function resolveActiveDayTimestamp(
  items: TimelineItem[],
  topVisibleIndex: number | null,
): number | null {
  if (topVisibleIndex === null) {
    return null;
  }
  for (let i = Math.min(topVisibleIndex, items.length - 1); i >= 0; i--) {
    const item = items[i];
    if (item.kind === "day-divider") {
      return item.headingTimestamp;
    }
  }
  return null;
}

function entryRenderKey(entry: MainTimelineEntry): string {
  return entry.message.renderKey ?? entry.message.id;
}

/**
 * Walks the (already top-level-filtered) entries once, emitting a day-divider
 * at each calendar-day boundary and an unread-divider above the first unread
 * message, then the message/system row itself. The index map records where
 * each message landed in the flat stream so scroll targets resolve in O(1)
 * without touching the DOM.
 */
export function buildTimelineItems(
  entries: MainTimelineEntry[],
  firstUnreadMessageId: string | null,
): TimelineItemsResult {
  const items: TimelineItem[] = [];
  const indexByMessageId = new Map<string, number>();

  // Index boundaries by their start position so the walk below can look up the
  // prepend-stable section key (start-of-local-day). Keying the divider by
  // start-of-day, not by the first message, keeps the day section from
  // remounting when older messages prepend into it.
  const dayBoundariesByStartIndex = new Map(
    buildDayGroupBoundaries(entries.map((entry) => entry.message)).map(
      (boundary: DayGroupBoundary) => [boundary.startIndex, boundary] as const,
    ),
  );

  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    const { message } = entry;
    const renderKey = entryRenderKey(entry);

    const dayBoundary = dayBoundariesByStartIndex.get(i);
    if (dayBoundary) {
      items.push({
        kind: "day-divider",
        key: dayBoundary.key,
        headingTimestamp: message.createdAt,
      });
    }

    if (shouldRenderUnreadDivider(i, message.id, firstUnreadMessageId)) {
      items.push({ kind: "unread-divider", key: `unread-${renderKey}` });
    }

    const kind = message.kind === KIND_SYSTEM_MESSAGE ? "system" : "message";
    indexByMessageId.set(message.id, items.length);
    items.push({ kind, key: renderKey, entry });
  }

  return { items, indexByMessageId };
}
