import type { InboxItem } from "@/features/home/lib/inbox";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import { hasMentionForEvent } from "@/features/notifications/lib/shouldNotify";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Per-channel ceiling on the catch-up query. Well under the relay's 1000-event
 * cap because every candidate is shipped to the triage backend, so breadth
 * across channels matters more than depth in any one of them.
 */
export const PER_CHANNEL_CAP = 200;

/** Ceiling on one scan payload, applied after merging both layers. */
export const TOTAL_CAP = 800;

/** Parallel catch-up queries. Each is a relay REQ, so keep the fan-out modest. */
export const FETCH_CONCURRENCY = 6;

/**
 * DM bodies are readable by this client, so triaging them means their content
 * leaves the device for the external backend. Flip to `false` to keep DMs local.
 */
export const INCLUDE_DMS = true;

export type TriageCandidate = {
  eventId: string;
  channelId: string | null;
  channelName: string | null;
  channelType: string | null;
  authorPubkey: string;
  authorLabel: string;
  createdAt: number;
  content: string;
  threadRootId: string | null;
  isMention: boolean;
  isDm: boolean;
  isReply: boolean;
  isSelf: boolean;
  /** `inbox` came from the pre-filtered home feed, `channel` from catch-up. */
  source: "inbox" | "channel" | "live";
};

type CandidateContext = {
  currentPubkey?: string;
  profiles?: UserProfileLookup;
};

type TriageChannel = Pick<Channel, "id" | "name" | "channelType">;

function labelFor(pubkey: string, context: CandidateContext) {
  return resolveUserLabel({
    pubkey,
    currentPubkey: context.currentPubkey,
    profiles: context.profiles,
    preferResolvedSelfLabel: true,
  });
}

export function candidateFromEvent(
  event: RelayEvent,
  channel: TriageChannel | undefined,
  context: CandidateContext,
): TriageCandidate {
  const reference = getThreadReference(event.tags);
  const normalizedSelf = context.currentPubkey
    ? normalizePubkey(context.currentPubkey)
    : "";

  const isSelf = Boolean(
    normalizedSelf && event.pubkey.toLowerCase() === normalizedSelf,
  );

  return {
    eventId: event.id,
    channelId:
      channel?.id ?? event.tags.find((tag) => tag[0] === "h")?.[1] ?? null,
    channelName: channel?.name ?? null,
    channelType: channel?.channelType ?? null,
    authorPubkey: event.pubkey,
    authorLabel: labelFor(event.pubkey, context),
    createdAt: event.created_at,
    content: event.content,
    threadRootId: reference.rootId,
    // Every message carries a `p` tag for its own author, so without the
    // `isSelf` guard the viewer's own messages read as mentions of themselves.
    isMention: !isSelf && hasMentionForEvent(event, normalizedSelf),
    isDm: channel?.channelType === "dm",
    isReply: reference.parentId !== null && !isBroadcastReply(event.tags),
    isSelf,
    source: "channel",
  };
}

export function candidatesFromInboxItems(
  items: readonly InboxItem[],
  context: CandidateContext,
): TriageCandidate[] {
  const normalizedSelf = context.currentPubkey
    ? normalizePubkey(context.currentPubkey)
    : "";

  return items.map((entry) => {
    const item = entry.item;
    const reference = getThreadReference(item.tags);
    const mentionTagged = item.tags.some(
      (tag) => tag[0] === "p" && tag[1]?.toLowerCase() === normalizedSelf,
    );
    const isSelf = Boolean(
      normalizedSelf && item.pubkey.toLowerCase() === normalizedSelf,
    );

    return {
      eventId: item.id,
      channelId: item.channelId,
      channelName: item.channelName || null,
      channelType: item.channelType ?? null,
      authorPubkey: item.pubkey,
      authorLabel: labelFor(item.pubkey, context),
      createdAt: item.createdAt,
      content: item.content,
      threadRootId: reference.rootId ?? entry.conversationId ?? null,
      isMention:
        !isSelf && (mentionTagged || entry.categories.includes("mention")),
      isDm: item.channelType === "dm",
      isReply: reference.parentId !== null && !isBroadcastReply(item.tags),
      isSelf,
      source: "inbox",
    };
  });
}

/**
 * Inbox items win over catch-up duplicates: they carry the resolved channel
 * name and the feed's own mention categorisation.
 */
export function mergeCandidates(
  inbox: readonly TriageCandidate[],
  channel: readonly TriageCandidate[],
): TriageCandidate[] {
  const byEventId = new Map<string, TriageCandidate>();
  for (const candidate of channel) byEventId.set(candidate.eventId, candidate);
  for (const candidate of inbox) byEventId.set(candidate.eventId, candidate);

  return [...byEventId.values()]
    .sort((a, b) => b.createdAt - a.createdAt)
    .slice(0, TOTAL_CAP);
}

export function sortOldestFirst(
  candidates: readonly TriageCandidate[],
): TriageCandidate[] {
  return [...candidates].sort((a, b) => a.createdAt - b.createdAt);
}

export async function mapWithConcurrency<Input, Output>(
  inputs: readonly Input[],
  limit: number,
  worker: (input: Input) => Promise<Output>,
): Promise<Output[]> {
  const results: Output[] = [];
  let cursor = 0;

  async function run() {
    while (cursor < inputs.length) {
      const index = cursor++;
      results[index] = await worker(inputs[index] as Input);
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(limit, inputs.length) }, run),
  );
  return results;
}

export function triageableChannels(
  channels: readonly Channel[],
): TriageChannel[] {
  return channels.filter(
    (channel) =>
      channel.isMember &&
      !channel.archivedAt &&
      (INCLUDE_DMS || channel.channelType !== "dm"),
  );
}

/**
 * Recent messages across member channels, including the viewer's own. Fibres
 * are extracted from self-authored commitments and ideas as well as inbound
 * asks, so the read frontier and `shouldNotifyForEvent` are both ignored.
 */
export async function collectChannelCandidates({
  channels,
  context,
  fetchEvents,
  kindsForChannel,
}: {
  channels: readonly Channel[];
  context: CandidateContext;
  fetchEvents: (filter: {
    kinds: number[];
    "#h": string[];
    since: number;
    limit: number;
  }) => Promise<RelayEvent[]>;
  kindsForChannel: (channelType: Channel["channelType"]) => readonly number[];
}): Promise<TriageCandidate[]> {
  const targets = triageableChannels(channels);

  const perChannel = await mapWithConcurrency(
    targets,
    FETCH_CONCURRENCY,
    async (channel) => {
      try {
        const events = await fetchEvents({
          kinds: [...kindsForChannel(channel.channelType)],
          "#h": [channel.id],
          since: 0,
          limit: PER_CHANNEL_CAP,
        });

        return events
          .filter((event) => event.content.trim().length > 0)
          .map((event) => candidateFromEvent(event, channel, context));
      } catch {
        // One unreadable channel must not sink the whole scan.
        return [];
      }
    },
  );

  return perChannel.flat();
}
