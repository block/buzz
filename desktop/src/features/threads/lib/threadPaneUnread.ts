import {
  buildDirectReplyIdsByParentId,
  collectReplyDescendantIds,
} from "@/features/channels/lib/subtreeCreatedAt";

type UnreadFeedReply = { id: string; rootId: string | null };
type PaneMessage = { id: string; parentId?: string | null; createdAt: number };

/** Ids of the unread replies the app tracks for one thread. */
export function unreadReplyIdsForRoot(
  items: readonly UnreadFeedReply[],
  rootId: string | null,
): ReadonlySet<string> {
  const ids = new Set<string>();
  if (!rootId) return ids;
  for (const item of items) {
    if (item.rootId === rootId && item.id !== rootId) ids.add(item.id);
  }
  return ids;
}

/**
 * Where the pane's "New" divider goes: the first visible reply that was unread
 * when the thread opened, skipping your own replies as the channel does.
 */
export function firstUnreadReplyId(
  replies: readonly { id: string; pubkey?: string }[],
  unreadAtOpen: ReadonlySet<string>,
  currentPubkey: string | undefined,
): string | null {
  const self = currentPubkey?.toLowerCase();
  for (const reply of replies) {
    if (unreadAtOpen.has(reply.id) && reply.pubkey?.toLowerCase() !== self) {
      return reply.id;
    }
  }
  return null;
}

/** Unread replies beneath each message, for the collapsed-row badges. */
export function unreadDescendantCounts(
  messages: readonly PaneMessage[],
  unreadIds: ReadonlySet<string>,
): ReadonlyMap<string, number> {
  const counts = new Map<string, number>();
  if (unreadIds.size === 0) return counts;
  const repliesByParent = buildDirectReplyIdsByParentId(messages);
  for (const message of messages) {
    let count = 0;
    for (const id of collectReplyDescendantIds(message.id, repliesByParent)) {
      if (unreadIds.has(id)) count += 1;
    }
    if (count > 0) counts.set(message.id, count);
  }
  return counts;
}

/**
 * The replies a "Mark read" or "Mark unread" on `messageId` covers: its whole
 * subtree, plus the message itself unless it is the root (which is not a reply).
 */
export function subtreeReplies<T extends PaneMessage>(
  messageId: string,
  rootId: string | null,
  messages: readonly T[],
): T[] {
  const ids = new Set(
    collectReplyDescendantIds(
      messageId,
      buildDirectReplyIdsByParentId(messages),
    ),
  );
  if (messageId !== rootId) ids.add(messageId);
  return messages.filter((message) => ids.has(message.id));
}
