import { computeThreadUnreadMarker } from "@/features/messages/lib/unreadMarker";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Per-thread unread reply counts for the summary rows in the main timeline.
 *
 * Counts are computed only for threads the user has notification interest in
 * (`isNotified`). The count spans the root's WHOLE subtree, so a reply nested
 * under another reply still tallies toward the root's badge.
 *
 * Subtree membership is keyed on each reply's `rootId` rather than walked
 * through the parent chain: a reply whose intermediate ancestor is absent from
 * the loaded window still carries its true rootId (getThreadReference), so it
 * rolls up to the root the parent-chain walk could never reach. For an intact
 * chain every descendant carries the root's rootId, so the tally is identical
 * to the old adjacency walk. Each reply has exactly one rootId, so it is
 * counted once and a malformed parent cycle keys off no root.
 *
 * Unread is decided per-reply against `getReadAt` (LP4 v3): each reply lights
 * iff `createdAt > effective(msg:<id>)`, so reading one reply never clears
 * another and a collapsed-branch reply keeps its badge until revealed.
 *
 * @param messages Top-level timeline entries in chronological order.
 * @param repliesByRootId Replies grouped by their resolved thread root id.
 * @param getReadAt Per-message read resolver; `null` means never read.
 * @param isNotified Whether a thread root is one the user is notified for.
 * @param currentPubkey Replies authored by this pubkey never count as unread.
 * @param isForcedUnread Session-local OR-overlay: a reply forced unread this
 *   session counts regardless of its marker (per-message mark-unread).
 */
export function computeThreadBadgeCounts(
  messages: TimelineMessage[],
  repliesByRootId: ReadonlyMap<string, TimelineMessage[]>,
  getReadAt: (messageId: string) => number | null,
  isNotified: (rootId: string) => boolean,
  currentPubkey?: string,
  isForcedUnread: (messageId: string) => boolean = () => false,
): Map<string, number> {
  const counts = new Map<string, number>();
  for (const message of messages) {
    if (message.parentId) continue;
    if (!isNotified(message.id)) continue;
    const subtreeReplies = repliesByRootId.get(message.id);
    if (!subtreeReplies || subtreeReplies.length === 0) continue;
    const { unreadCount } = computeThreadUnreadMarker(
      subtreeReplies,
      getReadAt,
      currentPubkey,
      isForcedUnread,
    );
    if (unreadCount > 0) {
      counts.set(message.id, unreadCount);
    }
  }
  return counts;
}
