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
import {
  hasSameMessageAuthor,
  isWithinGroupingWindow,
} from "@/features/messages/lib/messageGrouping";
import { KIND_SYSTEM_MESSAGE } from "@/shared/constants/kinds";

/**
 * One renderable row in the flattened timeline. Dividers carry no message and
 * never appear in the index map; the message-bearing kinds do.
 */
export type TimelineItem =
  // `headingTimestamp` (not a prebaked label) so the render still resolves
  // "Today"/"Yesterday" relative to the current clock, not to build time.
  | { kind: "day-divider"; key: string; headingTimestamp: number }
  | { kind: "unread-divider"; key: string }
  | { kind: "system"; key: string; entry: MainTimelineEntry }
  | {
      kind: "system-group";
      key: string;
      entries: MainTimelineEntry[];
    }
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

const MEMBERSHIP_GROUP_WINDOW_SECONDS = 5 * 60;

type MembershipChangePayload = {
  actor: string | null;
  mode: "added" | "joined";
  target: string;
};

function parseMembershipChangePayload(
  entry: MainTimelineEntry,
): MembershipChangePayload | null {
  if (entry.message.kind !== KIND_SYSTEM_MESSAGE) return null;

  try {
    const payload = JSON.parse(entry.message.body) as {
      type?: unknown;
      actor?: unknown;
      target?: unknown;
    };
    if (
      payload.type !== "member_joined" ||
      typeof payload.actor !== "string" ||
      typeof payload.target !== "string"
    ) {
      return null;
    }

    const actor = payload.actor.trim().toLowerCase();
    const target = payload.target.trim().toLowerCase();
    if (!actor || !target) return null;

    return actor === target
      ? { actor: null, mode: "joined", target }
      : { actor, mode: "added", target };
  } catch {
    return null;
  }
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
  let previousMembershipItemIndex: number | null = null;

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
      previousMembershipItemIndex = null;
      items.push({
        kind: "day-divider",
        key: dayBoundary.key,
        headingTimestamp: message.createdAt,
      });
    }

    if (shouldRenderUnreadDivider(i, message.id, firstUnreadMessageId)) {
      previousGroupEntry = null;
      previousMessageItemIndex = null;
      previousMembershipItemIndex = null;
      items.push({ kind: "unread-divider", key: `unread-${renderKey}` });
    }

    const kind = message.kind === KIND_SYSTEM_MESSAGE ? "system" : "message";
    if (kind === "system") {
      previousGroupEntry = null;
      previousMessageItemIndex = null;

      const membershipChange = parseMembershipChangePayload(entry);
      const previousItem =
        previousMembershipItemIndex === null
          ? null
          : items[previousMembershipItemIndex];
      const previousEntries =
        previousItem?.kind === "system-group"
          ? previousItem.entries
          : previousItem?.kind === "system"
            ? [previousItem.entry]
            : [];
      const firstPreviousEntry = previousEntries[0];
      const firstPreviousPayload = firstPreviousEntry
        ? parseMembershipChangePayload(firstPreviousEntry)
        : null;

      if (
        membershipChange &&
        firstPreviousEntry &&
        firstPreviousPayload?.mode === membershipChange.mode &&
        (membershipChange.mode === "joined" ||
          firstPreviousPayload.actor === membershipChange.actor) &&
        message.createdAt >= firstPreviousEntry.message.createdAt &&
        message.createdAt - firstPreviousEntry.message.createdAt <=
          MEMBERSHIP_GROUP_WINDOW_SECONDS
      ) {
        const groupIndex = previousMembershipItemIndex as number;
        items[groupIndex] = {
          kind: "system-group",
          key: entryRenderKey(firstPreviousEntry),
          entries: [...previousEntries, entry],
        };
        continue;
      }

      items.push({ kind, key: renderKey, entry });
      previousMembershipItemIndex = membershipChange ? items.length - 1 : null;
      continue;
    }

    previousMembershipItemIndex = null;

    const isContinuation =
      previousGroupEntry !== null &&
      hasSameMessageAuthor(previousGroupEntry.message, message) &&
      isWithinGroupingWindow(
        previousGroupEntry.message.createdAt,
        message.createdAt,
      );

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
