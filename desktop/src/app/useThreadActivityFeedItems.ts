import * as React from "react";

import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { Channel, FeedItem } from "@/shared/api/types";

export function buildThreadActivityFeedItems(
  threadActivityItems: ThreadActivityItem[],
  mutedRootIds: ReadonlySet<string>,
  channels: Channel[],
  includeDmHomeActivity = false,
): FeedItem[] {
  const channelById = new Map(channels.map((channel) => [channel.id, channel]));

  return threadActivityItems
    .filter((item) => {
      // Defense-in-depth relay-scope fence: only synthesize rows whose channel
      // is present in the active community's channel set. Rows persisted under
      // a different relay's scope key should never reach this function, but
      // this filter is the last line of defense against cross-community leaks.
      const channel = channelById.get(item.channelId);
      if (channel === undefined) return false;
      if (
        channel.channelType === "dm" &&
        getThreadReference(item.tags).parentId === null &&
        !includeDmHomeActivity
      ) {
        return false;
      }
      const rootId = getThreadReference(item.tags).rootId;
      return !rootId || !mutedRootIds.has(rootId);
    })
    .map((item) => {
      const channel = channelById.get(item.channelId);
      return {
        id: item.id,
        kind: item.kind,
        pubkey: item.pubkey,
        content: item.content,
        createdAt: item.createdAt,
        channelId: item.channelId,
        channelName: item.channelName,
        channelType: channel?.channelType,
        tags: item.tags,
        category: "activity" as const,
      };
    });
}

export function useThreadActivityFeedItems(
  threadActivityItems: ThreadActivityItem[],
  mutedRootIds: ReadonlySet<string>,
  channels: Channel[],
  includeDmHomeActivity = false,
): FeedItem[] {
  const mutedRootIdsKey = [...mutedRootIds].sort().join("\0");

  return React.useMemo(() => {
    // mutedRootIds is a mutable Set; this key invalidates when contents change.
    void mutedRootIdsKey;
    return buildThreadActivityFeedItems(
      threadActivityItems,
      mutedRootIds,
      channels,
      includeDmHomeActivity,
    );
  }, [
    channels,
    includeDmHomeActivity,
    mutedRootIds,
    mutedRootIdsKey,
    threadActivityItems,
  ]);
}
