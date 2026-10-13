// Inbox <-> push pairing: DMs and thread replies that notify you (desktop toast or
// phone push) become Inbox rows, using the same rules the relay push matcher uses:
//   dm           any message in a DM channel you're in, not authored by you
//   thread_reply a reply in a thread you started, replied in or were @-mentioned in
//                (plus threads you follow or starred, which also toast on desktop)
// Both honor the per-type switches and channel/DM/thread mutes. Messages that
// @-mention you are already Inbox mentions, so they're skipped here (no doubles).
import {
  getChannelIdFromTags,
  getThreadReference,
  isThreadReply,
} from "@/features/messages/lib/threading";
import type { FeedItem, RelayEvent } from "@/shared/api/types";

export const NOTIFY_MESSAGE_KINDS = [9, 40002];

type ChannelLite = { id: string; name: string; channelType?: string };

export type NotificationInboxOptions = {
  me: string;
  channels: readonly ChannelLite[];
  dmEnabled: boolean;
  threadRepliesEnabled: boolean;
  mutedChannelIds: ReadonlySet<string>;
  mutedRootIds: ReadonlySet<string>;
};

function mentionsMe(ev: Pick<RelayEvent, "tags">, me: string) {
  return ev.tags.some((t) => t[0] === "p" && t[1]?.toLowerCase() === me);
}

/** Thread roots you participate in, from your own recent messages and mentions of you. */
export function participatedRoots(
  mine: readonly Pick<RelayEvent, "id" | "tags">[],
  mentions: readonly Pick<RelayEvent, "id" | "tags">[],
  extra: Iterable<string> = [],
): Set<string> {
  const roots = new Set<string>(extra);
  for (const ev of [...mine, ...mentions]) {
    const ref = getThreadReference(ev.tags);
    roots.add(ref.rootId ?? ev.id);
  }
  return roots;
}

function toFeedItem(
  ev: RelayEvent,
  channel: ChannelLite | undefined,
): FeedItem {
  return {
    id: ev.id,
    kind: ev.kind,
    pubkey: ev.pubkey,
    content: ev.content,
    createdAt: ev.created_at,
    channelId: channel?.id ?? getChannelIdFromTags(ev.tags),
    channelName: channel?.name ?? "",
    channelType: channel?.channelType,
    tags: ev.tags,
    category: "activity",
  };
}

export function buildNotificationInboxItems(
  dmEvents: readonly RelayEvent[],
  replyEvents: readonly RelayEvent[],
  roots: ReadonlySet<string>,
  o: NotificationInboxOptions,
): FeedItem[] {
  const me = o.me.toLowerCase();
  const byId = new Map(o.channels.map((c) => [c.id, c]));
  const out = new Map<string, FeedItem>();
  const ok = (ev: RelayEvent) =>
    NOTIFY_MESSAGE_KINDS.includes(ev.kind) &&
    ev.pubkey.toLowerCase() !== me &&
    !mentionsMe(ev, me);
  if (o.dmEnabled) {
    for (const ev of dmEvents) {
      if (!ok(ev)) continue;
      const channelId = getChannelIdFromTags(ev.tags);
      const channel = channelId ? byId.get(channelId) : undefined;
      if (!channel || channel.channelType !== "dm") continue;
      if (o.mutedChannelIds.has(channel.id)) continue;
      out.set(ev.id, toFeedItem(ev, channel));
    }
  }
  if (o.threadRepliesEnabled) {
    for (const ev of replyEvents) {
      if (!ok(ev) || !isThreadReply(ev.tags) || out.has(ev.id)) continue;
      const rootId = getThreadReference(ev.tags).rootId;
      if (!rootId || !roots.has(rootId) || o.mutedRootIds.has(rootId)) continue;
      const channelId = getChannelIdFromTags(ev.tags);
      const channel = channelId ? byId.get(channelId) : undefined;
      // Only channels you can see in this community (relay-scope fence).
      if (!channel || o.mutedChannelIds.has(channel.id)) continue;
      // DM thread replies are DM notifications.
      if (channel.channelType === "dm" && !o.dmEnabled) continue;
      out.set(ev.id, toFeedItem(ev, channel));
    }
  }
  return [...out.values()].sort((a, b) => b.createdAt - a.createdAt);
}

/** Merge live + fetched rows by id (live copy wins). */
export function mergeFeedItemsById(
  primary: readonly FeedItem[],
  extra: readonly FeedItem[],
): FeedItem[] {
  if (extra.length === 0) return primary as FeedItem[];
  const seen = new Set(primary.map((i) => i.id));
  return [...primary, ...extra.filter((i) => !seen.has(i.id))];
}
