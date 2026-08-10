import { makeRootIdStore } from "@/features/channels/unreadRootIdStore";
import {
  forcedUnreadMarker,
  forcedUnreadStore,
  type ForcedUnreadMap,
} from "@/features/channels/forcedUnreadStore";
import { DM_NOTIFIABLE_EVENT_KINDS } from "@/features/channels/isDmNotifiableKind";
import { mergeReadStateEventsStructured } from "@/features/channels/readState/readStateSnapshot";
import { deduplicateByCoordinate } from "@/features/channels/readState/readStateFencedLoader";
import {
  isOverrideActive,
  maxReadAt,
  msgContextKey,
  type OverrideRegister,
} from "@/features/channels/readState/readStateFormat";
import type { ReadStateProjection } from "@/features/channels/readState/readStateManager";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import { shouldNotifyForEvent } from "@/features/notifications/lib/shouldNotify";
import {
  mutedChannelIdsFromStore,
  parseMutePayload,
} from "@/features/sidebar/lib/channelMutesStorage";
import type { Community } from "@/features/communities/types";
import { withReadOnlyRelayClient } from "@/shared/api/readOnlyRelayClient";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";
import { nip44DecryptFromSelf } from "@/shared/api/tauri";
import type { ChannelType, RelayEvent } from "@/shared/api/types";
import {
  CHANNEL_MESSAGE_EVENT_KINDS,
  HOME_MENTION_EVENT_KINDS,
  KIND_CHANNEL_MUTES,
  KIND_DM_VISIBILITY,
  KIND_READ_STATE,
} from "@/shared/constants/kinds";

const KIND_NIP29_GROUP_METADATA = 39000;
const KIND_NIP29_GROUP_MEMBERS = 39002;

// Stores for thread-relationship sets. Keyed by pubkey only (no relay/community),
// so they read correctly from the same origin regardless of which community is active.
const participationStore = makeRootIdStore("buzz-thread-participation.v1");
const authoredStore = makeRootIdStore("buzz-thread-authored.v1");
const mutedRootsStore = makeRootIdStore("buzz-thread-muted.v1");
const FOLLOWS_STORAGE_KEY_PREFIX = "buzz-thread-follows.v1";

export type ThreadRelationships = {
  participatedRootIds: ReadonlySet<string>;
  followedRootIds: ReadonlySet<string>;
  authoredRootIds: ReadonlySet<string>;
  mutedRootIds: ReadonlySet<string>;
};

function readFollowedRootIds(pubkey: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(
      `${FOLLOWS_STORAGE_KEY_PREFIX}:${pubkey}`,
    );
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    const ids = new Set<string>();
    for (const entry of parsed) {
      if (
        typeof entry === "object" &&
        entry !== null &&
        typeof entry.rootId === "string"
      ) {
        ids.add(entry.rootId);
      }
    }
    return ids;
  } catch {
    return new Set();
  }
}

function defaultReadThreadRelationships(pubkey: string): ThreadRelationships {
  return {
    participatedRootIds: participationStore.read(pubkey),
    followedRootIds: readFollowedRootIds(pubkey),
    authoredRootIds: authoredStore.read(pubkey),
    mutedRootIds: mutedRootsStore.read(pubkey),
  };
}

const MEMBER_CHANNEL_LIMIT = 1000;
const METADATA_LIMIT = 1000;
const UNREAD_EXISTENCE_LIMIT = 50;
const MENTION_COUNT_LIMIT = 100;
const READ_STATE_FETCH_LIMIT = 500;
const READ_STATE_HORIZON_SECONDS = 7 * 24 * 60 * 60;

export type CommunityUnreadObserverResult = {
  hasUnread: boolean;
  mentionCount: number;
};

