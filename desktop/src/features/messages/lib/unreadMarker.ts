import type { TimelineMessage } from "@/features/messages/types";

/**
 * Identifies the first unread top-level channel message relative to a read
 * frontier captured when the channel was opened.
 *
 * "Unread" is defined against the open-time frontier, not the live read
 * marker: opening a channel immediately advances the live marker to latest,
 * so the divider must be computed from the snapshot taken before that
 * advance. Thread replies (messages with a parent) are out of scope here —
 * the channel divider marks top-level messages only.
 */
export type ChannelUnreadMarker = {
  /** Event id of the oldest unread top-level message, or null if none. */
  firstUnreadMessageId: string | null;
  /** Count of unread top-level messages at or after the first unread one. */
  unreadCount: number;
};

const EMPTY_MARKER: ChannelUnreadMarker = {
  firstUnreadMessageId: null,
  unreadCount: 0,
};

/**
 * @param messages Timeline messages in chronological order.
 * @param frontierSeconds Read frontier in unix seconds captured at channel
 *   open. `null` means the channel was never read, so every top-level message
 *   counts as unread.
 * @param suppressed When true, the channel was manually marked unread this
 *   session; there is no meaningful in-timeline boundary, so no marker is
 *   produced regardless of the frontier.
 * @param currentPubkey When provided, messages authored by this pubkey are
 *   never counted as unread (the user knows about their own posts).
 */
export function computeChannelUnreadMarker(
  messages: TimelineMessage[],
  frontierSeconds: number | null,
  suppressed = false,
  currentPubkey?: string,
): ChannelUnreadMarker {
  if (suppressed) {
    return EMPTY_MARKER;
  }

  // Normalize once: signer-emitted and identity pubkeys are both lowercase hex
  // today, but a case mismatch would otherwise miscount a self-post as unread.
  const normalizedPubkey = currentPubkey?.toLowerCase();

  let firstUnreadMessageId: string | null = null;
  let unreadCount = 0;

  for (const message of messages) {
    if (message.parentId) {
      continue;
    }
    if (
      normalizedPubkey &&
      message.pubkey?.toLowerCase() === normalizedPubkey
    ) {
      continue;
    }
    const isUnread =
      frontierSeconds === null || message.createdAt > frontierSeconds;
    if (!isUnread) {
      continue;
    }
    if (firstUnreadMessageId === null) {
      firstUnreadMessageId = message.id;
    }
    unreadCount += 1;
  }

  return firstUnreadMessageId === null
    ? EMPTY_MARKER
    : { firstUnreadMessageId, unreadCount };
}

/**
 * Thread-scoped unread marker. Counts replies newer than the thread read
 * frontier and identifies the first unread reply.
 *
 * Unlike the channel marker, every entry is a reply (no parentId filter) and
 * there is no suppression mechanism.
 */
export type ThreadUnreadMarker = {
  /** Event id of the oldest unread reply, or null if none. */
  firstUnreadReplyId: string | null;
  /** Count of unread replies at or after the first unread one. */
  unreadCount: number;
};

const EMPTY_THREAD_MARKER: ThreadUnreadMarker = {
  firstUnreadReplyId: null,
  unreadCount: 0,
};

/**
 * @param replies Thread replies in chronological order.
 * @param frontierSeconds Read frontier in unix seconds captured at thread
 *   open. `null` means the thread was never read, so every reply counts as
 *   unread.
 * @param currentPubkey When provided, replies authored by this pubkey are
 *   never counted as unread (the user knows about their own posts).
 */
export function computeThreadUnreadMarker(
  replies: Pick<TimelineMessage, "id" | "createdAt" | "pubkey">[],
  frontierSeconds: number | null,
  currentPubkey?: string,
): ThreadUnreadMarker {
  // Normalize once: see computeChannelUnreadMarker for the case-mismatch guard.
  const normalizedPubkey = currentPubkey?.toLowerCase();

  let firstUnreadReplyId: string | null = null;
  let unreadCount = 0;

  for (const reply of replies) {
    if (normalizedPubkey && reply.pubkey?.toLowerCase() === normalizedPubkey) {
      continue;
    }
    const isUnread =
      frontierSeconds === null || reply.createdAt > frontierSeconds;
    if (!isUnread) {
      continue;
    }
    if (firstUnreadReplyId === null) {
      firstUnreadReplyId = reply.id;
    }
    unreadCount += 1;
  }

  return firstUnreadReplyId === null
    ? EMPTY_THREAD_MARKER
    : { firstUnreadReplyId, unreadCount };
}
