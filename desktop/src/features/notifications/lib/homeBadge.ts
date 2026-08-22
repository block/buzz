import type { FeedItem, HomeFeedResponse } from "@/shared/api/types";
import { maxReadAt } from "@/features/channels/readState/readStateFormat";
import {
  getThreadReference,
  isBroadcastReply,
  isThreadReply,
} from "@/features/messages/lib/threading";

const EMPTY_DM_CHANNEL_IDS: ReadonlySet<string> = new Set();

function dedupeFeedItemsById(items: readonly FeedItem[]): FeedItem[] {
  const seen = new Set<string>();
  const result: FeedItem[] = [];
  for (const item of items) {
    if (seen.has(item.id)) {
      continue;
    }
    seen.add(item.id);
    result.push(item);
  }
  return result;
}

/**
 * Whether a mention-feed item belongs in the Home/dock badge count.
 *
 * A reply reaches the mention feed only because of its NIP-10 addressing `p`
 * tag, which is byte-identical to a typed `@mention`. The backend marks those
 * with `replyToSelf`, and the toast path already skips them — so counting them
 * here would make the numeral disagree with what the user was actually shown.
 *
 * It also closes a mute bypass: this count is not thread-mute aware (it has no
 * `mutedRootIds` input, and the only mute check downstream is per-channel), so
 * a reply to your own message in a muted thread inside an *unmuted* channel
 * would increment the Inbox numeral and the macOS dock badge while the sidebar
 * and toasts correctly stayed silent. Dropping `replyToSelf` items removes that
 * whole class, because a reply that is muted-but-still-p-tags-you is exactly a
 * reply to your own message.
 *
 * A typed `@mention` inside a muted thread is *not* dropped — mentions are meant
 * to pierce mutes, and those carry `replyToSelf: false`.
 */
function isBadgeCountableMention(
  item: FeedItem,
  localUnreadFeedIds: ReadonlySet<string>,
  dmChannelIds: ReadonlySet<string>,
): boolean {
  // An explicit "Mark as unread" outranks the rule. The Inbox still renders
  // these rows, so without this the row shows its unread dot while the numeral
  // and the dock badge stay at zero for as long as the user leaves it marked.
  if (localUnreadFeedIds.has(item.id)) {
    return true;
  }
  // DMs are exempt, matching every other path in this feature
  // (`shouldNotifyForEvent`, `communityUnreadObserver`, `catchUpParentAuthors`):
  // every DM message p-tags both participants, so there the addressing tag *is*
  // the addressing and `replyToSelf` carries no information. The backend cannot
  // make this call for us — `feed_item_from_event` never populates
  // `channel_type` — so without the channel list an answer inside a DM would
  // stop counting toward the Home numeral while the first message in the same
  // conversation still counted.
  if (item.channelId !== null && dmChannelIds.has(item.channelId)) {
    return true;
  }
  return !(item.replyToSelf === true && isThreadReply(item.tags));
}

export function buildHomeBadgeFeedItems(
  feed: HomeFeedResponse | undefined,
  extraInboxItems: readonly FeedItem[],
  localUnreadFeedIds: ReadonlySet<string>,
  dmChannelIds: ReadonlySet<string> = EMPTY_DM_CHANNEL_IDS,
): FeedItem[] {
  // Thread activity is surfaced directly on its channel's hover preview. It
  // should not also inflate the Inbox numeral, which is reserved for the
  // Inbox's own high-priority activity.
  const nonThreadExtraInboxItems = extraInboxItems.filter(
    (item) => !isThreadReply(item.tags),
  );
  const items = feed
    ? [
        ...feed.feed.mentions.filter((item) =>
          isBadgeCountableMention(item, localUnreadFeedIds, dmChannelIds),
        ),
        ...feed.feed.needsAction,
        ...nonThreadExtraInboxItems,
      ]
    : [...nonThreadExtraInboxItems];

  if (feed && localUnreadFeedIds.size > 0) {
    items.push(
      ...feed.feed.activity.filter((item) => localUnreadFeedIds.has(item.id)),
      ...feed.feed.agentActivity.filter((item) =>
        localUnreadFeedIds.has(item.id),
      ),
    );
  }

  return dedupeFeedItemsById(items);
}

