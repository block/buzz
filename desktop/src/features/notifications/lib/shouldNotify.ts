import type { RelayEvent } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";

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
