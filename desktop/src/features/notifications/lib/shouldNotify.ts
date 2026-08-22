import type { RelayEvent } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
  pTagRoleFor,
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

/**
 * True only for a `p` tag that means "you were mentioned", not one that means
 * "this answers you".
 *
 * A reply p-tags the author it answers so agent `require_mention`
 * subscriptions receive it, which makes the raw tag useless for telling the
 * two apart. Pass the parent's author to separate them; without it this is
 * `hasMentionForEvent`.
 */
export function hasAuthoredMentionForEvent(
  event: RelayEvent,
  currentPubkey: string,
  parentAuthorPubkey?: string | null,
): boolean {
  if (!hasMentionForEvent(event, currentPubkey)) {
    return false;
  }
  // A broadcast reply is addressed to the channel, not just to the parent's
  // author, and `shouldNotifyForEvent` admits it before ever reading the
  // parent. The live thread-reply path never sees one either — `isThreadReply`
  // excludes them — so demoting it here would leave every surface disagreeing
  // about the same event.
  if (isBroadcastReply(event.tags)) {
    return true;
  }
  // The sender's own answer, when it gave one. Only a marker that is actually
  // present is authoritative — `unknown` and `none` fall through to the parent.
  const role = pTagRoleFor(event.tags, currentPubkey);
  if (role === "mention") {
    return true;
  }
  if (role === "addressing") {
    return false;
  }
  const { parentId } = getThreadReference(event.tags);
  return !(
    parentId !== null &&
    parentAuthorPubkey != null &&
    parentAuthorPubkey.toLowerCase() === currentPubkey.toLowerCase()
  );
}

