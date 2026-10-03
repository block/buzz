import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import type { FeedItem, RelayEvent } from "@/shared/api/types";
import type { DesktopNotificationTarget } from "./desktop";

/**
 * Build the click-through navigation target for a live relay event (DM or
 * thread-reply). Every notification path constructs its target here so the
 * payload the OS hands back on activation always carries the full routing
 * anchor (eventId + threadRootId), not a hand-rolled subset.
 */
export function buildEventNotificationTarget(
  event: Pick<
    RelayEvent,
    "content" | "created_at" | "id" | "kind" | "pubkey" | "tags"
  >,
  channel: { id: string; name?: string | null },
  options: { openInThread?: boolean } = {},
): DesktopNotificationTarget {
  return {
    channelId: channel.id,
    channelName: channel.name?.trim() || null,
    content: event.content,
    createdAt: event.created_at,
    eventId: event.id,
    kind: event.kind,
    pubkey: event.pubkey,
    openInThread: options.openInThread === true,
    threadRootId: options.openInThread
      ? (getThreadReference(event.tags).rootId ?? null)
      : null,
  };
}

/** Build the click-through navigation target for a home-feed item. */
export function buildFeedItemNotificationTarget(
  item: FeedItem,
): DesktopNotificationTarget {
  const threadRootId = getThreadReference(item.tags).rootId;
  // Broadcast replies retain ancestry tags for context but render as their
  // own channel-timeline rows, so activation must not open a thread panel.
  const openInThread = threadRootId !== null && !isBroadcastReply(item.tags);

  return {
    channelId: item.channelId,
    channelName: item.channelName,
    content: item.content,
    createdAt: item.createdAt,
    eventId: item.id,
    kind: item.kind,
    pubkey: item.pubkey,
    openInThread,
    threadRootId: openInThread ? threadRootId : null,
  };
}
