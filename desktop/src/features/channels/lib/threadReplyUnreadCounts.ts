import { buildDescendantStatsByMessageId } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Per-row subtree unread counts for the in-panel thread summary rows. A
 * collapsed branch's badge counts unread replies anywhere beneath it; the
 * count is omitted for expanded branches (suppress-on-expand happens here,
 * upstream of the panel, so the panel needs no gate) and for rows with zero
 * unread descendants (no "0" badge).
 *
 * Unread is measured against the open-time frontier snapshot — the same
 * boundary the in-thread divider uses — so the mark-read-on-open advance does
 * not zero the badges the instant the panel opens. A null frontier (thread
 * never read) treats every subtree reply as unread.
 *
 * @param subtreeReplyIds Descendant reply ids of the open thread head. Scoping
 *   the unread set to this subtree keeps one thread's frontier from marking
 *   replies that belong to a different thread.
 * @param visibleReplyIds Ids of the rows actually rendered in the panel; only
 *   these are keyed, keeping the map consistent with row presence.
 * @param expandedSubtreeReplyIds Reply ids beneath any expanded row. Expanding
 *   a branch persistently marks its whole subtree read (mark-read-on-expand),
 *   so those replies are dropped from the unread set — otherwise a revealed
 *   child would carry a stale badge for a reply the same gesture just read.
 */
export function computeThreadReplyUnreadCounts(params: {
  timelineMessages: TimelineMessage[];
  subtreeReplyIds: Iterable<string>;
  visibleReplyIds: Iterable<string>;
  expandedReplyIds: ReadonlySet<string>;
  expandedSubtreeReplyIds: ReadonlySet<string>;
  frontierSeconds: number | null;
}): Map<string, number> {
  const {
    timelineMessages,
    subtreeReplyIds,
    visibleReplyIds,
    expandedReplyIds,
    expandedSubtreeReplyIds,
    frontierSeconds,
  } = params;

  const subtree = new Set(subtreeReplyIds);
  const unreadReplyIds = new Set(
    timelineMessages
      .filter(
        (message) =>
          subtree.has(message.id) &&
          !expandedSubtreeReplyIds.has(message.id) &&
          (frontierSeconds === null || message.createdAt > frontierSeconds),
      )
      .map((message) => message.id),
  );

  const stats = buildDescendantStatsByMessageId(
    timelineMessages,
    unreadReplyIds,
  );

  const counts = new Map<string, number>();
  for (const replyId of visibleReplyIds) {
    if (expandedReplyIds.has(replyId)) continue;
    const unread = stats.get(replyId)?.unreadDescendantCount ?? 0;
    if (unread > 0) counts.set(replyId, unread);
  }
  return counts;
}
