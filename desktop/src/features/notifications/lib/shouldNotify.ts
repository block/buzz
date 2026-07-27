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
 * Transitional boolean view of {@link notifyDecisionForEvent} for call sites
 * that have not yet been split into the unread / alert tiers.
 */
export function shouldNotifyForEvent(
  event: RelayEvent,
  currentPubkey: string,
  options: NotifyOptions,
): boolean {
  return notifyDecisionForEvent(event, currentPubkey, options).unread;
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
