import type { TimelineMessage } from "@/features/messages/types";

/**
 * Target for the floating "N new replies in threads" pill.
 *
 * The channel-level unread pill counts top-level messages only
 * (`computeChannelUnreadMarker` skips replies by design), so a thread reply
 * bolds the channel in the sidebar while leaving the scrollback with no
 * visible cue. This computes the complementary signal: the total unread
 * thread replies across the loaded window and the oldest thread root that
 * holds one, so the pill can jump the user to the parent row (which carries
 * the lit badge and accent pointing into the thread).
 */
export type ThreadUnreadPillTarget = {
  /** Sum of unread replies across thread roots in the loaded window. */
  totalUnreadReplies: number;
  /** Oldest loaded thread root with unread replies, or null if none. */
  oldestParentId: string | null;
};

const EMPTY_TARGET: ThreadUnreadPillTarget = {
  totalUnreadReplies: 0,
  oldestParentId: null,
};

/**
 * @param messages Loaded timeline messages in chronological order — the pill
 *   may only target rows that are actually mounted, so the sum is scoped to
 *   the loaded window rather than the full per-channel badge counts.
 * @param threadUnreadCounts Per-thread unread counts keyed by thread root id
 *   (see `computeThreadBadgeCounts`).
 */
export function computeThreadUnreadPillTarget(
  messages: ReadonlyArray<Pick<TimelineMessage, "id">>,
  threadUnreadCounts: ReadonlyMap<string, number> | undefined,
): ThreadUnreadPillTarget {
  if (!threadUnreadCounts || threadUnreadCounts.size === 0) {
    return EMPTY_TARGET;
  }
  let totalUnreadReplies = 0;
  let oldestParentId: string | null = null;
  for (const message of messages) {
    const count = threadUnreadCounts.get(message.id) ?? 0;
    if (count <= 0) continue;
    totalUnreadReplies += count;
    if (oldestParentId === null) {
      oldestParentId = message.id;
    }
  }
  return { totalUnreadReplies, oldestParentId };
}

export function threadUnreadPillLabel(count: number) {
  return `${count} new ${count === 1 ? "reply" : "replies"} in threads`;
}
