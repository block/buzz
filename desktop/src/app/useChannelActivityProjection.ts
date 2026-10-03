import * as React from "react";

import { useThreadActivityFeedItems } from "@/app/useThreadActivityFeedItems";
import {
  maxReadAt,
  msgContextKey,
} from "@/features/channels/readState/readStateFormat";
import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";
import {
  getThreadReference,
  isThreadReply,
} from "@/features/messages/lib/threading";
import type { Channel, FeedItem, HomeFeed } from "@/shared/api/types";

type ReadTimestamp = (contextKey: string) => number | null;
type MarkChannelRead = (
  contextKey: string,
  readAt: string | null | undefined,
  options?: { topLevelOnly?: boolean },
) => void;

type UseChannelActivityProjectionOptions = {
  channels: Channel[];
  feed: HomeFeed | undefined;
  unreadFeedItemIds: ReadonlySet<string>;
  getChannelReadAt: ReadTimestamp;
  getOwnReadAt: ReadTimestamp;
  markChannelRead: MarkChannelRead;
  readStateVersion: number;
  threadActivityItems: ThreadActivityItem[];
  mutedRootIds: ReadonlySet<string>;
};

export function resolveChannelActivityFeedItemReadAt(
  item: Pick<FeedItem, "channelId" | "id">,
  getOwnReadAt: ReadTimestamp,
): number | null {
  return maxReadAt(
    getOwnReadAt(msgContextKey(item.id)),
    item.channelId ? getOwnReadAt(item.channelId) : null,
  );
}

/**
 * Read frontier for a thread reply, folding in the thread's own marker.
 *
 * `resolveChannelActivityFeedItemReadAt` knows only the per-message and
 * channel markers. Reading a thread advances `thread:<rootId>`, a key it never
 * looks at — so without this, marking a thread read leaves all of its replies
 * looking unread indefinitely, since nothing else advances a marker that
 * function can see. `getThreadReadAt` already folds the channel marker in
 * itself; taking the max here is belt-and-braces for a reply whose own message
 * marker is ahead of both.
 */
export function resolveThreadActivityItemReadAt(
  item: Pick<FeedItem, "channelId" | "id" | "tags">,
  getOwnReadAt: ReadTimestamp,
  getThreadReadAt: (rootId: string, channelId?: string | null) => number | null,
): number | null {
  const rootId = getThreadReference(item.tags).rootId;
  return maxReadAt(
    resolveChannelActivityFeedItemReadAt(item, getOwnReadAt),
    rootId ? getThreadReadAt(rootId, item.channelId) : null,
  );
}

export function useChannelActivityProjection({
  channels,
  feed,
  unreadFeedItemIds,
  getChannelReadAt,
  getOwnReadAt,
  markChannelRead,
  readStateVersion,
  threadActivityItems,
  mutedRootIds,
}: UseChannelActivityProjectionOptions) {
  const getThreadReadAt = React.useCallback(
    (rootId: string, channelId?: string | null) => {
      const threadReadAt = getOwnReadAt(`thread:${rootId}`);
      if (!channelId) return threadReadAt;

      const channelReadAt = getChannelReadAt(channelId);
      if (threadReadAt === null) return channelReadAt;
      if (channelReadAt === null) return threadReadAt;
      return Math.max(threadReadAt, channelReadAt);
    },
    [getChannelReadAt, getOwnReadAt],
  );
  const markThreadRead = React.useCallback(
    (rootId: string, timestamp: number) =>
      markChannelRead(
        `thread:${rootId}`,
        new Date(timestamp * 1_000).toISOString(),
      ),
    [markChannelRead],
  );
  const getMessageReadAt = React.useCallback(
    (messageId: string) => getChannelReadAt(msgContextKey(messageId)),
    [getChannelReadAt],
  );
  const getChannelActivityItemReadAt = React.useCallback(
    (item: Pick<FeedItem, "channelId" | "id">) =>
      resolveChannelActivityFeedItemReadAt(item, getOwnReadAt),
    [getOwnReadAt],
  );
  const markMessageRead = React.useCallback(
    (messageId: string, timestamp: number) =>
      markChannelRead(
        msgContextKey(messageId),
        new Date(timestamp * 1_000).toISOString(),
      ),
    [markChannelRead],
  );
  const threadActivityFeedItems = useThreadActivityFeedItems(
    threadActivityItems,
    mutedRootIds,
    channels,
  );
  const locallyUnreadFeedItems = React.useMemo(() => {
    if (!feed || unreadFeedItemIds.size === 0) return [];
    return [
      ...feed.mentions,
      ...feed.needsAction,
      ...feed.activity,
      ...feed.agentActivity,
    ].filter((item) => unreadFeedItemIds.has(item.id));
  }, [feed, unreadFeedItemIds]);
  const unreadThreadFeedItems = React.useMemo(() => {
    void readStateVersion;
    const candidatesById = new Map<string, FeedItem>(
      threadActivityFeedItems.map((item) => [item.id, item]),
    );
    for (const item of locallyUnreadFeedItems)
      candidatesById.set(item.id, item);

    return [...candidatesById.values()].filter(
      (item) =>
        isThreadReply(item.tags) &&
        (unreadFeedItemIds.has(item.id) ||
          item.createdAt >
            (resolveThreadActivityItemReadAt(
              item,
              getOwnReadAt,
              getThreadReadAt,
            ) ?? 0)),
    );
  }, [
    getOwnReadAt,
    getThreadReadAt,
    locallyUnreadFeedItems,
    readStateVersion,
    threadActivityFeedItems,
    unreadFeedItemIds,
  ]);
  const unreadThreadChannelIds = React.useMemo(
    () =>
      new Set(
        unreadThreadFeedItems.flatMap((item) =>
          item.channelId ? [item.channelId] : [],
        ),
      ) as ReadonlySet<string>,
    [unreadThreadFeedItems],
  );

  return {
    getThreadReadAt,
    markThreadRead,
    getMessageReadAt,
    getChannelActivityItemReadAt,
    markMessageRead,
    threadActivityFeedItems,
    locallyUnreadFeedItems,
    unreadThreadFeedItems,
    unreadThreadChannelIds,
  };
}
