import * as React from "react";

import type { InboxItem } from "@/features/home/lib/inbox";

type UseHomeInboxReadStateOptions = {
  /** Inbox items to project read-state across. */
  items: InboxItem[];
  /** NIP-RS read marker resolver for channel-backed items (unix seconds, or null when unknown). */
  getChannelReadAt: (channelId: string) => number | null;
  /** Invalidation signal for the channel-marker projection. */
  readStateVersion: number;
  /** Local fallback "done" set (used only for items with no channelId). */
  localDoneSet: ReadonlySet<string>;
  /** Mark a channel read up to the given ISO timestamp (NIP-RS). */
  markChannelRead: (
    channelId: string,
    readAt: string | null | undefined,
  ) => void;
  /** Mark a channel unread locally for the current session. */
  markChannelUnread: (channelId: string) => void;
  /** Local fallback: mark a non-channel item done. */
  markDoneLocal: (id: string) => void;
  /** Local fallback: undo a non-channel item done. */
  undoDoneLocal: (id: string) => void;
};

/**
 * Projects Home inbox read-state from the shared NIP-RS read marker, with
 * the local `useFeedItemState` done-set as a fallback for items that don't
 * belong to a channel (reminders etc.).
 *
 * "Mark as read/unread" actions on channel-backed items are routed through
 * `markChannelRead`/`markChannelUnread` so the sidebar, home badge, and any
 * other surfaces consuming the same ReadStateManager stay in lockstep.
 * Caveat: NIP-RS channel read markers are monotonic, so marking an older item
 * unread is an in-session local affordance rather than synced state.
 */
export function useHomeInboxReadState({
  items,
  getChannelReadAt,
  readStateVersion,
  localDoneSet,
  markChannelRead,
  markChannelUnread,
  markDoneLocal,
  undoDoneLocal,
}: UseHomeInboxReadStateOptions) {
  const itemById = React.useMemo(
    () => new Map(items.map((item) => [item.id, item])),
    [items],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion invalidates getChannelReadAt
  const effectiveDoneSet = React.useMemo<ReadonlySet<string>>(() => {
    const result = new Set<string>();
    for (const item of items) {
      const channelId = item.item.channelId;
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
  }, [getChannelReadAt, items, localDoneSet, readStateVersion]);

  const markItemRead = React.useCallback(
    (itemId: string) => {
      const item = itemById.get(itemId);
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
    [itemById, markChannelRead, markDoneLocal],
  );

  const markItemUnread = React.useCallback(
    (itemId: string) => {
      const item = itemById.get(itemId);
      const channelId = item?.item.channelId ?? null;
      if (item && channelId) {
        markChannelUnread(channelId);
        return;
      }
      undoDoneLocal(itemId);
    },
    [itemById, markChannelUnread, undoDoneLocal],
  );

  return { effectiveDoneSet, markItemRead, markItemUnread };
}
