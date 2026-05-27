import type { RelayEvent } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";

export function shouldNotifyForEvent(
  event: RelayEvent,
  currentPubkey: string,
  participatedRootIds: ReadonlySet<string>,
  followedRootIds: ReadonlySet<string>,
  authoredRootIds: ReadonlySet<string>,
  mutedRootIds: ReadonlySet<string> = new Set(),
): boolean {
  const { parentId, rootId } = getThreadReference(event.tags);

  if (parentId === null) {
    return true;
  }

  if (isBroadcastReply(event.tags)) {
    return true;
  }

  if (
    currentPubkey.length > 0 &&
    event.tags.some(
      (tag) => tag[0] === "p" && tag[1]?.toLowerCase() === currentPubkey,
    )
  ) {
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
