import type { RelayEvent } from "@/shared/api/types";
import {
  eventNotifyMode,
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import {
  DEFAULT_CHANNEL_NOTIFY_STATE,
  type ResolvedChannelNotifyState,
} from "@/features/notifications/lib/resolveChannelNotifyState";

/**
 * Whether `tags` carry a direct `p`-tag mention of `normalizedPubkey`. Tag
 * values are compared case-insensitively; an empty pubkey never matches. Kept
 * tags-only so both the event-shaped ladder and the tag-shaped feed predicate
 * share one discrimination point.
 */
export function tagsMentionPubkey(
  tags: string[][] | undefined,
  normalizedPubkey: string,
): boolean {
  if (normalizedPubkey.length === 0) return false;
  return (
    tags?.some(
      (tag) => tag[0] === "p" && tag[1]?.toLowerCase() === normalizedPubkey,
    ) ?? false
  );
}

export function hasMentionForEvent(
  event: RelayEvent,
  currentPubkey: string,
): boolean {
  return tagsMentionPubkey(event.tags, currentPubkey);
}

/** Per-channel resolved notification state lookup, injected by the caller. */
export type ChannelNotifyPrefsLookup = (
  channelId: string,
) => ResolvedChannelNotifyState;

export type NotifyOptions = {
  participatedRootIds: ReadonlySet<string>;
  followedRootIds: ReadonlySet<string>;
  authoredRootIds: ReadonlySet<string>;
  mutedRootIds?: ReadonlySet<string>;
  mutedChannelIds?: ReadonlySet<string>;
  channelId?: string | null;
  /**
   * Resolved per-channel prefs (NIP-CN). When supplied it is authoritative —
   * `resolveChannelNotifyState` already folds in the legacy `channel-mutes`
   * blob, so `mutedChannelIds` is ignored.
   */
  channelPrefs?: ChannelNotifyPrefsLookup;
};

/**
 * What a single event earns the user, split by tier so a channel can record
 * unread without alerting.
 */
export type NotifyDecision = {
  /** Marks the channel unread / advances latest-per-channel / counts. */
  unread: boolean;
  /** Dock bounce, OS banner, sound tier — still slot-gated at delivery. */
  alert: boolean;
  /** Numeric-badge (mention) tier. */
  highPriority: boolean;
};

const NO_NOTIFY: NotifyDecision = Object.freeze({
  unread: false,
  alert: false,
  highPriority: false,
});

const MENTION_NOTIFY: NotifyDecision = Object.freeze({
  unread: true,
  alert: true,
  highPriority: true,
});

type ChannelStateOptions = Pick<
  NotifyOptions,
  "channelId" | "mutedChannelIds" | "channelPrefs"
>;

function channelNotifyState(
  options: ChannelStateOptions,
): ResolvedChannelNotifyState {
  const { channelId = null, channelPrefs, mutedChannelIds } = options;
  if (channelId === null) return DEFAULT_CHANNEL_NOTIFY_STATE;
  if (channelPrefs) return channelPrefs(channelId);
  // Callers not yet threading prefs still express a boolean mute.
  if (mutedChannelIds?.has(channelId)) {
    return { ...DEFAULT_CHANNEL_NOTIFY_STATE, level: "mute" };
  }
  return DEFAULT_CHANNEL_NOTIFY_STATE;
}

/**
 * The normative NIP-CN precedence ladder: the one place level, broadcast
 * opt-out, thread-follow and thread-mute semantics are combined. Pure, so the
 * non-React cross-community observer uses it too.
 *
 * Direct `p`-tag mentions pierce every level (Slack keeps their badge);
 * `@channel` / `@here` markers are gated by the level and the broadcasts
 * opt-out; top-level posts and NIP-CW broadcast replies sit under the level
 * gate; thread replies need a follow (explicit, participation, authorship or
 * the channel's "follow every thread") and lose to both mutes.
 */
export function notifyDecisionForEvent(
  event: RelayEvent,
  currentPubkey: string,
  options: NotifyOptions,
): NotifyDecision {
  const {
    participatedRootIds,
    followedRootIds,
    authoredRootIds,
    mutedRootIds = new Set(),
  } = options;
  const { parentId, rootId } = getThreadReference(event.tags);
  const state = channelNotifyState(options);

  if (hasMentionForEvent(event, currentPubkey)) {
    return MENTION_NOTIFY;
  }

  if (
    eventNotifyMode(event.tags) !== null &&
    state.level !== "mute" &&
    state.broadcasts
  ) {
    return MENTION_NOTIFY;
  }

  const broadcastReply = isBroadcastReply(event.tags);
  if (parentId === null || broadcastReply) {
    if (state.level === "mute") return NO_NOTIFY;
    if (state.level === "mentions") {
      return { unread: true, alert: false, highPriority: false };
    }
    return { unread: true, alert: true, highPriority: broadcastReply };
  }

  if (rootId !== null && mutedRootIds.has(rootId)) return NO_NOTIFY;

  const followsThread =
    state.followAllThreads ||
    (rootId !== null &&
      (participatedRootIds.has(rootId) ||
        followedRootIds.has(rootId) ||
        authoredRootIds.has(rootId)));
  if (!followsThread || state.level === "mute") return NO_NOTIFY;

  return { unread: true, alert: true, highPriority: false };
}

/**
 * Channel gate for a Home-feed item (badge counts and feed-driven desktop
 * banners). The feed has no event graph, so it cannot run the full ladder —
 * this is the ladder's channel dimension expressed over what a `FeedItem`
 * carries: its `notify` marker tags and whether the relay categorised it as a
 * mention.
 *
 * Evaluated in the ladder's order, first match wins:
 *
 * 1. a direct `p`-tag mention of `currentPubkey` pierces every level — even
 *    when the same item also carries an `@channel` / `@here` marker;
 * 2. otherwise `@channel` / `@here` items obey the level and the broadcasts
 *    opt-out (NIP-CN N7) instead of riding the mention exemption;
 * 3. otherwise a relay-categorised mention passes (the feed may know the item
 *    is a mention without exposing the tags that prove it);
 * 4. otherwise the item is suppressed while the channel resolves to "mute".
 *
 * `isMentionCategory` is passed in because the feed's category taxonomy is
 * spelled differently at different call sites. `currentPubkey` must already be
 * normalized (trimmed, lowercased) or "".
 */
export function allowsFeedItemForChannel(
  state: ResolvedChannelNotifyState,
  isMentionCategory: boolean,
  tags: string[][] | undefined,
  currentPubkey: string,
): boolean {
  if (tagsMentionPubkey(tags, currentPubkey)) return true;
  if (eventNotifyMode(tags ?? []) !== null) {
    return state.level !== "mute" && state.broadcasts;
  }
  if (isMentionCategory) return true;
  return state.level !== "mute";
}

/**
 * Mention-tier classification for badge counts. Agrees with
 * `notifyDecisionForEvent(...).highPriority` — channel-blind callers get the
 * default state, so an `@channel` in an opted-out or muted channel and a
 * broadcast reply below level "all" are no longer badged.
 */
export function isHighPriorityEventForUser(
  event: RelayEvent,
  currentPubkey: string,
  options: ChannelStateOptions = {},
): boolean {
  if (hasMentionForEvent(event, currentPubkey)) {
    return true;
  }
  const state = channelNotifyState(options);
  if (eventNotifyMode(event.tags) !== null) {
    return state.level !== "mute" && state.broadcasts;
  }
  if (isBroadcastReply(event.tags)) {
    return state.level === "all";
  }
  return false;
}
