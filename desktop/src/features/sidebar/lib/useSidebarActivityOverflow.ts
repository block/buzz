import { useOffscreenActivityChannelIds } from "@/features/sidebar/lib/useOffscreenActivityChannelIds";
import { useUnreadOverflow } from "@/features/sidebar/lib/useUnreadOverflow";

type ActivityOptions = Parameters<typeof useOffscreenActivityChannelIds>[0];
type ScrollRef = Parameters<typeof useUnreadOverflow>[0]["scrollRef"];

export function getSidebarActivityOverflowLabel({
  activityCount,
  messageCount,
}: {
  activityCount: number;
  messageCount: number;
}) {
  return activityCount === messageCount
    ? undefined
    : `${activityCount} new activity`;
}

export function hasHighPriorityOverflow(
  offscreenChannelIds: readonly string[],
  highPriorityUnreadChannelIds: ReadonlySet<string>,
) {
  return offscreenChannelIds.some((channelId) =>
    highPriorityUnreadChannelIds.has(channelId),
  );
}

export function getOffscreenMentionCount(
  offscreenChannelIds: readonly string[],
  dmChannelIds: ReadonlySet<string>,
  highPriorityUnreadChannelIds: ReadonlySet<string>,
  unreadChannelCounts: ReadonlyMap<string, number>,
) {
  return offscreenChannelIds.reduce(
    (count, channelId) =>
      count +
      (!dmChannelIds.has(channelId) &&
      highPriorityUnreadChannelIds.has(channelId)
        ? (unreadChannelCounts.get(channelId) ?? 0)
        : 0),
    0,
  );
}

export function useSidebarActivityOverflow({
  dmChannelIds,
  highPriorityUnreadChannelIds,
  scrollRef,
  unreadChannelCounts,
  ...activityOptions
}: ActivityOptions & {
  dmChannelIds: ReadonlySet<string>;
  highPriorityUnreadChannelIds: ReadonlySet<string>;
  scrollRef: ScrollRef;
  unreadChannelCounts: ReadonlyMap<string, number>;
}) {
  const { channelIds, messageChannelIds } =
    useOffscreenActivityChannelIds(activityOptions);
  const activityOverflow = useUnreadOverflow({
    scrollRef,
    unreadChannelIds: channelIds,
  });
  const messageOverflow = useUnreadOverflow({
    scrollRef,
    unreadChannelIds: messageChannelIds,
  });

  return {
    ...activityOverflow,
    unreadMessageAboveChannelIds: messageOverflow.unreadAboveChannelIds,
    unreadMessageBelowChannelIds: messageOverflow.unreadBelowChannelIds,
    hasHighPriorityAbove: hasHighPriorityOverflow(
      messageOverflow.unreadAboveChannelIds,
      highPriorityUnreadChannelIds,
    ),
    hasHighPriorityBelow: hasHighPriorityOverflow(
      messageOverflow.unreadBelowChannelIds,
      highPriorityUnreadChannelIds,
    ),
    mentionAboveCount: getOffscreenMentionCount(
      messageOverflow.unreadAboveChannelIds,
      dmChannelIds,
      highPriorityUnreadChannelIds,
      unreadChannelCounts,
    ),
    mentionBelowCount: getOffscreenMentionCount(
      messageOverflow.unreadBelowChannelIds,
      dmChannelIds,
      highPriorityUnreadChannelIds,
      unreadChannelCounts,
    ),
    unreadAboveLabel: getSidebarActivityOverflowLabel({
      activityCount: activityOverflow.unreadAboveCount,
      messageCount: messageOverflow.unreadAboveCount,
    }),
    unreadBelowLabel: getSidebarActivityOverflowLabel({
      activityCount: activityOverflow.unreadBelowCount,
      messageCount: messageOverflow.unreadBelowCount,
    }),
  };
}
