import * as React from "react";

import type { InboxItem } from "@/features/home/lib/inbox";
import {
  getThreadReference,
  isThreadReply,
} from "@/features/messages/lib/threading";

type UseHomeInboxReadStateOptions = {
  /** Inbox items to project read-state across. */
  items: InboxItem[];
  /** NIP-RS read marker resolver for channel-backed items (unix seconds, or null when unknown). */
  getChannelReadAt: (channelId: string) => number | null;
  /** NIP-RS read marker resolver for thread-backed items (unix seconds, or null when unknown). */
  getThreadReadAt: (rootId: string, channelId?: string | null) => number | null;
  /** Invalidation signal for the channel-marker projection. */
  readStateVersion: number;
  /** Local fallback "done" set (used only for items with no channelId). */
  localDoneSet: ReadonlySet<string>;
  /** Per-item local unread override for inbox rows. */
  localUnreadSet: ReadonlySet<string>;
  /** Mark a channel read up to the given ISO timestamp (NIP-RS). */
  markChannelRead: (
    channelId: string,
    readAt: string | null | undefined,
  ) => void;
  /** Advance the thread read marker to the given unix-seconds timestamp. */
  markThreadRead: (rootId: string, timestamp: number) => void;
  /** Local fallback: mark a non-channel item done. */
  markDoneLocal: (id: string) => void;
  /** Local inbox row override: mark an item unread without touching the channel. */
  markUnreadLocal: (id: string) => void;
  /** Local fallback: undo a non-channel item done. */
  undoDoneLocal: (id: string) => void;
  /** Clear the local inbox row unread override. */
  undoUnreadLocal: (id: string) => void;
};

const EMPTY_ITEM_SET: ReadonlySet<string> = new Set();

function getInboxThreadRootId(item: InboxItem): string | null {
  if (!isThreadReply(item.item.tags)) {
    return null;
  }

  return getThreadReference(item.item.tags).rootId;
}

export function getGroupedChannelReadTimestamp(
  item: InboxItem,
): { channelId: string; timestamp: number } | null {
  const channelId = item.item.channelId ?? null;
  if (!channelId) {
    return null;
  }

  let timestamp: number | null = null;
  for (const groupItem of item.groupItems) {
    if (groupItem.channelId !== channelId || isThreadReply(groupItem.tags)) {
      continue;
    }
    timestamp = Math.max(timestamp ?? 0, groupItem.createdAt);
  }

  return timestamp === null ? null : { channelId, timestamp };
}

export function getGroupedInboxItemIds(item: InboxItem): string[] {
  return [
    ...new Set([item.id, item.item.id, ...item.groupItems.map((i) => i.id)]),
  ];
}

export function hasGroupedUnreadOverride(
  item: InboxItem,
  localUnreadSet: ReadonlySet<string>,
): boolean {
  return getGroupedInboxItemIds(item).some((id) => localUnreadSet.has(id));
}

/**
 * Projects Home inbox read-state from the shared NIP-RS read marker, with
 * the local `useFeedItemState` done-set as a fallback for items that don't
 * belong to a channel (reminders etc.).
 *
 * "Mark as read" on channel-backed items is routed through `markChannelRead`;
 * thread rows use their own `thread:<root>` marker so they do not affect the
 * sidebar channel dot. "Mark unread" is item-local: it only reopens the
 * specific inbox row and must not light up the channel.
 */
export function useHomeInboxReadState({
  items,
  getChannelReadAt,
  getThreadReadAt,
  readStateVersion,
  localDoneSet,
  localUnreadSet = EMPTY_ITEM_SET,
  markChannelRead,
  markThreadRead,
  markDoneLocal,
  markUnreadLocal,
  undoDoneLocal,
  undoUnreadLocal,
}: UseHomeInboxReadStateOptions) {
  const itemById = React.useMemo(
    () => new Map(items.map((item) => [item.id, item])),
    [items],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion invalidates getChannelReadAt
  const effectiveDoneSet = React.useMemo<ReadonlySet<string>>(() => {
    const result = new Set<string>();
    for (const item of items) {
      if (hasGroupedUnreadOverride(item, localUnreadSet)) {
        continue;
      }

      const channelId = item.item.channelId;
      const threadRootId = getInboxThreadRootId(item);
      if (threadRootId) {
        const readAt = getThreadReadAt(threadRootId, channelId);
        if (readAt !== null && item.latestActivityAt <= readAt) {
          result.add(item.id);
        }
        continue;
      }

      if (channelId) {
        const readAt = getChannelReadAt(channelId);
        if (readAt !== null && item.latestActivityAt <= readAt) {
          result.add(item.id);
        }
        continue;
      }
      if (localDoneSet.has(item.id)) {
        result.add(item.id);
      }
    }
    return result;
  }, [
    getChannelReadAt,
    getThreadReadAt,
    items,
    localDoneSet,
    localUnreadSet,
    readStateVersion,
  ]);

  const markItemRead = React.useCallback(
    (itemId: string) => {
      const item = itemById.get(itemId);
      const localUnreadIds = item ? getGroupedInboxItemIds(item) : [itemId];
      for (const id of localUnreadIds) {
        undoUnreadLocal(id);
      }
      const threadRootId = item ? getInboxThreadRootId(item) : null;
      if (item && threadRootId) {
        markThreadRead(threadRootId, item.latestActivityAt);
        const groupedChannelRead = getGroupedChannelReadTimestamp(item);
        if (groupedChannelRead) {
          markChannelRead(
            groupedChannelRead.channelId,
            new Date(groupedChannelRead.timestamp * 1_000).toISOString(),
          );
        }
        return;
      }

      const channelId = item?.item.channelId ?? null;
      if (item && channelId) {
        markChannelRead(
          channelId,
          new Date(item.latestActivityAt * 1_000).toISOString(),
        );
        return;
      }
      markDoneLocal(itemId);
    },
    [itemById, markChannelRead, markDoneLocal, markThreadRead, undoUnreadLocal],
  );

  const markItemUnread = React.useCallback(
    (itemId: string) => {
      undoDoneLocal(itemId);
      markUnreadLocal(itemId);
    },
    [markUnreadLocal, undoDoneLocal],
  );

  return { effectiveDoneSet, markItemRead, markItemUnread };
}
