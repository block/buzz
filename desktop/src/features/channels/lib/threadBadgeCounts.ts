import { computeThreadUnreadMarker } from "@/features/messages/lib/unreadMarker";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Per-thread unread reply counts for the summary rows in the main timeline.
 *
 * Counts are computed only for threads the user has notification interest in
 * (`isNotified`) and measured against the per-root frontier snapshot rather
 * than the live marker, so badges stay stable for the session (see
 * nextThreadBadgeFrontier for the snapshot-advance-on-read rationale). The
 * count spans the root's WHOLE subtree, so a reply nested under another reply
 * still tallies toward the root's badge.
 *
 * Subtree membership is keyed on each reply's `rootId` rather than walked
 * through the parent chain: a reply whose intermediate ancestor is absent from
 * the loaded window still carries its true rootId (getThreadReference), so it
 * rolls up to the root the parent-chain walk could never reach. For an intact
 * chain every descendant carries the root's rootId, so the tally is identical
 * to the old adjacency walk. Each reply has exactly one rootId, so it is
 * counted once and a malformed parent cycle keys off no root.
 *
 * @param messages Top-level timeline entries in chronological order.
 * @param repliesByRootId Replies grouped by their resolved thread root id.
 * @param frontiers Per-root read frontier in unix seconds, or null/undefined
 *   when the thread was never read (every reply counts unread).
 * @param isNotified Whether a thread root is one the user is notified for.
 * @param currentPubkey Replies authored by this pubkey never count as unread.
 */
export function computeThreadBadgeCounts(
  messages: TimelineMessage[],
  repliesByRootId: ReadonlyMap<string, TimelineMessage[]>,
  frontiers: ReadonlyMap<string, number | null> | undefined,
  isNotified: (rootId: string) => boolean,
  currentPubkey?: string,
): Map<string, number> {
  const counts = new Map<string, number>();
  for (const message of messages) {
    if (message.parentId) continue;
    if (!isNotified(message.id)) continue;
    const subtreeReplies = repliesByRootId.get(message.id);
    if (!subtreeReplies || subtreeReplies.length === 0) continue;
    const { unreadCount } = computeThreadUnreadMarker(
      subtreeReplies,
      frontiers?.get(message.id) ?? null,
      currentPubkey,
    );
    if (unreadCount > 0) {
      counts.set(message.id, unreadCount);
    }
  }
  return counts;
}
