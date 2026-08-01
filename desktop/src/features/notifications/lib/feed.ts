import type { Channel, FeedItem, HomeFeedResponse } from "@/shared/api/types";
import {
  formatNotificationTitle,
  truncateNotificationBody,
} from "@/features/notifications/lib/notificationFormat";
import { messageNotificationTier } from "@/features/notifications/lib/shouldNotify";
import { normalizePubkey } from "@/shared/lib/pubkey";

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
  options: {
    mentions: boolean;
    needsAction: boolean;
    agentPubkeys?: ReadonlySet<string>;
  },
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
        .filter((item) => item.channelType !== "dm")
        // Agent-authored updates remain visible in the feed, unread state, and
        // ticker. Only an explicit blocked tier can become a system alert.
        // The sender check keeps human-authored messages on normal semantics.
        .filter(
          (item) =>
            !options.agentPubkeys?.has(normalizePubkey(item.pubkey)) ||
            messageNotificationTier(item.tags) === "blocked",
        ),
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
