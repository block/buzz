import type {
  ChannelMember,
  FeedItem,
  HomeFeedResponse,
  RelayEvent,
} from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { normalizePubkey } from "@/shared/lib/pubkey";

const CHANNEL_MESSAGE_KINDS = new Set<number>(CHANNEL_MESSAGE_EVENT_KINDS);
const HEX_PUBKEY = /^[0-9a-f]{64}$/;

export function dmPeerPubkeysFromMembers(
  members: readonly Pick<ChannelMember, "pubkey">[],
  currentPubkey: string | undefined,
): string[] {
  const self = normalizePubkey(currentPubkey ?? "");
  const normalized = [
    ...new Set(members.map((member) => normalizePubkey(member.pubkey))),
  ].filter((pubkey) => HEX_PUBKEY.test(pubkey));
  if (!HEX_PUBKEY.test(self) || !normalized.includes(self)) return [];
  return normalized.filter((pubkey) => pubkey !== self);
}

// The resurface subscription is `#h`-scoped to the hidden-DM set, so the relay
// only delivers events already addressed to a hidden channel the reader belongs
// to. Eligibility therefore drops the `#p` requirement — an untagged DM (a CLI
// or agent send that omits participant `p` tags) still resurfaces the row.
export function isIncomingChannelMessageFromOther(
  event: RelayEvent,
  currentPubkey: string | undefined,
): boolean {
  const self = normalizePubkey(currentPubkey ?? "");
  return (
    self.length > 0 &&
    CHANNEL_MESSAGE_KINDS.has(event.kind) &&
    relayEventChannelId(event) !== null &&
    normalizePubkey(event.pubkey) !== self
  );
}

export function relayEventChannelId(event: RelayEvent): string | null {
  return event.tags.find((tag) => tag[0] === "h" && tag[1])?.[1] ?? null;
}

export function markHiddenDmFeedItems(
  feed: HomeFeedResponse,
  hiddenDmIds: ReadonlySet<string>,
): HomeFeedResponse {
  if (hiddenDmIds.size === 0) return feed;

  const mark = (item: FeedItem): FeedItem =>
    item.channelId && hiddenDmIds.has(item.channelId)
      ? { ...item, channelType: "dm" }
      : item;

  return {
    ...feed,
    feed: {
      mentions: feed.feed.mentions.map(mark),
      needsAction: feed.feed.needsAction.map(mark),
      activity: feed.feed.activity.map(mark),
      agentActivity: feed.feed.agentActivity.map(mark),
    },
  };
}
