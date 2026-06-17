import { computeThreadUnreadMarker } from "@/features/messages/lib/unreadMarker";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Per-thread unread reply counts for the summary rows in the main timeline.
 *
 * Counts are computed only for threads the user has notification interest in
 * (`isNotified`), aligning the badge display with the read-state write path,
 * and measured against the per-root frontier snapshot rather than the live
 * marker so badges stay stable for the session and don't flash when
 * markChannelRead advances the channel marker. The snapshot is advanced toward
 * the live marker on read upstream of this function, so a read thread's badge
 * clears here once its frontier passes the last reply.
 *
 * @param messages Timeline messages (top-level entries plus their replies) in
 *   chronological order.
 * @param frontiers Per-root read frontier in unix seconds, or null/undefined
 *   when the thread was never read (every reply counts unread).
 * @param isNotified Whether a thread root is one the user is notified for.
 * @param currentPubkey Replies authored by this pubkey never count as unread.
 */
export function computeThreadBadgeCounts(
  messages: TimelineMessage[],
  frontiers: ReadonlyMap<string, number | null> | undefined,
  isNotified: (rootId: string) => boolean,
  currentPubkey?: string,
): Map<string, number> {
  const counts = new Map<string, number>();
  for (const message of messages) {
    if (message.parentId) continue;
    if (!isNotified(message.id)) continue;
    const directReplies = messages.filter((m) => m.parentId === message.id);
    if (directReplies.length === 0) continue;
    const { unreadCount } = computeThreadUnreadMarker(
      directReplies,
      frontiers?.get(message.id) ?? null,
      currentPubkey,
    );
    if (unreadCount > 0) {
      counts.set(message.id, unreadCount);
    }
  }
  return counts;
}
