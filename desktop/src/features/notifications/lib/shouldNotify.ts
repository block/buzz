import type { PresenceStatus, RelayEvent } from "@/shared/api/types";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import {
  mentionScopeOf,
  shouldNotifyForMentionScope,
} from "@/features/messages/lib/globalMentions.mjs";

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
  /** This user's own presence, for resolving `@here`. */
  presence?: PresenceStatus;
  /**
   * Whether `@channel` may pull this user out of a muted channel. Defaults to
   * true; the per-user opt-out sets it false.
   */
  allowChannelMentionWhileMuted?: boolean;
};

/**
 * Does a global mention (`@channel` / `@here`) on this event reach this user?
 *
 * Returns false when the event carries no scope marker, so callers can treat a
 * `false` as "not applicable" and fall through to their ordinary rules.
 *
 * The audience is resolved here rather than baked into the event at send time:
 * a marker plus current membership means someone who joined after the message
 * was posted still sees it as addressed to them.
 */
export function hasGlobalMentionForEvent(
  event: RelayEvent,
  currentPubkey: string,
  options: {
    isMuted?: boolean;
    presence?: PresenceStatus;
    allowChannelMentionWhileMuted?: boolean;
  } = {},
): boolean {
  const scope = mentionScopeOf(event.tags);
  if (scope === null) return false;

  return shouldNotifyForMentionScope({
    scope,
    isAuthor: event.pubkey?.toLowerCase() === currentPubkey,
    isMuted: options.isMuted ?? false,
    presence: options.presence ?? "online",
    allowChannelMentionWhileMuted:
      options.allowChannelMentionWhileMuted ?? true,
  });
}

export function shouldNotifyForEvent(
  event: RelayEvent,
  currentPubkey: string,
  options: NotifyOptions,
): boolean {
  if (
    currentPubkey.length > 0 &&
    event.pubkey?.toLowerCase() === currentPubkey.toLowerCase()
  ) {
    return false;
  }

  const {
    participatedRootIds,
    followedRootIds,
    authoredRootIds,
    mutedRootIds = new Set(),
    mutedChannelIds = new Set(),
    channelId = null,
    presence = "online",
    allowChannelMentionWhileMuted = true,
  } = options;
  const { parentId, rootId } = getThreadReference(event.tags);
  const isMutedChannel = channelId !== null && mutedChannelIds.has(channelId);

  if (isBroadcastReply(event.tags)) {
    return true;
  }

  if (hasMentionForEvent(event, currentPubkey)) {
    return true;
  }

  if (mentionScopeOf(event.tags) !== null) {
    return hasGlobalMentionForEvent(event, currentPubkey, {
      isMuted: isMutedChannel,
      presence,
      allowChannelMentionWhileMuted,
    });
  }

  if (isMutedChannel) {
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

/**
 * Is this event aimed at this user, rather than at the room in general?
 *
 * Drives the sidebar's count-vs-dot badge tier and the collapsed-section
 * rollup, and is the same predicate the notification path uses — so what shows
 * a number is what actually pinged.
 *
 * `presence` is accepted so `@here` can be evaluated: a `@here` that did not
 * reach you (because you were away) should not then sit in your sidebar
 * claiming urgency. Callers without presence to hand get the "online"
 * default, which is the conservative choice for a badge — it may over-report
 * slightly, but under-reporting would hide something aimed at you.
 */
export function isHighPriorityEventForUser(
  event: RelayEvent,
  currentPubkey: string,
  options: { presence?: PresenceStatus } = {},
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
  // Mute is deliberately not consulted here: this classifies *what the message
  // is*, not whether to interrupt. Muting already suppresses the notification
  // via `shouldNotifyForEvent`; it should not also erase the badge, since the
  // established behaviour is that muted channels still show an unread mark.
  if (
    hasGlobalMentionForEvent(event, currentPubkey, {
      presence: options.presence ?? "online",
    })
  ) {
    return true;
  }
  return false;
}
