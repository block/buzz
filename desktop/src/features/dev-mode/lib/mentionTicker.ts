import { getThreadReference } from "@/features/messages/lib/threading";
import {
  hasMentionForEvent,
  isBlockedNotificationForUser,
} from "@/features/notifications/lib/shouldNotify";
import type { Channel, RelayEvent } from "@/shared/api/types";

export const DEV_MENTION_TICKER_DURATION_MS = 6_000;

export type DevMentionTickerItem = {
  channelId: string;
  channelName: string;
  content: string;
  eventId: string;
  blocked: boolean;
  threadRootId: string;
};

export function toDevMentionTickerItem(
  event: RelayEvent,
  currentPubkey: string,
  channels: readonly Pick<Channel, "id" | "name">[],
  knownAgentPubkeys: ReadonlySet<string>,
): DevMentionTickerItem | null {
  if (!hasMentionForEvent(event, currentPubkey)) return null;
  const channelId = event.tags.find((tag) => tag[0] === "h")?.[1];
  if (!channelId) return null;

  const channelName =
    channels.find((channel) => channel.id === channelId)?.name ?? "channel";
  const content = event.content.replace(/\s+/g, " ").trim();

  return {
    channelId,
    channelName,
    content: content || "Mentioned you",
    eventId: event.id,
    blocked: isBlockedNotificationForUser(
      event,
      currentPubkey,
      knownAgentPubkeys,
    ),
    threadRootId: getThreadReference(event.tags).rootId ?? event.id,
  };
}
