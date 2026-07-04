/**
 * Flattens the heterogeneous day-grouped timeline tree into a flat
 * discriminated-union item stream the list renders one row per entry.
 *
 * Kept pure (no React, no DOM) so it is covered by the lib-level `*.test.mjs`
 * suite.
 */

import {
  buildDayGroupBoundaries,
  type DayGroupBoundary,
} from "@/features/messages/lib/timelineSnapshot";
import { shouldRenderUnreadDivider } from "@/features/messages/lib/threadPanel";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { hasSameMessageAuthor } from "@/features/messages/lib/messageGrouping";
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
  | {
      kind: "message";
      key: string;
      entry: MainTimelineEntry;
      isContinuation: boolean;
      isFollowedByContinuation: boolean;
    };

export type TimelineItemsResult = {
  items: TimelineItem[];
};

export type TimelineNonDayItem = Exclude<TimelineItem, { kind: "day-divider" }>;

export type TimelineDayGroup = {
  key: string;
  headingTimestamp: number | null;
  items: TimelineNonDayItem[];
};

/** Stable per-item key, unique across the flattened stream. */
export function getTimelineItemKey(item: TimelineItem): string {
  return item.key;
}

function entryRenderKey(entry: MainTimelineEntry): string {
  return entry.message.renderKey ?? entry.message.id;
}

/**
 * Walks the (already top-level-filtered) entries once, emitting a day-divider
 * at each calendar-day boundary and an unread-divider above the first unread
 * message, then the message/system row itself.
 */
export function buildTimelineItems(
  entries: MainTimelineEntry[],
  firstUnreadMessageId: string | null,
): TimelineItemsResult {
  const items: TimelineItem[] = [];
  let previousGroupEntry: MainTimelineEntry | null = null;
  let previousMessageItemIndex: number | null = null;

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
      previousGroupEntry = null;
      previousMessageItemIndex = null;
      items.push({
        kind: "day-divider",
        key: dayBoundary.key,
        headingTimestamp: message.createdAt,
      });
    }

    if (shouldRenderUnreadDivider(i, message.id, firstUnreadMessageId)) {
      previousGroupEntry = null;
      previousMessageItemIndex = null;
      items.push({ kind: "unread-divider", key: `unread-${renderKey}` });
    }

    const kind = message.kind === KIND_SYSTEM_MESSAGE ? "system" : "message";
    if (kind === "system") {
      previousGroupEntry = null;
      previousMessageItemIndex = null;
      items.push({ kind, key: renderKey, entry });
      continue;
    }

    const isContinuation =
      previousGroupEntry !== null &&
      hasSameMessageAuthor(previousGroupEntry.message, message);

    if (isContinuation && previousMessageItemIndex !== null) {
      const previousItem = items[previousMessageItemIndex];
      if (previousItem?.kind === "message") {
        previousItem.isFollowedByContinuation = true;
      }
    }

    previousMessageItemIndex = items.length;
    items.push({
      kind,
      key: renderKey,
      entry,
      isContinuation,
      isFollowedByContinuation: false,
    });
    previousGroupEntry = entry;
  }

  return { items };
}

export function buildTimelineDayGroups(
  items: readonly TimelineItem[],
): TimelineDayGroup[] {
  const groups: TimelineDayGroup[] = [];
  let currentGroup: TimelineDayGroup | null = null;

  for (const item of items) {
    if (item.kind === "day-divider") {
      currentGroup = {
        key: item.key,
        headingTimestamp: item.headingTimestamp,
        items: [],
      };
      groups.push(currentGroup);
      continue;
    }

    if (!currentGroup) {
      currentGroup = {
        key: "day-undated",
        headingTimestamp: null,
        items: [],
      };
      groups.push(currentGroup);
    }

    currentGroup.items.push(item);
  }

  return groups;
}
