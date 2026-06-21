import * as React from "react";

import type { ThreadActivityItem } from "@/features/channels/useUnreadChannels";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { Channel, FeedItem } from "@/shared/api/types";

export function useThreadActivityFeedItems(
  threadActivityItems: ThreadActivityItem[],
  mutedRootIds: ReadonlySet<string>,
  channels: Channel[],
): FeedItem[] {
  const mutedRootIdsKey = [...mutedRootIds].sort().join("\0");
  const channelById = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  return React.useMemo(() => {
    // mutedRootIds is a mutable Set; this key invalidates when contents change.
    void mutedRootIdsKey;
    return threadActivityItems
      .filter((item) => {
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
  }, [channelById, mutedRootIds, mutedRootIdsKey, threadActivityItems]);
}