/**
 * Whether a channel mute keeps this item out of the badge count.
 *
 * The other half of {@link isBadgeCountableMention}'s decision, kept in the same
 * file because the two can cancel each other. Re-testing `replyToSelf` here as
 * well used to undo the mark-as-unread override for every muted channel: the
 * only replies that survive `isBadgeCountableMention` are ones the user
 * explicitly marked unread, and dropping those again left the Inbox row dotted
 * while the numeral read zero.
 *
 * A real `@mention` deliberately survives a channel mute — mentions are meant to
 * pierce mutes.
 */
export function isMutedOutOfBadgeCount(
  item: Pick<FeedItem, "category" | "channelId">,
  mutedChannelIds: ReadonlySet<string> | undefined,
): boolean {
  return Boolean(
    item.channelId &&
      mutedChannelIds?.has(item.channelId) &&
      item.category !== "mention",
  );
}

export function shouldCountTowardHomeBadgeSubtotal(
  item: Pick<FeedItem, "channelId" | "channelType" | "tags">,
  highPriorityChannelIds: ReadonlySet<string>,
  forceHomeCount = false,
  dmChannelIds: ReadonlySet<string> = EMPTY_DM_CHANNEL_IDS,
): boolean {
  if (forceHomeCount) {
    return true;
  }

  if (item.channelId === null || !highPriorityChannelIds.has(item.channelId)) {
    return true;
  }

  const threadRef = getThreadReference(item.tags);
  const isThreadedReply =
    threadRef.parentId !== null && !isBroadcastReply(item.tags);
  // `channelType` alone is not enough: the backend never populates it on a feed
  // item (`feed_item_from_event` emits `channel_type: None`) and, unlike the toast
  // path, nothing enriches it from the channel list here. Testing only that field
  // made this a dead guard in production — a DM thread reply counted in this
  // subtotal *and* in the channel-side count, so the dock badge read 2 for one
  // message. `dmChannelIds` is the authoritative source, the same one
  // `isBadgeCountableMention` uses.
  const isDm =
    item.channelType === "dm" ||
    (item.channelId !== null && dmChannelIds.has(item.channelId));
  return isThreadedReply && !isDm;
}

type FeedItemReadState = Pick<
  FeedItem,
  "channelId" | "createdAt" | "id" | "tags"
>;

export function feedItemThreadRootId(item: Pick<FeedItem, "tags">) {
  return isThreadReply(item.tags) ? getThreadReference(item.tags).rootId : null;
}

export function isHomeBadgeFeedItemUnread(
  item: FeedItemReadState,
  options: {
    getChannelReadAt: (channelId: string) => number | null;
    getMessageReadAt?: (messageId: string) => number | null;
    getThreadReadAt: (
      rootId: string,
      channelId?: string | null,
    ) => number | null;
    isLocallyUnread?: boolean;
    seenFeedIdSet: ReadonlySet<string>;
  },
): boolean {
  if (options.isLocallyUnread) {
    return true;
  }

  const readAt = resolveHomeBadgeFeedItemReadAt(item, options);
  return readAt !== null
    ? item.createdAt > readAt
    : !options.seenFeedIdSet.has(item.id);
}

export function resolveHomeBadgeFeedItemReadAt(
  item: FeedItemReadState,
  options: {
    getChannelReadAt: (channelId: string) => number | null;
    getMessageReadAt?: (messageId: string) => number | null;
    getThreadReadAt: (
      rootId: string,
      channelId?: string | null,
    ) => number | null;
  },
): number | null {
  const threadRootId = feedItemThreadRootId(item);
  const markers: Array<number | null> = [];

  if (item.channelId && !threadRootId) {
    markers.push(options.getChannelReadAt(item.channelId));
  }
  if (threadRootId) {
    markers.push(options.getThreadReadAt(threadRootId, item.channelId));
    markers.push(options.getMessageReadAt?.(item.id) ?? null);
  }

  return maxReadAt(...markers);
}
