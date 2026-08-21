import type { Channel, FeedItem, HomeFeedResponse } from "@/shared/api/types";
import { formatMessageNotification } from "@/features/notifications/lib/notificationFormat";

export type NotificationChannel = Pick<Channel, "id" | "name" | "channelType">;

export function enrichFeedItemChannel(
  item: FeedItem,
  channels: readonly NotificationChannel[],
): FeedItem {
  const needsName = !item.channelName.trim();
  const needsType = item.channelType === undefined;
  if (!item.channelId || (!needsName && !needsType)) {
    return item;
  }

  const channel = channels.find((candidate) => candidate.id === item.channelId);
  if (!channel) {
    return item;
  }

  // Fill each missing field independently: the backend feed may supply a
  // channel name but no type (or vice versa), and the DM-exclusion filter in
  // eligibleFeedNotificationItems depends on channelType being resolved.
  return {
    ...item,
    channelName: needsName ? channel.name : item.channelName,
    channelType: needsType ? channel.channelType : item.channelType,
  };
}

function feedNotificationSource(item: FeedItem) {
  if (item.channelType === "dm") return "dm" as const;
  if (item.category === "mention") return "mention" as const;
  if (item.kind === 46010) return "approval" as const;
  return "needs_action" as const;
}

export function formatFeedNotification(item: FeedItem, senderName?: string) {
  return formatMessageNotification({
    source: feedNotificationSource(item),
    senderName,
    channelName: item.channelType !== "dm" ? item.channelName : null,
    content: item.content,
  });
}

export function notificationTitle(item: FeedItem, senderName?: string) {
  return formatFeedNotification(item, senderName).title;
}

export function notificationBody(item: FeedItem) {
  return formatFeedNotification(item).body;
}

export function collectHomeAlertItems(feed: HomeFeedResponse) {
  return [...feed.feed.mentions, ...feed.feed.needsAction];
}

export function eligibleFeedNotificationItems(
  feed: HomeFeedResponse,
  options: { mentions: boolean; needsAction: boolean },
  channels: readonly NotificationChannel[] = [],
) {
  const items: FeedItem[] = [];

  // DM notifications are handled by the real-time WebSocket hook, so we
  // exclude DM items here to avoid duplicate toasts. The backend feed emits
  // no channelType, so resolve it from the loaded channel list BEFORE
  // filtering — otherwise every DM sails through as `undefined !== "dm"`.
  if (options.mentions) {
    items.push(
      ...feed.feed.mentions
        .map((item) => enrichFeedItemChannel(item, channels))
        .filter((item) => item.channelType !== "dm"),
    );
  }

  if (options.needsAction) {
    items.push(
      ...feed.feed.needsAction.map((item) =>
        enrichFeedItemChannel(item, channels),
      ),
    );
  }

  return items.sort((left, right) => left.createdAt - right.createdAt);
}
