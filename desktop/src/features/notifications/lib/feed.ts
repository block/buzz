import type { Channel, FeedItem, HomeFeedResponse } from "@/shared/api/types";
import {
  formatNotificationTitle,
  truncateNotificationBody,
} from "@/features/notifications/lib/notificationFormat";

export type NotificationChannel = Pick<Channel, "id" | "name" | "channelType">;

export function enrichFeedItemChannel(
  item: FeedItem,
  channels: readonly NotificationChannel[],
): FeedItem {
  if (!item.channelId || item.channelName.trim()) {
    return item;
  }

  const channel = channels.find((candidate) => candidate.id === item.channelId);
  if (!channel) {
    return item;
  }

  return {
    ...item,
    channelName: channel.name,
    channelType: item.channelType ?? channel.channelType,
  };
}

export function notificationTitle(item: FeedItem, senderName?: string) {
  const channelLabel =
    item.channelType !== "dm" && item.channelName.trim()
      ? `#${item.channelName.trim()}`
      : null;

  if (item.channelType === "dm") {
    return senderName || "Direct message";
  }

  if (item.category === "mention") {
    return formatNotificationTitle({
      prefix: senderName ? `${senderName} mentioned you` : "@Mention",
      channelLabel,
    });
  }

  if (item.kind === 46010) {
    return formatNotificationTitle({
      prefix: senderName
        ? `${senderName} requested approval`
        : "Approval Requested",
      channelLabel,
    });
  }

  return formatNotificationTitle({
    prefix: senderName ? senderName : "Needs Action",
    channelLabel,
  });
}

export function notificationBody(item: FeedItem) {
  const fallback =
    item.kind === 46010
      ? "A workflow is waiting for your approval."
      : "Something in Buzz needs your attention.";
  return truncateNotificationBody(item.content, fallback);
}

export function collectHomeAlertItems(feed: HomeFeedResponse) {
  return [...feed.feed.mentions, ...feed.feed.needsAction];
}

export function eligibleFeedNotificationItems(
  feed: HomeFeedResponse,
  options: { mentions: boolean; needsAction: boolean },
) {
  const items: FeedItem[] = [];

  // DM notifications are handled by the real-time WebSocket hook, so we
  // exclude DM items here to avoid duplicate toasts.
  if (options.mentions) {
    items.push(
      ...feed.feed.mentions.filter((item) => item.channelType !== "dm"),
    );
  }

  if (options.needsAction) {
    items.push(...feed.feed.needsAction);
  }

  return items.sort((left, right) => left.createdAt - right.createdAt);
}
