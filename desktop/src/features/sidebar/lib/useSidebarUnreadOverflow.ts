import * as React from "react";

import { useUnreadOverflow } from "@/features/sidebar/lib/useUnreadOverflow";

type ScrollRef = Parameters<typeof useUnreadOverflow>[0]["scrollRef"];

/**
 * Returns whether any offscreen destination is a DM or has directed unread
 * activity, which should keep the sidebar overflow control emphasized.
 */
export function hasHighPriorityOverflow(
  offscreenChannelIds: readonly string[],
  highPriorityUnreadChannelIds: ReadonlySet<string>,
  dmChannelIds: ReadonlySet<string>,
) {
  return offscreenChannelIds.some(
    (channelId) =>
      dmChannelIds.has(channelId) ||
      highPriorityUnreadChannelIds.has(channelId),
  );
}

/** Formats the accessible label for a distinct unread destination count. */
export function sidebarOverflowUnreadLabel(count: number) {
  return `${count} unread`;
}

/**
 * Projects unread message and thread activity into offscreen destination sets.
 * Message and preview destinations are unioned and deduplicated; DMs and
 * destinations with directed unread activity receive high-priority treatment.
 */
export function useSidebarUnreadOverflow({
  dmChannelIds,
  highPriorityUnreadChannelIds,
  previewActivityChannelIds,
  scrollRef,
  unreadChannelIds,
}: {
  dmChannelIds: ReadonlySet<string>;
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
      dmChannelIds,
    ),
    hasHighPriorityBelow: hasHighPriorityOverflow(
      messageOverflow.unreadBelowChannelIds,
      highPriorityUnreadChannelIds,
      dmChannelIds,
    ),
  };
}
