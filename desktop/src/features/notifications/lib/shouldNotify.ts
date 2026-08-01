import type { RelayEvent } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import { normalizePubkey } from "@/shared/lib/pubkey";

export const MESSAGE_NOTIFICATION_TAG = "buzz-notification";
export const MESSAGE_NOTIFICATION_SOUND_TAG = "buzz-notification-sound";

export type MessageNotificationTier = "update" | "blocked";

export function messageNotificationTier(
  tags: readonly (readonly string[])[],
): MessageNotificationTier | null {
  const value = tags.find((tag) => tag[0] === MESSAGE_NOTIFICATION_TAG)?.[1];
  return value === "update" || value === "blocked" ? value : null;
}

export function messageNotificationSound(
  tags: readonly (readonly string[])[],
): "amp" | null {
  const value = tags.find(
    (tag) => tag[0] === MESSAGE_NOTIFICATION_SOUND_TAG,
  )?.[1];
  return value === "amp" ? value : null;
}

export function isBlockedNotificationForUser(
  event: RelayEvent,
  currentPubkey: string,
  knownAgentPubkeys: ReadonlySet<string>,
): boolean {
  return (
    knownAgentPubkeys.has(normalizePubkey(event.pubkey)) &&
    messageNotificationTier(event.tags) === "blocked" &&
    hasMentionForEvent(event, currentPubkey)
  );
}

export function hasMentionForEvent(
  event: RelayEvent,
  currentPubkey: string,
): boolean {
  return (
    currentPubkey.length > 0 &&
    event.tags.some(
      (tag) => tag[0] === "p" && tag[1]?.toLowerCase() === currentPubkey,
    )
  );
}

export type NotifyOptions = {
  participatedRootIds: ReadonlySet<string>;
  followedRootIds: ReadonlySet<string>;
  authoredRootIds: ReadonlySet<string>;
  mutedRootIds?: ReadonlySet<string>;
  mutedChannelIds?: ReadonlySet<string>;
  channelId?: string | null;
};

export function shouldNotifyForEvent(
  event: RelayEvent,
  currentPubkey: string,
  options: NotifyOptions,
): boolean {
  const {
    participatedRootIds,
    followedRootIds,
    authoredRootIds,
    mutedRootIds = new Set(),
    mutedChannelIds = new Set(),
    channelId = null,
  } = options;
  const { parentId, rootId } = getThreadReference(event.tags);

  if (isBroadcastReply(event.tags)) {
    return true;
  }

  if (hasMentionForEvent(event, currentPubkey)) {
    return true;
  }

  if (channelId !== null && mutedChannelIds.has(channelId)) {
    return false;
  }

  if (parentId === null) {
    return true;
  }

  if (rootId !== null && mutedRootIds.has(rootId)) {
    return false;
  }

  if (rootId !== null && participatedRootIds.has(rootId)) {
    return true;
  }

  if (rootId !== null && followedRootIds.has(rootId)) {
    return true;
  }

  if (rootId !== null && authoredRootIds.has(rootId)) {
    return true;
  }

  return false;
}

export function isHighPriorityEventForUser(
  event: RelayEvent,
  currentPubkey: string,
): boolean {
  if (
    currentPubkey.length > 0 &&
    event.tags.some(
      (tag) => tag[0] === "p" && tag[1]?.toLowerCase() === currentPubkey,
    )
  ) {
    return true;
  }
  if (isBroadcastReply(event.tags)) {
    return true;
  }
  return false;
}
