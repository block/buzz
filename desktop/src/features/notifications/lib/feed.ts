import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";

const FEED_NOTIFICATION_BODY_MAX_LENGTH = 140;

export function notificationTitle(item: FeedItem, senderName?: string) {
  const channelLabel = item.channelName.trim()
    ? ` in #${item.channelName.trim()}`
    : "";

  if (item.channelType === "dm") {
    return senderName || "Direct message";
  }

  if (item.category === "mention") {
    return senderName
      ? `${senderName} mentioned you${channelLabel}`
      : `@Mention${channelLabel}`;
  }

  if (item.kind === 46010) {
    return senderName
      ? `${senderName} requested approval${channelLabel}`
      : `Approval Requested${channelLabel}`;
  }

  return senderName
    ? `${senderName}${channelLabel}`
    : `Needs Action${channelLabel}`;
}

export function notificationBody(item: FeedItem) {
  const content = item.content.trim();
  const fallback =
    item.kind === 46010
      ? "A workflow is waiting for your approval."
      : "Something in Sprout needs your attention.";
  const body = content.length > 0 ? content : fallback;

  if (body.length <= FEED_NOTIFICATION_BODY_MAX_LENGTH) {
    return body;
  }

  return `${body.slice(0, FEED_NOTIFICATION_BODY_MAX_LENGTH - 3).trimEnd()}...`;
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