export type NotifyOptions = {
  participatedRootIds: ReadonlySet<string>;
  followedRootIds: ReadonlySet<string>;
  authoredRootIds: ReadonlySet<string>;
  /**
   * Threads where someone `@mentioned` the user.
   *
   * A term in this gate because it is already a term in `isNotifiedForThread`,
   * which renders the thread as "Following" and hides its Follow action. Left
   * out, the two disagreed: the user was told they were subscribed to a thread
   * that never notified them again, and the control that would have subscribed
   * them was gone.
   */
  mentionedRootIds?: ReadonlySet<string>;
  mutedRootIds?: ReadonlySet<string>;
  mutedChannelIds?: ReadonlySet<string>;
  channelId?: string | null;
  /**
   * Author of the event this one replies to, when the caller can resolve it.
   *
   * Replies p-tag the author they answer so agent `require_mention`
   * subscriptions receive them. That tag is indistinguishable from a typed
   * `@mention` in the event itself, so without this hint every direct reply
   * would pierce a channel or thread mute. Supplying it keeps the mention
   * override for real mentions only. Omit it to keep the previous behaviour.
   */
  parentAuthorPubkey?: string | null;
  /**
   * Whether `channelId` is a DM.
   *
   * Every DM message p-tags both participants, so the "is this p tag a real
   * mention or just addressing?" question has no meaning there — the tag is
   * how a DM is addressed at all. Without this, a muted DM notifies for a new
   * message but stays silent when someone answers you, which is backwards.
   */
  isDmChannel?: boolean;
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
    mentionedRootIds = new Set(),
    mutedRootIds = new Set(),
    mutedChannelIds = new Set(),
    channelId = null,
    parentAuthorPubkey = null,
    isDmChannel = false,
  } = options;
  const { parentId, rootId } = getThreadReference(event.tags);

  if (isBroadcastReply(event.tags)) {
    return true;
  }

  // A reply we authored the parent of always carries our `p` tag, so that tag
  // alone cannot mean "this message mentions you". Only a real mention skips
  // the mute gates below; a reply answering us is re-admitted after them.
  // Never in a DM: there the addressing tag is the whole point, so demoting it
  // would silence answers to you while letting new messages through.
  const role = pTagRoleFor(event.tags, currentPubkey);
  const isReplyToCurrentUser =
    !isDmChannel &&
    parentId !== null &&
    currentPubkey.length > 0 &&
    // The sender's marker when it left one, the parent's author otherwise. A
    // `mention` marker settles it the other way: that is the case the parent
    // cannot decide, because the recipient is both the author being answered
    // and someone typed in the body.
    (role === "addressing" ||
      (role !== "mention" &&
        parentAuthorPubkey !== null &&
        parentAuthorPubkey.toLowerCase() === currentPubkey.toLowerCase()));

  if (!isReplyToCurrentUser && hasMentionForEvent(event, currentPubkey)) {
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

  // Past the mute gates, a reply answering us always notifies. The
  // participated/authored sets below are local and rebuilt from the unread
  // window, so on a fresh install they can be empty for a thread we started —
  // without this, "someone replied to you" would go unreported.
  if (isReplyToCurrentUser) {
    return true;
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

  // Below the mute gates on purpose. Being mentioned in a thread subscribes you
  // to it, but muting that thread afterwards still wins — the same precedence
  // `isNotifiedForThread` applies, where `mutedRootIds` short-circuits the whole
  // membership test.
  if (rootId !== null && mentionedRootIds.has(rootId)) {
    return true;
  }

  return false;
}

/**
 * Whether the parent's author is worth a relay round trip for this event.
 *
 * The query caches only hold the channel being viewed, so a cache lookup
 * always misses for a channel the user has not opened this session. Without
 * the parent, a reply's addressing `p` tag is indistinguishable from a
 * mention — it pierces a mute, and it marks the whole channel high-priority,
 * which then hides top-level items there from the dock badge.
 *
 * The trigger is the `p` tag, not a mute. An earlier version gated on the mute
 * alone, which was right while `shouldNotifyForEvent` was the only consumer
 * and wrong as soon as `isHighPriorityEventForUser` started depending on the
 * parent too — that one needs the answer in unmuted channels as well. Every
 * event that does not tag the user still stays on the synchronous path.
 */
export function needsResolvedParentAuthor(
  event: RelayEvent,
  currentPubkey: string,
  options: {
    cachedParentAuthor: string | null;
    isDmChannel?: boolean;
  },
): boolean {
  if (options.cachedParentAuthor !== null) {
    return false;
  }
  // In a DM the addressing tag is the whole point, so every consumer ignores
  // the parent's author there.
  if (options.isDmChannel === true) {
    return false;
  }
  // A broadcast reply is admitted before the parent is ever consulted, so the
  // round trip could not change the answer — and `deliver` (with the unread
  // bump behind it) would be stalled on it for nothing.
  if (isBroadcastReply(event.tags)) {
    return false;
  }
  // The whole point of the markers: when the sender said which role its `p` tag
  // plays, there is nothing to look up. This is the round trip that goes away.
  const role = pTagRoleFor(event.tags, currentPubkey);
  if (role === "addressing" || role === "mention") {
    return false;
  }
  const { parentId } = getThreadReference(event.tags);
  return parentId !== null && hasMentionForEvent(event, currentPubkey);
}

/**
 * High priority means "this is addressed at you personally".
 *
 * `parentAuthorPubkey` matters here for the same reason it does everywhere
 * else: a reply p-tags the author it answers, and treating that tag as a
 * mention marks the whole channel high-priority. `shouldCountTowardHomeBadge
 * Subtotal` then drops top-level items in a high-priority channel, so a stray
 * reply to you could hide a real approval request from the dock badge.
 */
export function isHighPriorityEventForUser(
  event: RelayEvent,
  currentPubkey: string,
  parentAuthorPubkey?: string | null,
): boolean {
  if (isBroadcastReply(event.tags)) {
    return true;
  }
  if (!hasMentionForEvent(event, currentPubkey)) {
    return false;
  }
  // The sender's marker, when present, is the answer — and it short-circuits
  // the fail-closed branch below, which only exists because a null parent
  // author is ambiguous between "not resolved" and "not us".
  const role = pTagRoleFor(event.tags, currentPubkey);
  if (role === "mention") {
    return true;
  }
  if (role === "addressing") {
    return false;
  }
  // Fails closed where notification delivery fails open. Callers resolve the
  // parent for exactly the replies that tag the user, so a null here means the
  // lookup was tried and failed — and this flag is persisted and makes
  // `shouldCountTowardHomeBadgeSubtotal` drop the channel's top-level items
  // from the dock badge. Missing a red dot after a relay flap is recoverable;
  // silently hiding an approval request until the channel is read is not.
  const { parentId } = getThreadReference(event.tags);
  if (parentId !== null && parentAuthorPubkey == null) {
    return false;
  }
  return hasAuthoredMentionForEvent(event, currentPubkey, parentAuthorPubkey);
}
