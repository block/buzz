import { computeThreadUnreadMarker } from "@/features/messages/lib/unreadMarker";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * All reply messages in a root's subtree — direct children plus every deeper
 * descendant, walked through the direct-replies adjacency map. A reply-to-a-
 * reply must count toward the root's badge, so the badge tally needs the whole
 * subtree rather than the root's direct children alone.
 *
 * Terminates without a visited-set: buildDirectRepliesByParentId places each
 * message under exactly one parent key, and the only caller seeds the walk from
 * true roots (parentId === null), so a node in a malformed parent cycle — whose
 * members all key off each other, never off a root — is unreachable from any
 * root's bucket. Seeding from a non-root id, or a builder that filed one node
 * under two keys, would break that invariant.
 */
function collectSubtreeReplies(
  rootId: string,
  directRepliesByParentId: ReadonlyMap<string, TimelineMessage[]>,
): TimelineMessage[] {
  const replies: TimelineMessage[] = [];
  const pending = [...(directRepliesByParentId.get(rootId) ?? [])];
  while (pending.length > 0) {
    const reply = pending.pop();
    if (!reply) continue;
    replies.push(reply);
    pending.push(...(directRepliesByParentId.get(reply.id) ?? []));
  }
  return replies;
}

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
 * @param messages Top-level timeline entries in chronological order.
 * @param directRepliesByParentId Direct replies keyed by parent id, walked to
 *   collect each root's full descendant subtree.
 * @param frontiers Per-root read frontier in unix seconds, or null/undefined
 *   when the thread was never read (every reply counts unread).
 * @param isNotified Whether a thread root is one the user is notified for.
 * @param currentPubkey Replies authored by this pubkey never count as unread.
 */
export function computeThreadBadgeCounts(
  messages: TimelineMessage[],
  directRepliesByParentId: ReadonlyMap<string, TimelineMessage[]>,
  frontiers: ReadonlyMap<string, number | null> | undefined,
  isNotified: (rootId: string) => boolean,
  currentPubkey?: string,
): Map<string, number> {
  const counts = new Map<string, number>();
  for (const message of messages) {
    if (message.parentId) continue;
    if (!isNotified(message.id)) continue;
    const subtreeReplies = collectSubtreeReplies(
      message.id,
      directRepliesByParentId,
    );
    if (subtreeReplies.length === 0) continue;
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
