/**
 * Gate for "open this thread at the reader's last-read reply".
 *
 * Computes no unread math of its own — `firstUnreadReplyId` already comes from
 * the open-time read snapshot in `useChannelUnreadState`. This only decides
 * whether that marker is allowed to steer the initial scroll.
 */
export type ThreadResumeTargetInput = {
  /** Thread currently open, or null when the panel is closed. */
  openThreadHeadId: string | null;
  /** Oldest unread reply from the open-time snapshot. */
  firstUnreadReplyId: string | null;
  /** Deep-link target owned by the panel (search hit, permalink). */
  externalScrollTargetId: string | null;
  /** Deep-link target owned by the route (`?messageId=`). */
  routeTargetMessageId: string | null;
  /** False until the thread has rendered at least one reply. */
  hasReplies: boolean;
  /** Huddle transcripts are a replay surface, not a reading position. */
  isHuddleTranscript: boolean;
};

/**
 * The reply to land on when a thread opens, or null to keep today's
 * bottom-pin.
 *
 * A deep link names its own destination and always wins — matching mobile,
 * where `hasUnreadDeepLink` short-circuits the same way.
 */
export function selectThreadResumeTargetId(
  input: ThreadResumeTargetInput,
): string | null {
  if (input.openThreadHeadId === null) return null;
  if (input.isHuddleTranscript) return null;
  if (input.routeTargetMessageId !== null) return null;
  if (input.externalScrollTargetId !== null) return null;
  if (!input.hasReplies) return null;
  return input.firstUnreadReplyId;
}
