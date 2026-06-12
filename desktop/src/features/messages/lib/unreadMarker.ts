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
 */
export function computeChannelUnreadMarker(
  messages: TimelineMessage[],
  frontierSeconds: number | null,
  suppressed = false,
): ChannelUnreadMarker {
  if (suppressed) {
    return EMPTY_MARKER;
  }

  let firstUnreadMessageId: string | null = null;
  let unreadCount = 0;

  for (const message of messages) {
    if (message.parentId) {
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
