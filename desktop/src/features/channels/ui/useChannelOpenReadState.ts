import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { isThreadReply } from "@/features/messages/lib/threading";
import type { FeedItem } from "@/shared/api/types";

/**
 * Inbox overrides for top-level rows are consumed by opening the channel.
 * Thread-reply overrides intentionally remain until their thread is read.
 */
export function getTopLevelInboxUnreadOverrideIds(
  items: FeedItem[],
  channelId: string,
): string[] {
  return items.flatMap((item) =>
    item.channelId === channelId && !isThreadReply(item.tags) ? [item.id] : [],
  );
}

export function shouldAdvanceChannelReadState({
  activeChannelId,
  enabled,
  isChannelMember,
}: {
  activeChannelId: string | null;
  enabled: boolean;
  isChannelMember: boolean | undefined;
}): boolean {
  return enabled && activeChannelId !== null && isChannelMember !== false;
}

export function useChannelOpenReadState(
  activeChannelId: string | null,
  isChannelMember: boolean | undefined,
  activeReadAt: string | null,
  enabled = true,
) {
  const { feedItemState, locallyUnreadFeedItems, markChannelRead } =
    useAppShell();

  React.useEffect(() => {
    if (
      !shouldAdvanceChannelReadState({
        activeChannelId,
        enabled,
        isChannelMember,
      }) ||
      !activeChannelId
    ) {
      return;
    }
    for (const itemId of getTopLevelInboxUnreadOverrideIds(
      locallyUnreadFeedItems,
      activeChannelId,
    )) {
      feedItemState.undoUnread(itemId);
    }
    markChannelRead(activeChannelId, activeReadAt, { topLevelOnly: true });
  }, [
    activeChannelId,
    activeReadAt,
    enabled,
    feedItemState.undoUnread,
    isChannelMember,
    locallyUnreadFeedItems,
    markChannelRead,
  ]);
}
