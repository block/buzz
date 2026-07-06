import { DM_NOTIFIABLE_EVENT_KINDS } from "@/features/channels/isDmNotifiableKind";
import { mergeReadStateEvents } from "@/features/channels/readState/readStateSnapshot";
import {
  maxReadAt,
  msgContextKey,
} from "@/features/channels/readState/readStateFormat";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import type { Workspace } from "@/features/workspaces/types";
import { withReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import type { ChannelType, RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_MESSAGE_EVENT_KINDS,
  HOME_MENTION_EVENT_KINDS,
  KIND_DM_VISIBILITY,
  KIND_READ_STATE,
} from "@/shared/constants/kinds";

const KIND_NIP29_GROUP_METADATA = 39000;
const KIND_NIP29_GROUP_MEMBERS = 39002;

const MEMBER_CHANNEL_LIMIT = 1000;
const METADATA_LIMIT = 1000;
const UNREAD_EXISTENCE_LIMIT = 50;
const MENTION_COUNT_LIMIT = 100;
const READ_STATE_FETCH_LIMIT = 500;
const READ_STATE_HORIZON_SECONDS = 7 * 24 * 60 * 60;

export type WorkspaceUnreadObserverResult = {
  hasUnread: boolean;
  mentionCount: number;
};

type WorkspaceUnreadRelay = {
  fetchEvents(filter: RelaySubscriptionFilter): Promise<RelayEvent[]>;
};

type ObservedChannel = {
  id: string;
  channelType: ChannelType;
  archived: boolean;
};

/**
 * List the channels this pubkey is a member of on the observed relay,
 * excluding archived channels and hidden DMs — the same visibility set the
 * unread poll and "mark all as read" must agree on.
 */
export async function fetchObservedChannels(
  client: WorkspaceUnreadRelay,
  pubkey: string,
): Promise<ObservedChannel[]> {
  const memberEvents = await client.fetchEvents({
    kinds: [KIND_NIP29_GROUP_MEMBERS],
    "#p": [pubkey],
    limit: MEMBER_CHANNEL_LIMIT,
  });
  const channelIds = extractMemberChannelIds(memberEvents);
  if (channelIds.length === 0) {
    return [];
  }

  const [metadataEvents, visibilityEvents] = await Promise.all([
    client.fetchEvents({
      kinds: [KIND_NIP29_GROUP_METADATA],
      "#d": channelIds,
      limit: METADATA_LIMIT,
    }),
    client.fetchEvents({
      kinds: [KIND_DM_VISIBILITY],
      "#p": [pubkey],
      limit: 1,
    }),
  ]);

  const hiddenDmIds = extractHiddenDmIds(visibilityEvents);
  return resolveObservedChannels(channelIds, metadataEvents).filter(
    (channel) =>
      !channel.archived &&
      (channel.channelType !== "dm" || !hiddenDmIds.has(channel.id)),
  );
}

export async function pollWorkspaceUnread(
  workspace: Workspace,
  pubkey: string,
): Promise<WorkspaceUnreadObserverResult> {
  return withReadOnlyRelayClient(workspace.relayUrl, (client) =>
    fetchWorkspaceUnread({ client, pubkey }),
  );
}

export async function fetchWorkspaceUnread(args: {
  client: WorkspaceUnreadRelay;
  pubkey: string;
  nowSeconds?: number;
  decryptReadState?: (ciphertext: string) => Promise<string>;
}): Promise<WorkspaceUnreadObserverResult> {
  const { client, pubkey } = args;
  const normalizedPubkey = pubkey.toLowerCase();
  const nowSeconds = args.nowSeconds ?? Math.floor(Date.now() / 1_000);

  const channels = await fetchObservedChannels(client, pubkey);
  if (channels.length === 0) {
    return { hasUnread: false, mentionCount: 0 };
  }
  const readStateEvents = await client.fetchEvents({
    kinds: [KIND_READ_STATE],
    authors: [pubkey],
    "#t": ["read-state"],
    since: nowSeconds - READ_STATE_HORIZON_SECONDS,
    limit: READ_STATE_FETCH_LIMIT,
  });
  const readState = await mergeReadStateEvents(
    readStateEvents,
    pubkey,
    args.decryptReadState,
  );

  let hasUnread = false;
  let mentionCount = 0;

  for (const channel of channels) {
    const readAt = readState.get(channel.id) ?? null;
    const since = readAt === null ? 0 : readAt + 1;
    const kinds = unreadKindsForChannel(channel.channelType);

    const unreadEventsPromise: Promise<RelayEvent[]> = hasUnread
      ? Promise.resolve([])
      : client.fetchEvents({
          kinds,
          "#h": [channel.id],
          since,
          limit: UNREAD_EXISTENCE_LIMIT,
        });
    const mentionEventsPromise: Promise<RelayEvent[]> = client.fetchEvents({
      kinds: [...HOME_MENTION_EVENT_KINDS],
      "#h": [channel.id],
      "#p": [pubkey],
      since,
      limit: MENTION_COUNT_LIMIT,
    });

    const [unreadEvents, mentionEvents] = await Promise.all([
      unreadEventsPromise,
      mentionEventsPromise,
    ]);

    if (!hasUnread) {
      hasUnread = unreadEvents.some((event) =>
        isUnreadExternalEvent(event, readState, readAt, normalizedPubkey),
      );
    }

    mentionCount += mentionEvents.filter((event) =>
      isUnreadExternalEvent(event, readState, readAt, normalizedPubkey),
    ).length;
  }

  return { hasUnread: hasUnread || mentionCount > 0, mentionCount };
}

export function extractMemberChannelIds(events: RelayEvent[]): string[] {
  const ids = new Set<string>();
  for (const event of events) {
    for (const tag of event.tags) {
      if (tag[0] === "d" && tag[1]) {
        ids.add(tag[1]);
      }
    }
  }
  return [...ids];
}

export function resolveObservedChannels(
  channelIds: string[],
  metadataEvents: RelayEvent[],
): ObservedChannel[] {
  const latestMetadata = new Map<string, RelayEvent>();
  for (const event of metadataEvents) {
    const channelId = tagValue(event, "d");
    if (!channelId) continue;
    const existing = latestMetadata.get(channelId);
    if (!existing || event.created_at > existing.created_at) {
      latestMetadata.set(channelId, event);
    }
  }

  return channelIds.map((id) => {
    const metadata = latestMetadata.get(id);
    const typeTag = metadata ? tagValue(metadata, "t") : null;
    return {
      id,
      channelType: toChannelType(typeTag),
      archived:
        metadata?.tags.some(
          (tag) => tag[0] === "archived" && tag[1] === "true",
        ) ?? false,
    };
  });
}

export function extractHiddenDmIds(events: RelayEvent[]): Set<string> {
  const latest = events.reduce<RelayEvent | null>(
    (current, event) =>
      current === null || event.created_at > current.created_at
        ? event
        : current,
    null,
  );
  return new Set(
    (latest?.tags ?? [])
      .filter((tag) => tag[0] === "h" && tag[1])
      .map((tag) => tag[1]),
  );
}

function unreadKindsForChannel(channelType: ChannelType): number[] {
  return channelType === "dm"
    ? [...DM_NOTIFIABLE_EVENT_KINDS]
    : [...CHANNEL_MESSAGE_EVENT_KINDS];
}

function isUnreadExternalEvent(
  event: RelayEvent,
  readState: ReadonlyMap<string, number>,
  channelReadAt: number | null,
  normalizedPubkey: string,
): boolean {
  if (event.pubkey.toLowerCase() === normalizedPubkey) return false;

  const rootId = isBroadcastReply(event.tags)
    ? null
    : getThreadReference(event.tags).rootId;
  const readAt = maxReadAt(
    channelReadAt,
    readState.get(msgContextKey(event.id)) ?? null,
    rootId === null ? null : (readState.get(`thread:${rootId}`) ?? null),
  );

  return readAt === null || event.created_at > readAt;
}

function tagValue(event: RelayEvent, name: string): string | null {
  return event.tags.find((tag) => tag[0] === name)?.[1] ?? null;
}

function toChannelType(value: string | null): ChannelType {
  return value === "forum" || value === "dm" ? value : "stream";
}
