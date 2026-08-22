import {
  formatInboxFullTimestamp,
  type InboxReply,
} from "@/features/home/lib/inbox";
import { formatTime } from "@/features/messages/lib/dateFormatters";
import { replyRecipientPubkeys } from "@/features/messages/lib/threading";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { SendChannelMessageResult } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Recipient `p` tags for an Inbox reply.
 *
 * A reply addresses the author it answers (NIP-10). That is not cosmetic here:
 * an agent's `require_mention` subscription is a `#p` REQ filter, so an Inbox
 * reply without this tag never reaches the agent being answered. The Inbox
 * sends without the channel timeline loaded, so the caller supplies the parent
 * author rather than the send path resolving it from cache.
 */
export function inboxReplyRecipientPubkeys({
  currentPubkey,
  mentionPubkeys,
  parentAuthorPubkey,
  parentEventId,
}: {
  currentPubkey?: string | null;
  mentionPubkeys: string[];
  parentAuthorPubkey: string | null;
  parentEventId: string | null;
}): string[] {
  if (!parentEventId || !parentAuthorPubkey) {
    return mentionPubkeys;
  }
  return replyRecipientPubkeys({
    currentPubkey: currentPubkey ?? "",
    mentionPubkeys,
    parentAuthorPubkey,
  });
}

/**
 * Optimistic Inbox reply row for the just-sent message.
 *
 * The Inbox renders replies from its own local list rather than the channel
 * timeline, so a sent reply needs this row to appear before the feed refresh
 * lands.
 */
export function buildOptimisticInboxReply({
  content,
  currentPubkey,
  fallbackAuthorPubkey,
  profiles,
  result,
  tags,
}: {
  content: string;
  currentPubkey?: string | null;
  fallbackAuthorPubkey: string;
  profiles?: UserProfileLookup;
  result: SendChannelMessageResult;
  tags: string[][];
}): InboxReply {
  const authorPubkey = currentPubkey ?? fallbackAuthorPubkey;
  return {
    authorLabel: currentPubkey
      ? resolveUserLabel({ currentPubkey, profiles, pubkey: authorPubkey })
      : "You",
    authorPubkey,
    avatarUrl:
      currentPubkey && profiles
        ? (profiles[normalizePubkey(currentPubkey)]?.avatarUrl ?? null)
        : null,
    content,
    createdAt: result.createdAt,
    depth: result.depth,
    fullTimestampLabel: formatInboxFullTimestamp(result.createdAt),
    id: result.eventId,
    parentId: result.parentEventId,
    rootId: result.rootEventId,
    tags,
    timeLabel: formatTime(result.createdAt),
  };
}
