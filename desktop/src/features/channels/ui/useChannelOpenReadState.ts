import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { isThreadReply } from "@/features/messages/lib/threading";
import { useAppFocused } from "@/shared/lib/useDocumentVisible";
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

export function useChannelOpenReadState(
  activeChannelId: string | null,
  isChannelMember: boolean | undefined,
  activeReadAt: string | null,
) {
  const { feedItemState, locallyUnreadFeedItems, markChannelRead } =
    useAppShell();

  // Only advance the read marker while the app is actually focused and
  // visible. A backgrounded or occluded window must not mark arriving
  // messages read: NIP-RS markers are monotonic and sync across devices,
  // so a parked desktop would otherwise silently clear the unread state on
  // the user's phone (#7470). When the app regains focus, the effect
  // re-runs with the same activeReadAt and catches the marker up.
  const appFocused = useAppFocused();

  React.useEffect(() => {
    if (!activeChannelId || isChannelMember === false) return;
    if (!appFocused) return;
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
    appFocused,
    feedItemState.undoUnread,
    isChannelMember,
    locallyUnreadFeedItems,
    markChannelRead,
  ]);
}
