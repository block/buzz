import { getThreadReference } from "@/features/messages/lib/threading";

/**
 * When the open thread is in Starred threads, the starred row owns the active
 * highlight — the parent channel row below must not also look selected.
 */
export function shouldSuppressChannelActiveForStarredThread(input: {
  isChannelActive: boolean;
  selectedThreadRootId: string | null;
  isThreadStarred: (rootId: string) => boolean;
}): boolean {
  if (!input.isChannelActive) return false;
  const rootId = input.selectedThreadRootId?.trim();
  if (!rootId) return false;
  return input.isThreadStarred(rootId);
}

export function formatSidebarUnreadCount(count: number): string {
  return count > 99 ? "99+" : String(count);
}

/**
 * Conversation id for a thread-feed item, matching inbox derivation for
 * ordinary channel messages (root → parent → event id). Avoids importing the
 * heavy home/inbox module into channel-row code paths.
 */
export function threadFeedConversationId(item: {
  id: string;
  tags: string[][];
}): string {
  const thread = getThreadReference(item.tags);
  return thread.rootId ?? thread.parentId ?? item.id;
}

/** Count unread thread-feed items that belong to a starred thread root. */
export function countUnreadForStarredThreadRoot(
  unreadThreadFeedItems: readonly { id: string; tags: string[][] }[],
  rootId: string,
): number {
  const trimmed = rootId.trim();
  if (!trimmed) return 0;
  let count = 0;
  for (const item of unreadThreadFeedItems) {
    if (threadFeedConversationId(item) === trimmed) count += 1;
  }
  return count;
}
