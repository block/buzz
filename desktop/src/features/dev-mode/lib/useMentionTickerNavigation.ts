import * as React from "react";

import type { DevMentionTickerItem } from "./mentionTicker";

export function useMentionTickerNavigation({
  activeChannelId,
  item,
  onDismiss,
  onOpenChannel,
  onOpenThread,
}: {
  activeChannelId: string | null;
  item: DevMentionTickerItem | null;
  onDismiss: () => void;
  onOpenChannel: (channelId: string) => void;
  onOpenThread: (rootId: string) => void;
}) {
  const pendingRef = React.useRef<{
    channelId: string;
    rootId: string;
  } | null>(null);

  const openMention = React.useCallback(() => {
    if (!item) return;
    if (item.channelId === activeChannelId) {
      onOpenChannel(item.channelId);
      onOpenThread(item.threadRootId);
    } else {
      pendingRef.current = {
        channelId: item.channelId,
        rootId: item.threadRootId,
      };
      onOpenChannel(item.channelId);
    }
    onDismiss();
  }, [activeChannelId, item, onDismiss, onOpenChannel, onOpenThread]);

  const consumePendingRoot = React.useCallback((channelId: string | null) => {
    const pending = pendingRef.current;
    pendingRef.current = null;
    return pending?.channelId === channelId ? pending.rootId : null;
  }, []);

  return { consumePendingRoot, openMention };
}
