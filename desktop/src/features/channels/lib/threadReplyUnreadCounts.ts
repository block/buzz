import { buildDescendantStatsByMessageId } from "@/features/messages/lib/threadPanel";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Per-row subtree unread counts for the in-panel thread summary rows. A
 * collapsed branch's badge counts unread replies anywhere beneath it; the
 * count is omitted for expanded branches (their children render inline, so no
 * summary badge) and for rows with zero unread descendants (no "0" badge).
 *
 * Unread is decided per-reply against `getReadAt` (LP4 v3): each reply counts
 * iff `createdAt > effective(msg:<id>)`. Expanding a branch marks only the
 * revealed (direct-child) set read, so a collapsed grandchild keeps its badge
 * until it too is revealed — no separate expanded-subtree gate is needed,
 * because the per-message marker already distinguishes a read parent from its
 * still-unread descendant. A `null` marker (reply never read) counts as unread.
 *
 * @param subtreeReplyIds Descendant reply ids of the open thread head. Scoping
 *   the unread set to this subtree keeps replies in a different thread from
 *   ever being counted here.
 * @param visibleReplyIds Ids of the rows actually rendered in the panel; only
 *   these are keyed, keeping the map consistent with row presence.
 * @param expandedReplyIds Ids of rows whose children are rendered inline; these
 *   rows carry no summary badge.
 * @param getReadAt Per-message read resolver; `null` means never read.
 * @param isForcedUnread Session-local OR-overlay: a reply forced unread this
 *   session counts regardless of its marker (per-message mark-unread).
 */
export function computeThreadReplyUnreadCounts(params: {
  timelineMessages: TimelineMessage[];
  subtreeReplyIds: Iterable<string>;
  visibleReplyIds: Iterable<string>;
  expandedReplyIds: ReadonlySet<string>;
  getReadAt: (messageId: string) => number | null;
  currentPubkey?: string;
  isForcedUnread?: (messageId: string) => boolean;
}): Map<string, number> {
  const {
    timelineMessages,
    subtreeReplyIds,
    visibleReplyIds,
    expandedReplyIds,
    getReadAt,
    currentPubkey,
    isForcedUnread = () => false,
  } = params;

  const subtree = new Set(subtreeReplyIds);
  const unreadReplyIds = new Set(
    timelineMessages
      .filter((message) => {
        if (!subtree.has(message.id)) return false;
        if (currentPubkey && message.pubkey === currentPubkey) return false;
        if (isForcedUnread(message.id)) return true;
        const readAt = getReadAt(message.id);
        return readAt === null || message.createdAt > readAt;
      })
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
