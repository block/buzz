import * as React from "react";

import { useUnreadOverflow } from "@/features/sidebar/lib/useUnreadOverflow";

type ScrollRef = Parameters<typeof useUnreadOverflow>[0]["scrollRef"];

export function hasHighPriorityOverflow(
  offscreenChannelIds: readonly string[],
  highPriorityUnreadChannelIds: ReadonlySet<string>,
) {
  return offscreenChannelIds.some((channelId) =>
    highPriorityUnreadChannelIds.has(channelId),
  );
}

export function sidebarOverflowUnreadLabel(count: number) {
  return `${count} unread`;
}

export function useSidebarUnreadOverflow({
  highPriorityUnreadChannelIds,
  previewActivityChannelIds,
  scrollRef,
  unreadChannelIds,
}: {
  highPriorityUnreadChannelIds: ReadonlySet<string>;
  previewActivityChannelIds: ReadonlySet<string>;
  scrollRef: ScrollRef;
  unreadChannelIds: ReadonlySet<string>;
}) {
  const messageChannelIds = React.useMemo(
    () => new Set([...unreadChannelIds, ...previewActivityChannelIds]),
    [previewActivityChannelIds, unreadChannelIds],
  );
  const messageOverflow = useUnreadOverflow({
    scrollRef,
    unreadChannelIds: messageChannelIds,
  });

  return {
    ...messageOverflow,
    unreadMessageBelowChannelIds: messageOverflow.unreadBelowChannelIds,
    hasHighPriorityAbove: hasHighPriorityOverflow(
      messageOverflow.unreadAboveChannelIds,
      highPriorityUnreadChannelIds,
    ),
    hasHighPriorityBelow: hasHighPriorityOverflow(
      messageOverflow.unreadBelowChannelIds,
      highPriorityUnreadChannelIds,
    ),
  };
}