type CommunityUnreadRelay = {
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
  client: CommunityUnreadRelay,
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

export async function pollCommunityUnread(
  community: Community,
  pubkey: string,
  getProjection?: () => ReadStateProjection | null,
): Promise<CommunityUnreadObserverResult> {
  return withReadOnlyRelayClient(community.relayUrl, (client) =>
    fetchCommunityUnread({ client, pubkey, getProjection }),
  );
}

export async function fetchCommunityUnread(args: {
  client: CommunityUnreadRelay;
  pubkey: string;
  nowSeconds?: number;
  decryptReadState?: (ciphertext: string) => Promise<string>;
  decryptMutes?: (ciphertext: string) => Promise<string>;
  readThreadRelationships?: (pubkey: string) => ThreadRelationships;
  readForcedUnread?: (pubkey: string) => ForcedUnreadMap;
  /** Coherent manager projection for override evaluation. When provided and
   *  `loadComplete` is true, the projection's `overrides` and `frontiers` are
   *  the authoritative source — the fetched registers are still used for
   *  frontier-only readAt, but override liveness is decided solely from the
   *  projection (no per-field max join with fetched registers).
   *
   *  When null or `loadComplete` is false, use fetched-deduped state only. */
  getProjection?: () => ReadStateProjection | null;
}): Promise<CommunityUnreadObserverResult> {
  const { client, pubkey } = args;
  const normalizedPubkey = pubkey.toLowerCase();
  const nowSeconds = args.nowSeconds ?? Math.floor(Date.now() / 1_000);
  const decryptMutes = args.decryptMutes ?? nip44DecryptFromSelf;
  const readRelationships =
    args.readThreadRelationships ?? defaultReadThreadRelationships;
  const readForcedUnread =
    args.readForcedUnread ?? ((pk) => forcedUnreadStore.read(pk));

  const channels = await fetchObservedChannels(client, pubkey);
  if (channels.length === 0) {
    return { hasUnread: false, mentionCount: 0 };
  }

  const projection = args.getProjection?.() ?? null;
  const completeProjection = projection?.loadComplete ? projection : null;

  const [readStateEvents, mutesEvents] = await Promise.all([
    completeProjection !== null
      ? // Complete projection is authoritative — still fetch for frontier data,
        // but override liveness comes from projection.overrides.
        // Override state is exempt from finite-horizon fetching: a register may
        // be older than seven days and must not be missed. Tag-free, no `since`.
        client.fetchEvents({
          kinds: [KIND_READ_STATE],
          authors: [pubkey],
          limit: READ_STATE_FETCH_LIMIT,
        })
      : client.fetchEvents({
          kinds: [KIND_READ_STATE],
          authors: [pubkey],
          "#t": ["read-state"],
          since: nowSeconds - READ_STATE_HORIZON_SECONDS,
          limit: READ_STATE_FETCH_LIMIT,
        }),
    client.fetchEvents({
      kinds: [KIND_CHANNEL_MUTES],
      authors: [pubkey],
      "#d": ["channel-mutes"],
      limit: 1,
    }),
  ]);

  // Read state: use structured (deduplicated by coordinate) when a complete
  // projection is available; use the simpler horizon-filtered merge otherwise.
  let readStateMap: ReadonlyMap<string, number>;
  let authoritative: ReadonlyMap<string, OverrideRegister>;

  if (completeProjection !== null) {
    const structured = await mergeReadStateEventsStructured(
      deduplicateByCoordinate(readStateEvents),
      pubkey,
      args.decryptReadState,
    );
    // Override liveness source: projection.overrides is the SOLE authority.
    authoritative = completeProjection.overrides;
    // Effective frontier: max of fetched and projection frontier.
    const merged = new Map<string, number>(structured.frontiers);
    for (const [ctx, pf] of completeProjection.frontiers) {
      const existing = merged.get(ctx);
      merged.set(ctx, existing !== undefined ? Math.max(existing, pf) : pf);
    }
    readStateMap = merged;
  } else {
    const structured = await mergeReadStateEventsStructured(
      deduplicateByCoordinate(readStateEvents),
      pubkey,
      args.decryptReadState,
    );
    // No complete projection — use fetched-deduped structured state as the
    // override authority so active remote registers are not silently dropped.
    authoritative = structured.overrides;
    readStateMap = structured.frontiers;
  }

  let mutedIds = new Set<string>();
  if (mutesEvents.length > 0) {
    try {
      const plaintext = await decryptMutes(mutesEvents[0].content);
      const store = parseMutePayload(JSON.parse(plaintext));
      if (store) {
        mutedIds = mutedChannelIdsFromStore(store);
      }
    } catch {
      // decryption failure → treat as empty mutes set
    }
  }

  const {
    participatedRootIds,
    followedRootIds,
    authoredRootIds,
    mutedRootIds,
  } = readRelationships(normalizedPubkey);

  // Channels manually marked unread on this device (used when no complete
  // projection is available — the projection's overrides map supersedes this
  // when loadComplete is true).
  const forcedUnreadMap =
    completeProjection !== null ? {} : readForcedUnread(normalizedPubkey);

  let hasUnread = false;
  let mentionCount = 0;

  for (const channel of channels) {
    if (mutedIds.has(channel.id)) continue;

    // Compute readAt from effective frontiers.
    const readAt = readStateMap.get(channel.id) ?? null;

    if (!hasUnread) {
      if (completeProjection !== null) {
        // Override liveness: evaluate the authoritative register against the
        // effective frontier. Projection.overrides is the SOLE authority.
        const reg = authoritative.get(channel.id);
        if (reg !== undefined && isOverrideActive(reg, readAt ?? 0)) {
          hasUnread = true;
        }
      } else {
        // No complete projection — check fetched-deduped override registers first
        // (remote NIP-RS marks), then fall back to the locally-stored forced-unread
        // map for marks not yet synced to the relay.
        const reg = authoritative.get(channel.id);
        if (reg !== undefined && isOverrideActive(reg, readAt ?? 0)) {
          hasUnread = true;
        } else if (
          reg === undefined &&
          Object.hasOwn(forcedUnreadMap, channel.id)
        ) {
          // Forced-unread lights the dot without a relay fetch, but only if the
          // synced read marker has NOT advanced past the stored baseline. This
          // prevents stale forced-unread from lighting the rail after a cross-device
          // read has covered the channel (the drain path in useUnreadChannels only
          // runs while the community is active, so the store may not be pruned for
          // inactive communities).
          const markerAtWhenForced = forcedUnreadMarker(
            forcedUnreadMap[channel.id],
          );
          if (
            readAt === null ||
            (markerAtWhenForced !== null && readAt <= markerAtWhenForced)
          ) {
            hasUnread = true;
          }
        }
      }
    }

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
      hasUnread = unreadEvents.some(
        (event) =>
          isUnreadExternalEvent(
            event,
            readStateMap,
            readAt,
            normalizedPubkey,
          ) &&
          shouldNotifyForEvent(event, normalizedPubkey, {
            participatedRootIds,
            followedRootIds,
            authoredRootIds,
            mutedRootIds,
            mutedChannelIds: mutedIds,
            channelId: channel.id,
          }),
      );
    }

    mentionCount += mentionEvents.filter((event) =>
      isUnreadExternalEvent(event, readStateMap, readAt, normalizedPubkey),
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
