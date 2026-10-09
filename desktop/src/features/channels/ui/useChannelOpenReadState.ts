import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { isThreadReply } from "@/features/messages/lib/threading";
import type { FeedItem } from "@/shared/api/types";
import { useAppFocused } from "@/shared/lib/useDocumentVisible";

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

type ObserveOpenChannelReadStateOptions = {
  activeChannelId: string | null;
  activeReadAt: string | null;
  appFocused: boolean;
  isChannelMember: boolean | undefined;
  locallyUnreadFeedItems: FeedItem[];
  markChannelRead: (
    channelId: string,
    readAt: string | null,
    options: { topLevelOnly: true },
  ) => void;
  undoUnread: (itemId: string) => void;
};

export function observeOpenChannelReadState({
  activeChannelId,
  activeReadAt,
  appFocused,
  isChannelMember,
  locallyUnreadFeedItems,
  markChannelRead,
  undoUnread,
}: ObserveOpenChannelReadStateOptions) {
  if (!appFocused || !activeChannelId || isChannelMember === false) return;
  for (const itemId of getTopLevelInboxUnreadOverrideIds(
    locallyUnreadFeedItems,
    activeChannelId,
  )) {
    undoUnread(itemId);
  }
  markChannelRead(activeChannelId, activeReadAt, { topLevelOnly: true });
}

export function useChannelOpenReadState(
  activeChannelId: string | null,
  isChannelMember: boolean | undefined,
  activeReadAt: string | null,
) {
  const { feedItemState, locallyUnreadFeedItems, markChannelRead } =
    useAppShell();
  const appFocused = useAppFocused();

  React.useEffect(() => {
    observeOpenChannelReadState({
      activeChannelId,
      activeReadAt,
      appFocused,
      isChannelMember,
      locallyUnreadFeedItems,
      markChannelRead,
      undoUnread: feedItemState.undoUnread,
    });
  }, [
    activeChannelId,
    activeReadAt,
    appFocused,
    feedItemState.undoUnread,
    isChannelMember,
    locallyUnreadFeedItems,
    markChannelRead,
  ]);
}
