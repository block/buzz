import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import {
  getChannelIdFromTags,
  getThreadReference,
} from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import { isProjectComment } from "@/features/pulse/lib/projectComments";
import type {
  Channel,
  RelayEvent,
  UserProfileSummary,
} from "@/shared/api/types";
import { truncatePubkey } from "@/shared/lib/pubkey";

export const PULSE_CONTENT_KINDS = [1, 9, 40002, 45001, 45003];
export const PULSE_CHANNEL_KINDS = PULSE_CONTENT_KINDS.filter(
  (kind) => kind !== 1,
);
export const PULSE_SOURCE_LIMIT = 128;
export type FeedFilter = "all" | "dm" | "channel" | "agent" | "note";
export type PulseConversation = {
  id: string;
  rootId: string;
  channel: Channel | null;
  messages: TimelineMessage[];
  latestAt: number;
  isAgent: boolean;
  isPrivate: boolean;
  isMention: boolean;
};

/** Build a private view of existing events; never copy content into a public note. */
export function buildPulseConversations(
  events: RelayEvent[],
  channels: Channel[],
  currentPubkey: string | undefined,
  profiles: Record<string, UserProfileSummary>,
  agentPubkeys: ReadonlySet<string>,
  relaySelfPubkey?: string | null,
): PulseConversation[] {
  const channelMap = new Map(
    channels.filter((c) => c.isMember && !c.archivedAt).map((c) => [c.id, c]),
  );
  const uniqueEvents = [
    ...new Map(events.map((event) => [event.id, event])).values(),
  ];
  const groups = new Map<string, PulseConversation>();
  const deleted = new Set(
    uniqueEvents
      .filter((e) => e.kind === 5 || e.kind === 9005)
      .flatMap((e) => e.tags.filter((t) => t[0] === "e").map((t) => t[1])),
  );
  const messages: Array<{ message: TimelineMessage; channel: Channel | null }> =
    [];
  for (const channel of channelMap.values()) {
    const scoped = uniqueEvents.filter(
      (e) =>
        getChannelIdFromTags(e.tags) === channel.id ||
        ([40003, 5, 9005].includes(e.kind) && !getChannelIdFromTags(e.tags)),
    );
    for (const message of formatTimelineMessages(
      scoped,
      channel,
      currentPubkey,
      null,
      profiles,
      undefined,
      undefined,
      undefined,
      relaySelfPubkey,
    )) {
      messages.push({ message, channel });
    }
  }
  // Notes and forum posts have their own kinds, outside the stream formatter.
  for (const event of uniqueEvents) {
    if (![1, 45001, 45003].includes(event.kind) || deleted.has(event.id))
      continue;
    const channelId = getChannelIdFromTags(event.tags);
    const channel = channelId ? channelMap.get(channelId) : null;
    if (channelId && !channel) continue;
    if (event.kind !== 1 && !channel) continue;
    if (
      event.kind === 1 &&
      (channelId || isProjectComment({ ...event, createdAt: event.created_at }))
    )
      continue;
    const profile = profiles[event.pubkey.toLowerCase()];
    const thread = getThreadReference(event.tags);
    messages.push({
      channel: channel ?? null,
      message: {
        id: event.id,
        pubkey: event.pubkey,
        author: profile?.displayName ?? truncatePubkey(event.pubkey),
        avatarUrl: profile?.avatarUrl,
        createdAt: event.created_at,
        body: event.content,
        kind: event.kind,
        tags: event.tags,
        parentId: thread.parentId,
        rootId: thread.rootId,
        depth: 0,
        time: "",
      },
    });
  }
  for (const { message, channel } of messages) {
    const rootId = message.rootId ?? message.parentId ?? message.id;
    const id = `${channel?.id ?? "note"}:${rootId}`;
    const isAgent = Boolean(
      message.isAgent ||
        agentPubkeys.has(message.pubkey?.toLowerCase() ?? "") ||
        profiles[message.pubkey?.toLowerCase() ?? ""]?.isAgent,
    );
    const isMention = Boolean(
      currentPubkey &&
        message.tags?.some(
          (t) =>
            t[0] === "p" && t[1]?.toLowerCase() === currentPubkey.toLowerCase(),
        ),
    );
    const group = groups.get(id) ?? {
      id,
      rootId,
      channel,
      messages: [],
      latestAt: 0,
      isAgent: false,
      isPrivate:
        channel?.channelType === "dm" || channel?.visibility === "private",
      isMention: false,
    };
    group.messages.push({ ...message, isAgent });
    group.latestAt = Math.max(group.latestAt, message.createdAt);
    group.isAgent ||= isAgent;
    group.isMention ||= isMention;
    groups.set(id, group);
  }
  return [...groups.values()]
    .map((group) => ({
      ...group,
      messages: group.messages.sort(
        (a, b) => a.createdAt - b.createdAt || a.id.localeCompare(b.id),
      ),
    }))
    .sort((a, b) => b.latestAt - a.latestAt || a.id.localeCompare(b.id));
}

export function matchesPulseFilter(
  item: PulseConversation,
  filter: FeedFilter,
  privateOnly: boolean,
  mentionsOnly: boolean,
  search: string,
): boolean {
  if (privateOnly && !item.isPrivate) return false;
  if (mentionsOnly && !item.isMention) return false;
  if (filter === "dm" && item.channel?.channelType !== "dm") return false;
  if (
    filter === "channel" &&
    (!item.channel || item.channel.channelType === "dm")
  )
    return false;
  if (filter === "agent" && !item.isAgent) return false;
  if (filter === "note" && item.channel) return false;
  const query = search.trim().toLowerCase();
  return (
    !query ||
    [
      item.channel?.name ?? "",
      ...item.messages.flatMap((m) => [m.author, m.body]),
    ].some((text) => text.toLowerCase().includes(query))
  );
}
