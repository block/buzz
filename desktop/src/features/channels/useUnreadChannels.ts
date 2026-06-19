import * as React from "react";
import {
  EMPTY_SET,
  useLiveChannelUpdates,
  type UseLiveChannelUpdatesOptions,
} from "@/features/channels/useLiveChannelUpdates";
import { useReadState } from "@/features/channels/readState/useReadState";
import {
  getThreadReference,
  isBroadcastReply,
  isThreadReply,
} from "@/features/messages/lib/threading";
import {
  hasMentionForEvent,
  isHighPriorityEventForUser,
  shouldNotifyForEvent,
} from "@/features/notifications/lib/shouldNotify";
import type { RelayClient } from "@/shared/api/relayClientSession";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";

type UseUnreadChannelsOptions = UseLiveChannelUpdatesOptions & {
  pubkey?: string;
  relayClient?: RelayClient;
  mutedChannelIds?: ReadonlySet<string>;
};

// Per-channel cap on the catch-up REQ. We only consume the *max matching*
// event per channel, but the relay can return self-authored / non-trigger
// events that we discard client-side, so we need enough head-room for the
// filter to find one external trigger message. 1000 matches the live sub's
// per-channel limit elsewhere in the app.
const CATCH_UP_LIMIT = 1000;
const THREAD_INTEREST_CHECK_LIMIT = 1;
const THREAD_INTEREST_BACKFILL_LIMIT = 300;
const THREAD_ACTIVITY_BACKFILL_LIMIT = 150;
const THREAD_ACTIVITY_ROOT_BATCH_SIZE = 50;

// All four thread root-id sets (participation, authored, mentioned, muted)
// share the same localStorage shape: a per-pubkey JSON array of ids, capped to
// the newest N entries on write and tolerant of malformed/absent data on read.
// One factory yields the read/write pair for each so the only difference is the
// key prefix. The closures capture the prefix lexically (no `this`), so a
// caller can alias one store's `write` into a variable and call it bare.
function makeRootIdStore(prefix: string, maxEntries = 1000) {
  const storageKey = (pubkey: string) => `${prefix}:${pubkey}`;
  return {
    read(pubkey: string): Set<string> {
      try {
        const raw = window.localStorage.getItem(storageKey(pubkey));
        if (!raw) return new Set();
        const parsed = JSON.parse(raw);
        if (!Array.isArray(parsed)) return new Set();
        return new Set(
          parsed.filter((id): id is string => typeof id === "string"),
        );
      } catch {
        return new Set();
      }
    },
    write(pubkey: string, rootIds: Set<string>): void {
      try {
        const arr = [...rootIds];
        const capped =
          arr.length > maxEntries ? arr.slice(arr.length - maxEntries) : arr;
        window.localStorage.setItem(storageKey(pubkey), JSON.stringify(capped));
      } catch {
        // Ignore storage errors (private browsing, quota exceeded).
      }
    },
  };
}

const participationStore = makeRootIdStore("buzz-thread-participation.v1");
const authoredStore = makeRootIdStore("buzz-thread-authored.v1");
// Thread roots where an external message @-mentioned the current user. The
// badge gate ORs this in so a mention recipient who never participated,
// authored, or followed still gets the thread-unread badge.
const mentionedStore = makeRootIdStore("buzz-thread-mentioned.v1");
const mutedStore = makeRootIdStore("buzz-thread-muted.v1");

export type ThreadActivityItem = {
  id: string;
  kind: number;
  pubkey: string;
  content: string;
  createdAt: number;
  channelId: string;
  channelName: string;
  tags: string[][];
};

const ACTIVITY_STORAGE_PREFIX = "buzz-thread-activity.v1";
const MAX_ACTIVITY_ITEMS = 100;

function activityStorageKey(pubkey: string): string {
  return `${ACTIVITY_STORAGE_PREFIX}:${pubkey}`;
}

function readActivityFromStorage(pubkey: string): ThreadActivityItem[] {
  try {
    const raw = window.localStorage.getItem(activityStorageKey(pubkey));
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(
      (item): item is ThreadActivityItem =>
        typeof item === "object" &&
        item !== null &&
        typeof item.id === "string",
    );
  } catch {
    return [];
  }
}

function writeActivityToStorage(
  pubkey: string,
  items: ThreadActivityItem[],
): void {
  try {
    const capped =
      items.length > MAX_ACTIVITY_ITEMS
        ? items.slice(items.length - MAX_ACTIVITY_ITEMS)
        : items;
    window.localStorage.setItem(
      activityStorageKey(pubkey),
      JSON.stringify(capped),
    );
  } catch {
    // Ignore storage errors.
  }
}

function addThreadActivityItems(
  existing: ThreadActivityItem[],
  items: ThreadActivityItem[],
) {
  if (items.length === 0) {
    return { didAdd: false, items: existing };
  }

  const existingIds = new Set(existing.map((item) => item.id));
  const newItems = items.filter((item) => !existingIds.has(item.id));
  if (newItems.length === 0) {
    return { didAdd: false, items: existing };
  }

  const merged = [...existing, ...newItems].sort(
    (left, right) => left.createdAt - right.createdAt,
  );
  const capped =
    merged.length > MAX_ACTIVITY_ITEMS
      ? merged.slice(merged.length - MAX_ACTIVITY_ITEMS)
      : merged;

  return { didAdd: true, items: capped };
}

function recordSelfThreadInterest(
  event: RelayEvent,
  participatedRootIds: Set<string>,
  authoredRootIds: Set<string>,
): string {
  const ref = getThreadReference(event.tags);
  if (ref.rootId !== null) {
    participatedRootIds.add(ref.rootId);
    return ref.rootId;
  }

  authoredRootIds.add(event.id);
  return event.id;
}

function threadInterestResolutionKey(
  channelId: string,
  rootId: string,
): string {
  return `${channelId}:${rootId}`;
}

function parseTimestamp(value: string | null | undefined) {
  if (!value) {
    return null;
  }

  const timestamp = Date.parse(value);
  return Number.isNaN(timestamp) ? null : timestamp;
}

function toUnixSeconds(isoOrMs: string | null | undefined): number | null {
  const ms = parseTimestamp(isoOrMs);
  return ms === null ? null : Math.floor(ms / 1_000);
}

// Resolve where the read marker should land when a channel is marked read.
// Folds the caller's timeline position together with the newest main-channel
// event this client has observed live (`observedLatest`), so an explicit
// "mark read" still covers messages that arrived faster than channel metadata.
// This fold is load-bearing for the Esc shortcut, sidebar mark-read, and
// empty-channel open, all of which pass a null/stale caller value.
// `clearObserved` reports whether the resulting marker covers the observed
// timestamp, signalling the caller to drop its observed refs so the unread memo
// sees `latest === undefined` until a genuinely newer event arrives.
export function resolveChannelReadMarker(
  callerReadAt: string | null | undefined,
  observedLatest: number | undefined,
): { markAt: number | null; clearObserved: boolean } {
  const callerUnix = toUnixSeconds(callerReadAt);
  const markAt = Math.max(callerUnix ?? 0, observedLatest ?? 0) || null;
  return {
    markAt,
    clearObserved:
      markAt !== null &&
      observedLatest !== undefined &&
      observedLatest <= markAt,
  };
}

function setsEqual(a: ReadonlySet<string>, b: ReadonlySet<string>): boolean {
  if (a.size !== b.size) return false;
  for (const item of a) {
    if (!b.has(item)) return false;
  }
  return true;
}

function addUnreadChannelEvent(
  target: Map<string, Map<string, number>>,
  channelId: string,
  eventId: string,
  createdAt: number,
): boolean {
  let channelEvents = target.get(channelId);
  if (!channelEvents) {
    channelEvents = new Map<string, number>();
    target.set(channelId, channelEvents);
  }

  const current = channelEvents.get(eventId);
  if (current === createdAt) {
    return false;
  }

  channelEvents.set(eventId, createdAt);
  return true;
}

function pruneUnreadChannelEvents(
  target: Map<string, Map<string, number>>,
  channelId: string,
  readAt: number,
) {
  const channelEvents = target.get(channelId);
  if (!channelEvents) return;

  for (const [eventId, createdAt] of channelEvents) {
    if (createdAt <= readAt) {
      channelEvents.delete(eventId);
    }
  }

  if (channelEvents.size === 0) {
    target.delete(channelId);
  }
}

export function useUnreadChannels(
  channels: Channel[],
  activeChannel: Channel | null,
  options: UseUnreadChannelsOptions = {},
) {
  const {
    pubkey,
    relayClient,
    mutedChannelIds: mutedChannelIdsOption,
    ...liveUpdateOptions
  } = options;
  const activeChannelId = activeChannel?.id ?? null;
  const normalizedPubkey = pubkey?.toLowerCase() ?? null;

  const {
    getEffectiveTimestamp,
    getOwnTimestamp,
    isReady: isReadStateReady,
    markContextRead,
    drainSyncedAdvances,
    setContextParentResolver,
    readStateVersion,
  } = useReadState(pubkey, relayClient);

  // Observed newest external main-channel event per channel (unix seconds). This
  // is *derived relay evidence*, not source-of-truth: it's populated from a
  // one-shot catch-up REQ per channel (keyed on the NIP-RS read marker) plus
  // ongoing live events. Thread replies are recorded separately for Home inbox
  // activity and never enter this map. The only thing we ever do with it is
  // compare against the NIP-RS read marker — see the unread memo below. Reset
  // on identity change. Stale entries for channels the user has left are
  // silently ignored by the memo (it iterates the current channels list, not
  // the map).
  const latestByChannelRef = React.useRef(new Map<string, number>());
  const latestHighPriorityByChannelRef = React.useRef(
    new Map<string, number>(),
  );
  const unreadEventsByChannelRef = React.useRef(
    new Map<string, Map<string, number>>(),
  );

  const channelsRef = React.useRef(channels);
  channelsRef.current = channels;

  // Channels manually marked unread this session (e.g., right-click → "mark
  // unread"). Because NIP-RS read markers are monotonic, this in-session flag
  // is what makes the badge appear *now* without lowering synced read state.
  // Cleared when the user opens the channel.
  const forcedUnreadRef = React.useRef(new Set<string>());

  // When a synced event advances a read marker (cross-device mark-as-read),
  // remove from forcedUnreadRef so the dot clears immediately.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion is the intentional drain trigger
  React.useEffect(() => {
    const advanced = drainSyncedAdvances();
    let anyNew = false;
    for (const channelId of advanced) {
      if (forcedUnreadRef.current.delete(channelId)) {
        anyNew = true;
      }
    }
    if (anyNew) bumpLatestVersion();
  }, [readStateVersion, drainSyncedAdvances]);

  // Root event IDs of threads where the current user has replied at least once.
  // Used to determine if thread replies should trigger unread notifications.
  const participatedRootIdsRef = React.useRef(new Set<string>());

  // Root event IDs of top-level messages authored by the current user.
  // Used to notify the author when someone replies to their posts.
  const authoredRootIdsRef = React.useRef(new Set<string>());

  // Root event IDs of threads where an external message @-mentioned the user.
  // ORed into the badge gate so a mention recipient who never participated,
  // authored, or followed the thread still gets the thread-unread badge.
  const mentionedRootIdsRef = React.useRef(new Set<string>());

  // Root event IDs of threads the user has explicitly muted. Takes precedence
  // over participation, follow, and authorship for notification suppression.
  const mutedRootIdsRef = React.useRef(new Set<string>());

  // Stable ref for the caller-supplied muted channel IDs. Updated every render
  // so the catch-up loop always reads the latest set without being a dep.
  const mutedChannelIdsRef = React.useRef<ReadonlySet<string>>(new Set());
  mutedChannelIdsRef.current = mutedChannelIdsOption ?? new Set();

  // Thread reply events that triggered notifications — surfaced in the Home
  // activity feed as synthetic FeedItems.
  const threadActivityRef = React.useRef<ThreadActivityItem[]>([]);

  // Root IDs whose authored/participated interest had to be checked against
  // the relay because this local client had not observed the user's earlier
  // root/reply yet.
  const threadInterestResolutionsRef = React.useRef(
    new Map<string, Promise<boolean>>(),
  );
  const threadActivityBackfillKeyRef = React.useRef<string | null>(null);

  // Tracks which channels we've already issued a catch-up REQ for this
  // session. Prevents re-fetching on every channels-list refetch, while still
  // letting newly-joined channels be caught up. Reset on identity change.
  const caughtUpChannelsRef = React.useRef(new Set<string>());

  const [latestVersion, bumpLatestVersion] = React.useReducer(
    (x: number) => x + 1,
    0,
  );

  // Version signal bumped only when the participated/authored/mentioned
  // root-id sets change, so the gate snapshots (re-derived below) don't
  // re-allocate on every observed external message the way reusing
  // latestVersion would.
  const [membershipVersion, bumpMembershipVersion] = React.useReducer(
    (x: number) => x + 1,
    0,
  );

  const getThreadActivityReadAt = React.useCallback(
    (channelId: string, rootId: string): number | null => {
      const threadReadAt = getOwnTimestamp(`thread:${rootId}`);
      const channelReadAt = getEffectiveTimestamp(channelId);
      if (threadReadAt === null) {
        return channelReadAt;
      }
      if (channelReadAt === null) {
        return threadReadAt;
      }
      return Math.max(threadReadAt, channelReadAt);
    },
    [getEffectiveTimestamp, getOwnTimestamp],
  );

  // Reset all in-session state when the identity or relay changes. Unread
  // tracking depends only on NIP-RS read markers + observed relay events for
  // this user; nothing here is persisted across restarts.
  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey/relayClient are intentional reset signals
  React.useEffect(() => {
    latestByChannelRef.current = new Map();
    latestHighPriorityByChannelRef.current = new Map();
    unreadEventsByChannelRef.current = new Map();
    forcedUnreadRef.current = new Set();
    caughtUpChannelsRef.current = new Set();
    participatedRootIdsRef.current = pubkey
      ? participationStore.read(pubkey)
      : new Set();
    authoredRootIdsRef.current = pubkey
      ? authoredStore.read(pubkey)
      : new Set();
    mentionedRootIdsRef.current = pubkey
      ? mentionedStore.read(pubkey)
      : new Set();
    mutedRootIdsRef.current = pubkey ? mutedStore.read(pubkey) : new Set();
    threadActivityRef.current = pubkey ? readActivityFromStorage(pubkey) : [];
    threadInterestResolutionsRef.current = new Map();
    threadActivityBackfillKeyRef.current = null;
    bumpLatestVersion();
    bumpMembershipVersion();
  }, [pubkey, relayClient]);

  // `topLevelOnly` is the passive channel-open path (NIP-RS Option 1): the
  // caller's `readAt` is already the newest TOP-LEVEL message, so the marker
  // must land exactly there without folding in a newer observed top-level
  // message. Thread replies are tracked separately as Home inbox activity and
  // never participate in the sidebar channel dot.
  const markChannelRead = React.useCallback(
    (
      channelId: string,
      readAt: string | null | undefined,
      { topLevelOnly = false }: { topLevelOnly?: boolean } = {},
    ) => {
      if (forcedUnreadRef.current.delete(channelId)) {
        bumpLatestVersion();
      }
      const observedLatest = topLevelOnly
        ? undefined
        : latestByChannelRef.current.get(channelId);
      const { markAt, clearObserved } = resolveChannelReadMarker(
        readAt,
        observedLatest,
      );
      if (markAt === null) return;
      markContextRead(channelId, markAt);
      pruneUnreadChannelEvents(
        unreadEventsByChannelRef.current,
        channelId,
        markAt,
      );
      // Clear observed-latest refs when the read marker covers them so the
      // unread memo sees `latest === undefined` until a genuinely new event
      // arrives. Without this, `latest > readAt` resolves to `T > T` (false)
      // but the channel lingers in the set when advanceContext's monotonic
      // guard suppresses the readStateVersion bump.
      if (clearObserved) {
        latestByChannelRef.current.delete(channelId);
        latestHighPriorityByChannelRef.current.delete(channelId);
        bumpLatestVersion();
      }
    },
    [markContextRead],
  );

  // Manually mark a channel unread (e.g., right-click → "mark unread"). Sets
  // the in-session forced flag so the sidebar badge appears immediately. NIP-RS
  // read markers are monotonic, so we do not publish a lower timestamp.
  const markChannelUnread = React.useCallback((channelId: string) => {
    if (!forcedUnreadRef.current.has(channelId)) {
      forcedUnreadRef.current.add(channelId);
      bumpLatestVersion();
    }
  }, []);

  // Record the thread root of an EXTERNAL message that @-mentioned the user.
  // Keyed on the thread root so the badge gate trips for a mention recipient
  // who never participated/authored/followed. Top-level mentions (no rootId)
  // are ignored — thread badges only exist for replies. Returns true when the
  // set actually grew so callers can decide whether to bump the gate snapshot.
  const recordMentionedRoot = React.useCallback(
    (event: RelayEvent): boolean => {
      if (normalizedPubkey === null) return false;
      if (event.pubkey.toLowerCase() === normalizedPubkey) return false;
      if (!hasMentionForEvent(event, normalizedPubkey)) return false;
      const { rootId } = getThreadReference(event.tags);
      if (rootId === null) return false;
      const target = mentionedRootIdsRef.current;
      const sizeBefore = target.size;
      target.add(rootId);
      if (target.size === sizeBefore) return false;
      mentionedStore.write(normalizedPubkey, target);
      return true;
    },
    [normalizedPubkey],
  );

  const resolveThreadInterestFromRelay = React.useCallback(
    async (rootId: string, channelId: string): Promise<boolean> => {
      if (!relayClient || normalizedPubkey === null) {
        return false;
      }

      if (
        participatedRootIdsRef.current.has(rootId) ||
        authoredRootIdsRef.current.has(rootId)
      ) {
        return true;
      }

      if (
        mutedRootIdsRef.current.has(rootId) ||
        mutedChannelIdsRef.current.has(channelId)
      ) {
        return false;
      }

      const resolutionKey = threadInterestResolutionKey(channelId, rootId);
      const cached = threadInterestResolutionsRef.current.get(resolutionKey);
      if (cached) {
        return cached;
      }

      const resolution = (async () => {
        try {
          const [authoredRootEvents, participatedEvents] = await Promise.all([
            relayClient.fetchEvents({
              ids: [rootId],
              kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
              authors: [normalizedPubkey],
              "#h": [channelId],
              limit: THREAD_INTEREST_CHECK_LIMIT,
            }),
            relayClient.fetchEvents({
              kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
              authors: [normalizedPubkey],
              "#e": [rootId],
              "#h": [channelId],
              limit: THREAD_INTEREST_CHECK_LIMIT,
            }),
          ]);

          let didResolveInterest = false;
          if (authoredRootEvents.length > 0) {
            authoredRootIdsRef.current.add(rootId);
            didResolveInterest = true;
          }
          if (participatedEvents.length > 0) {
            participatedRootIdsRef.current.add(rootId);
            didResolveInterest = true;
          }

          if (didResolveInterest) {
            authoredStore.write(normalizedPubkey, authoredRootIdsRef.current);
            participationStore.write(
              normalizedPubkey,
              participatedRootIdsRef.current,
            );
            bumpLatestVersion();
            bumpMembershipVersion();
          }

          return didResolveInterest;
        } catch {
          threadInterestResolutionsRef.current.delete(resolutionKey);
          return false;
        }
      })();

      threadInterestResolutionsRef.current.set(resolutionKey, resolution);
      return resolution;
    },
    [normalizedPubkey, relayClient],
  );

  const resolveThreadInterestsFromRelay = React.useCallback(
    async (rootIds: string[], channelId: string): Promise<void> => {
      if (!relayClient || normalizedPubkey === null || rootIds.length === 0) {
        return;
      }

      const unresolvedRootIds = [...new Set(rootIds)].filter(
        (rootId) =>
          !participatedRootIdsRef.current.has(rootId) &&
          !authoredRootIdsRef.current.has(rootId) &&
          !mutedRootIdsRef.current.has(rootId) &&
          !mutedChannelIdsRef.current.has(channelId),
      );
      if (unresolvedRootIds.length === 0) {
        return;
      }

      try {
        const [authoredRootEvents, participatedEvents] = await Promise.all([
          relayClient.fetchEvents({
            ids: unresolvedRootIds,
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            authors: [normalizedPubkey],
            "#h": [channelId],
            limit: Math.min(unresolvedRootIds.length, CATCH_UP_LIMIT),
          }),
          relayClient.fetchEvents({
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            authors: [normalizedPubkey],
            "#e": unresolvedRootIds,
            "#h": [channelId],
            limit: CATCH_UP_LIMIT,
          }),
        ]);

        const unresolved = new Set(unresolvedRootIds);
        const resolved = new Set<string>();
        for (const event of authoredRootEvents) {
          if (unresolved.has(event.id)) {
            authoredRootIdsRef.current.add(event.id);
            resolved.add(event.id);
          }
        }
        for (const event of participatedEvents) {
          const rootId = getThreadReference(event.tags).rootId;
          if (rootId !== null && unresolved.has(rootId)) {
            participatedRootIdsRef.current.add(rootId);
            resolved.add(rootId);
          }
        }

        for (const rootId of unresolvedRootIds) {
          threadInterestResolutionsRef.current.set(
            threadInterestResolutionKey(channelId, rootId),
            Promise.resolve(resolved.has(rootId)),
          );
        }

        if (resolved.size > 0) {
          authoredStore.write(normalizedPubkey, authoredRootIdsRef.current);
          participationStore.write(
            normalizedPubkey,
            participatedRootIdsRef.current,
          );
          bumpLatestVersion();
          bumpMembershipVersion();
        }
      } catch {
        for (const rootId of unresolvedRootIds) {
          threadInterestResolutionsRef.current.delete(
            threadInterestResolutionKey(channelId, rootId),
          );
        }
      }
    },
    [normalizedPubkey, relayClient],
  );

  // Feed the in-session newest-main-channel map from live channel events.
  // Composes with any caller-supplied onChannelMessage handler.
  // useLiveChannelUpdates already filters this callback to trigger kinds
  // and external authors, so the map is always a strict subset of "newest
  // external main-channel message this client has observed."
  const callerOnChannelMessage = liveUpdateOptions.onChannelMessage;
  const callerOnThreadReplyDesktopNotification =
    liveUpdateOptions.onThreadReplyDesktopNotification;
  const notifyForActiveChannel = liveUpdateOptions.notifyForActiveChannel;
  const handleChannelMessage = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      if (isThreadReply(event.tags)) {
        return;
      }

      const current = latestByChannelRef.current.get(channelId) ?? 0;
      if (event.created_at > current) {
        latestByChannelRef.current.set(channelId, event.created_at);
        bumpLatestVersion();
      }
      if (
        addUnreadChannelEvent(
          unreadEventsByChannelRef.current,
          channelId,
          event.id,
          event.created_at,
        )
      ) {
        bumpLatestVersion();
      }

      // Broadcast-style replies are treated as main-channel activity, but can
      // still carry a thread root that should make the thread badge-eligible.
      if (recordMentionedRoot(event)) {
        bumpMembershipVersion();
      }

      // Track high-priority events (DMs, mentions, broadcasts) separately.
      const channel = channelsRef.current.find((ch) => ch.id === channelId);
      if (
        channel?.channelType === "dm" ||
        (normalizedPubkey !== null &&
          isHighPriorityEventForUser(event, normalizedPubkey))
      ) {
        const currentHigh =
          latestHighPriorityByChannelRef.current.get(channelId) ?? 0;
        if (event.created_at > currentHigh) {
          latestHighPriorityByChannelRef.current.set(
            channelId,
            event.created_at,
          );
          bumpLatestVersion();
        }
      }

      callerOnChannelMessage?.(channelId, event);
    },
    [callerOnChannelMessage, normalizedPubkey, recordMentionedRoot],
  );

  const handleSelfChannelMessage = React.useCallback(
    (event: RelayEvent) => {
      const participatedSizeBefore = participatedRootIdsRef.current.size;
      const authoredSizeBefore = authoredRootIdsRef.current.size;
      const rootId = recordSelfThreadInterest(
        event,
        participatedRootIdsRef.current,
        authoredRootIdsRef.current,
      );
      const channelId = event.tags.find((tag) => tag[0] === "h")?.[1];
      if (channelId) {
        threadInterestResolutionsRef.current.delete(
          threadInterestResolutionKey(channelId, rootId),
        );
      }
      if (normalizedPubkey !== null) {
        participationStore.write(
          normalizedPubkey,
          participatedRootIdsRef.current,
        );
        authoredStore.write(normalizedPubkey, authoredRootIdsRef.current);
      }
      if (
        participatedRootIdsRef.current.size !== participatedSizeBefore ||
        authoredRootIdsRef.current.size !== authoredSizeBefore
      ) {
        bumpMembershipVersion();
      }
      bumpLatestVersion();
    },
    [normalizedPubkey],
  );

  const handleThreadReplyNotification = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      if (recordMentionedRoot(event)) {
        bumpMembershipVersion();
      }

      const channelName =
        channels.find((ch) => ch.id === channelId)?.name ?? "";
      const item: ThreadActivityItem = {
        id: event.id,
        kind: event.kind,
        pubkey: event.pubkey,
        content: event.content,
        createdAt: event.created_at,
        channelId,
        channelName,
        tags: [...event.tags],
      };
      const existing = threadActivityRef.current;
      if (existing.some((e) => e.id === item.id)) return;
      const next = [...existing, item];
      const capped =
        next.length > MAX_ACTIVITY_ITEMS
          ? next.slice(next.length - MAX_ACTIVITY_ITEMS)
          : next;
      threadActivityRef.current = capped;
      if (normalizedPubkey !== null) {
        writeActivityToStorage(normalizedPubkey, capped);
      }
      bumpLatestVersion();
    },
    [channels, normalizedPubkey, recordMentionedRoot],
  );

  const handleThreadReplyCandidate = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      const ref = getThreadReference(event.tags);
      const rootId = ref.rootId;
      if (rootId === null) {
        return;
      }

      void resolveThreadInterestFromRelay(rootId, channelId).then(
        (hasThreadInterest) => {
          if (!hasThreadInterest) {
            return;
          }

          if (
            !shouldNotifyForEvent(event, normalizedPubkey ?? "", {
              participatedRootIds: participatedRootIdsRef.current,
              followedRootIds: liveUpdateOptions.followedRootIds ?? EMPTY_SET,
              authoredRootIds: authoredRootIdsRef.current,
              mutedRootIds: mutedRootIdsRef.current,
              mutedChannelIds: mutedChannelIdsRef.current,
              channelId,
            })
          ) {
            return;
          }

          handleThreadReplyNotification(channelId, event);

          const channel = channelsRef.current.find(
            (entry) => entry.id === channelId,
          );
          if (
            channel?.channelType !== "dm" &&
            (channelId !== activeChannelId || notifyForActiveChannel)
          ) {
            callerOnThreadReplyDesktopNotification?.(channelId, event);
          }
        },
      );
    },
    [
      activeChannelId,
      callerOnThreadReplyDesktopNotification,
      handleThreadReplyNotification,
      liveUpdateOptions.followedRootIds,
      normalizedPubkey,
      notifyForActiveChannel,
      resolveThreadInterestFromRelay,
    ],
  );

  const muteThread = React.useCallback(
    (rootId: string) => {
      mutedRootIdsRef.current.add(rootId);
      if (normalizedPubkey !== null) {
        mutedStore.write(normalizedPubkey, mutedRootIdsRef.current);
      }
      bumpLatestVersion();
    },
    [normalizedPubkey],
  );

  const unmuteThread = React.useCallback(
    (rootId: string) => {
      mutedRootIdsRef.current.delete(rootId);
      if (normalizedPubkey !== null) {
        mutedStore.write(normalizedPubkey, mutedRootIdsRef.current);
      }
      bumpLatestVersion();
    },
    [normalizedPubkey],
  );

  useLiveChannelUpdates(channels, activeChannelId, {
    ...liveUpdateOptions,
    onChannelMessage: handleChannelMessage,
    onThreadReplyNotification: handleThreadReplyNotification,
    onThreadReplyCandidate: handleThreadReplyCandidate,
    onSelfChannelMessage: handleSelfChannelMessage,
    participatedRootIds: participatedRootIdsRef.current,
    followedRootIds: liveUpdateOptions.followedRootIds,
    authoredRootIds: authoredRootIdsRef.current,
    mutedRootIds: mutedRootIdsRef.current,
    mutedChannelIds: mutedChannelIdsRef.current,
  });

  // Effect-key the catch-up on the *set* of channel IDs, not the array
  // reference. React Query refetches return new array identities even when
  // the contents are unchanged; without this we'd cancel and never re-fire
  // every in-flight catch-up.
  const channelIdsKey = React.useMemo(
    () => [...new Set(channels.map((channel) => channel.id))].sort().join(","),
    [channels],
  );
  const followedRootIdsKey = React.useMemo(
    () =>
      [...(liveUpdateOptions.followedRootIds ?? EMPTY_SET)].sort().join(","),
    [liveUpdateOptions.followedRootIds],
  );

  React.useEffect(() => {
    if (
      !relayClient ||
      normalizedPubkey === null ||
      channelIdsKey.length === 0
    ) {
      return;
    }

    const backfillKey = `${normalizedPubkey}:${channelIdsKey}:${followedRootIdsKey}`;
    if (threadActivityBackfillKeyRef.current === backfillKey) {
      return;
    }
    threadActivityBackfillKeyRef.current = backfillKey;

    let isCancelled = false;
    let didFinish = false;
    const targetIds = channelIdsKey.split(",");
    const channelById = new Map(
      channels.map((channel) => [channel.id, channel]),
    );

    void (async () => {
      const participatedSizeBefore = participatedRootIdsRef.current.size;
      const authoredSizeBefore = authoredRootIdsRef.current.size;

      const [selfEvents, recentChannelEvents] = await Promise.all([
        relayClient
          .fetchEvents({
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            authors: [normalizedPubkey],
            "#h": targetIds,
            limit: THREAD_INTEREST_BACKFILL_LIMIT,
          })
          .catch(() => []),
        relayClient
          .fetchEvents({
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            "#h": targetIds,
            limit: THREAD_ACTIVITY_BACKFILL_LIMIT,
          })
          .catch(() => []),
      ]);

      if (isCancelled) {
        return;
      }

      const interestedRootIds = new Set<string>([
        ...participatedRootIdsRef.current,
        ...authoredRootIdsRef.current,
        ...(liveUpdateOptions.followedRootIds ?? EMPTY_SET),
      ]);

      for (const event of selfEvents) {
        interestedRootIds.add(
          recordSelfThreadInterest(
            event,
            participatedRootIdsRef.current,
            authoredRootIdsRef.current,
          ),
        );
      }

      if (normalizedPubkey !== null) {
        participationStore.write(
          normalizedPubkey,
          participatedRootIdsRef.current,
        );
        authoredStore.write(normalizedPubkey, authoredRootIdsRef.current);
      }

      const threadReplies: ThreadActivityItem[] = [];
      const candidateRepliesByChannel = new Map<string, RelayEvent[]>();
      const candidateRootIdsByChannel = new Map<string, Set<string>>();

      for (const event of recentChannelEvents) {
        if (event.pubkey.toLowerCase() === normalizedPubkey) {
          continue;
        }

        const ref = getThreadReference(event.tags);
        if (
          ref.parentId === null ||
          ref.rootId === null ||
          isBroadcastReply(event.tags)
        ) {
          continue;
        }

        const channelId = event.tags.find((tag) => tag[0] === "h")?.[1] ?? null;
        if (
          channelId === null ||
          mutedChannelIdsRef.current.has(channelId) ||
          mutedRootIdsRef.current.has(ref.rootId)
        ) {
          continue;
        }
        const readAt = getThreadActivityReadAt(channelId, ref.rootId);
        if (readAt !== null && event.created_at <= readAt) {
          continue;
        }

        const replies = candidateRepliesByChannel.get(channelId) ?? [];
        replies.push(event);
        candidateRepliesByChannel.set(channelId, replies);

        const roots = candidateRootIdsByChannel.get(channelId) ?? new Set();
        roots.add(ref.rootId);
        candidateRootIdsByChannel.set(channelId, roots);
      }

      await Promise.all(
        [...candidateRootIdsByChannel.entries()].map(([channelId, roots]) =>
          resolveThreadInterestsFromRelay([...roots], channelId),
        ),
      );

      if (isCancelled) {
        return;
      }

      for (const [channelId, events] of candidateRepliesByChannel) {
        const channelName = channelById.get(channelId)?.name ?? "";
        for (const event of events) {
          const ref = getThreadReference(event.tags);
          if (ref.rootId === null) {
            continue;
          }
          if (
            !shouldNotifyForEvent(event, normalizedPubkey, {
              participatedRootIds: participatedRootIdsRef.current,
              followedRootIds: liveUpdateOptions.followedRootIds ?? EMPTY_SET,
              authoredRootIds: authoredRootIdsRef.current,
              mutedRootIds: mutedRootIdsRef.current,
              mutedChannelIds: mutedChannelIdsRef.current,
              channelId,
            })
          ) {
            continue;
          }
          const readAt = getThreadActivityReadAt(channelId, ref.rootId);
          if (readAt !== null && event.created_at <= readAt) {
            continue;
          }

          threadReplies.push({
            id: event.id,
            kind: event.kind,
            pubkey: event.pubkey,
            content: event.content,
            createdAt: event.created_at,
            channelId,
            channelName,
            tags: [...event.tags],
          });
        }
      }

      const rootIds = [...interestedRootIds].filter(
        (rootId) => !mutedRootIdsRef.current.has(rootId),
      );
      if (rootIds.length === 0) {
        const added = addThreadActivityItems(
          threadActivityRef.current,
          threadReplies,
        );
        if (added.didAdd) {
          threadActivityRef.current = added.items;
          writeActivityToStorage(normalizedPubkey, added.items);
          bumpLatestVersion();
        }

        if (
          participatedRootIdsRef.current.size !== participatedSizeBefore ||
          authoredRootIdsRef.current.size !== authoredSizeBefore
        ) {
          bumpMembershipVersion();
        }

        return;
      }
      for (
        let start = 0;
        start < rootIds.length &&
        threadReplies.length < THREAD_ACTIVITY_BACKFILL_LIMIT;
        start += THREAD_ACTIVITY_ROOT_BATCH_SIZE
      ) {
        const rootBatch = rootIds.slice(
          start,
          start + THREAD_ACTIVITY_ROOT_BATCH_SIZE,
        );
        const events = await relayClient
          .fetchEvents({
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            "#e": rootBatch,
            "#h": targetIds,
            limit: THREAD_ACTIVITY_BACKFILL_LIMIT,
          })
          .catch(() => []);

        if (isCancelled) {
          return;
        }

        for (const event of events) {
          if (event.pubkey.toLowerCase() === normalizedPubkey) {
            continue;
          }

          const ref = getThreadReference(event.tags);
          if (
            ref.parentId === null ||
            ref.rootId === null ||
            !interestedRootIds.has(ref.rootId) ||
            isBroadcastReply(event.tags)
          ) {
            continue;
          }

          const channelId =
            event.tags.find((tag) => tag[0] === "h")?.[1] ?? null;
          if (
            channelId === null ||
            mutedChannelIdsRef.current.has(channelId) ||
            mutedRootIdsRef.current.has(ref.rootId)
          ) {
            continue;
          }
          const readAt = getThreadActivityReadAt(channelId, ref.rootId);
          if (readAt !== null && event.created_at <= readAt) {
            continue;
          }

          const channelName = channelById.get(channelId)?.name ?? "";
          threadReplies.push({
            id: event.id,
            kind: event.kind,
            pubkey: event.pubkey,
            content: event.content,
            createdAt: event.created_at,
            channelId,
            channelName,
            tags: [...event.tags],
          });
        }
      }

      const added = addThreadActivityItems(
        threadActivityRef.current,
        threadReplies,
      );
      if (added.didAdd) {
        threadActivityRef.current = added.items;
        writeActivityToStorage(normalizedPubkey, added.items);
        bumpLatestVersion();
      }

      if (
        participatedRootIdsRef.current.size !== participatedSizeBefore ||
        authoredRootIdsRef.current.size !== authoredSizeBefore
      ) {
        bumpMembershipVersion();
      }
    })().finally(() => {
      if (!isCancelled) {
        didFinish = true;
      }
    });

    return () => {
      isCancelled = true;
      if (!didFinish && threadActivityBackfillKeyRef.current === backfillKey) {
        threadActivityBackfillKeyRef.current = null;
      }
    };
  }, [
    channelIdsKey,
    channels,
    followedRootIdsKey,
    getThreadActivityReadAt,
    liveUpdateOptions.followedRootIds,
    normalizedPubkey,
    relayClient,
    resolveThreadInterestsFromRelay,
  ]);

  // Catch-up: for each channel we haven't already caught up this session,
  // ask the relay "are there any external trigger messages newer than the
  // NIP-RS read marker?" If yes, advance latestByChannelRef so the unread
  // predicate fires. This is the only way historical unreads survive an
  // app restart now that we don't persist any client-side "latest" state.
  // biome-ignore lint/correctness/useExhaustiveDependencies: options.followedRootIds intentionally omitted — it's a Set reference that changes identity every render; the catch-up is a one-shot per-channel operation controlled by caughtUpChannelsRef, not reactive to follow changes
  React.useEffect(() => {
    if (!isReadStateReady) return;
    if (!relayClient) return;
    if (channelIdsKey.length === 0) return;

    const targetIds = channelIdsKey.split(",");
    const toFetch = targetIds.filter(
      (id) => !caughtUpChannelsRef.current.has(id),
    );
    if (toFetch.length === 0) return;

    // Claim optimistically so re-renders mid-flight don't kick off duplicate
    // REQs. If the effect is cancelled (cleanup) we release the claims so
    // the next run retries.
    for (const id of toFetch) {
      caughtUpChannelsRef.current.add(id);
    }

    let isCancelled = false;

    // Snapshot membership sizes so the `.then` can detect whether the catch-up
    // discovered new participated/authored/mentioned roots (pass 1 mutates the
    // refs in place). A pure-participation or pure-mention discovery produces no
    // maxExternal advance, so without this the notify gate would never
    // invalidate to surface the badge.
    const participatedSizeBefore = participatedRootIdsRef.current.size;
    const authoredSizeBefore = authoredRootIdsRef.current.size;
    const mentionedSizeBefore = mentionedRootIdsRef.current.size;

    type CatchUpResult =
      | {
          channelId: string;
          ok: true;
          mainChannelEvents: Array<{ createdAt: number; id: string }>;
          maxExternal: number;
          maxHighPriority: number;
          threadReplies: ThreadActivityItem[];
        }
      | { channelId: string; ok: false };

    void Promise.all(
      toFetch.map(async (channelId): Promise<CatchUpResult> => {
        try {
          const readAt = getEffectiveTimestamp(channelId);
          // NIP-01 `since` is inclusive of `created_at >= since`. The +1
          // makes the relay-side filter strict-newer; the client-side
          // `> readAt` check below is the belt to the suspenders.
          const sinceParam = readAt === null ? 0 : readAt + 1;

          const events = await relayClient.fetchEvents({
            kinds: [...CHANNEL_MESSAGE_EVENT_KINDS],
            "#h": [channelId],
            since: sinceParam,
            limit: CATCH_UP_LIMIT,
          });

          // Pass 1: build participation from self-authored thread replies,
          // track self-authored top-level messages for author notifications,
          // and capture external mentions so their threads gate a badge.
          for (const event of events) {
            const isSelf =
              normalizedPubkey !== null &&
              event.pubkey.toLowerCase() === normalizedPubkey;
            if (isSelf) {
              const rootId = recordSelfThreadInterest(
                event,
                participatedRootIdsRef.current,
                authoredRootIdsRef.current,
              );
              const eventChannelId = event.tags.find(
                (tag) => tag[0] === "h",
              )?.[1];
              if (eventChannelId) {
                threadInterestResolutionsRef.current.delete(
                  threadInterestResolutionKey(eventChannelId, rootId),
                );
              }
            } else {
              recordMentionedRoot(event);
            }
          }

          if (normalizedPubkey !== null) {
            participationStore.write(
              normalizedPubkey,
              participatedRootIdsRef.current,
            );
            authoredStore.write(normalizedPubkey, authoredRootIdsRef.current);
          }

          if (normalizedPubkey !== null) {
            const rootIdsToBackfill = new Set<string>();
            for (const event of events) {
              if (
                event.pubkey.toLowerCase() === normalizedPubkey ||
                (readAt !== null && event.created_at <= readAt)
              ) {
                continue;
              }

              const evtRef = getThreadReference(event.tags);
              if (
                evtRef.parentId === null ||
                evtRef.rootId === null ||
                isBroadcastReply(event.tags)
              ) {
                continue;
              }

              const notifyChannelId =
                event.tags.find((t) => t[0] === "h")?.[1] ?? channelId;
              if (
                shouldNotifyForEvent(event, normalizedPubkey, {
                  participatedRootIds: participatedRootIdsRef.current,
                  followedRootIds: options.followedRootIds ?? EMPTY_SET,
                  authoredRootIds: authoredRootIdsRef.current,
                  mutedRootIds: mutedRootIdsRef.current,
                  mutedChannelIds: mutedChannelIdsRef.current,
                  channelId: notifyChannelId,
                })
              ) {
                continue;
              }

              rootIdsToBackfill.add(evtRef.rootId);
            }

            await resolveThreadInterestsFromRelay(
              [...rootIdsToBackfill],
              channelId,
            );
          }

          // Pass 2: compute the newest main-channel unread event and collect
          // thread reply activity, applying the notification filter to both.
          let maxExternal = 0;
          let maxHighPriority = 0;
          const mainChannelEvents: Array<{ createdAt: number; id: string }> =
            [];
          const threadReplies: ThreadActivityItem[] = [];
          const ch = channels.find((c) => c.id === channelId);
          const chType = ch?.channelType;
          const chName = ch?.name ?? "";
          for (const event of events) {
            if (
              normalizedPubkey !== null &&
              event.pubkey.toLowerCase() === normalizedPubkey
            ) {
              continue;
            }
            if (readAt !== null && event.created_at <= readAt) continue;
            const eventChannelId =
              event.tags.find((t) => t[0] === "h")?.[1] ?? null;
            const notifyChannelId = eventChannelId ?? channelId;
            const evtRef = getThreadReference(event.tags);
            if (
              !shouldNotifyForEvent(event, normalizedPubkey ?? "", {
                participatedRootIds: participatedRootIdsRef.current,
                followedRootIds: options.followedRootIds ?? EMPTY_SET,
                authoredRootIds: authoredRootIdsRef.current,
                mutedRootIds: mutedRootIdsRef.current,
                mutedChannelIds: mutedChannelIdsRef.current,
                channelId: notifyChannelId,
              })
            ) {
              continue;
            }
            const isThreadedReply =
              evtRef.parentId !== null && !isBroadcastReply(event.tags);
            if (!isThreadedReply) {
              mainChannelEvents.push({
                id: event.id,
                createdAt: event.created_at,
              });
              if (event.created_at > maxExternal) {
                maxExternal = event.created_at;
              }
              if (
                chType === "dm" ||
                (normalizedPubkey !== null &&
                  isHighPriorityEventForUser(event, normalizedPubkey))
              ) {
                if (event.created_at > maxHighPriority) {
                  maxHighPriority = event.created_at;
                }
              }
            }
            if (isThreadedReply) {
              const rootId = evtRef.rootId ?? evtRef.parentId;
              if (rootId) {
                const threadReadAt = getThreadActivityReadAt(channelId, rootId);
                if (threadReadAt !== null && event.created_at <= threadReadAt) {
                  continue;
                }
              }
              threadReplies.push({
                id: event.id,
                kind: event.kind,
                pubkey: event.pubkey,
                content: event.content,
                createdAt: event.created_at,
                channelId,
                channelName: chName,
                tags: [...event.tags],
              });
            }
          }

          return {
            channelId,
            ok: true,
            mainChannelEvents,
            maxExternal,
            maxHighPriority,
            threadReplies,
          };
        } catch {
          // Transient relay failure for this channel — release the claim
          // so we retry on the next effect run instead of staying stuck
          // until identity reset.
          return { channelId, ok: false };
        }
      }),
    ).then((results) => {
      if (isCancelled) return;
      let didAdvance = false;
      const allThreadReplies: ThreadActivityItem[] = [];
      for (const result of results) {
        if (!result.ok) {
          caughtUpChannelsRef.current.delete(result.channelId);
          continue;
        }
        const {
          channelId,
          mainChannelEvents,
          maxExternal,
          maxHighPriority,
          threadReplies,
        } = result;
        allThreadReplies.push(...threadReplies);
        for (const event of mainChannelEvents) {
          if (
            addUnreadChannelEvent(
              unreadEventsByChannelRef.current,
              channelId,
              event.id,
              event.createdAt,
            )
          ) {
            didAdvance = true;
          }
        }
        if (maxExternal > 0) {
          const readAtNow = getEffectiveTimestamp(channelId) ?? 0;
          if (maxExternal > readAtNow) {
            const current = latestByChannelRef.current.get(channelId) ?? 0;
            if (maxExternal > current) {
              latestByChannelRef.current.set(channelId, maxExternal);
              didAdvance = true;
            }
          }
        }
        if (maxHighPriority > 0) {
          const readAtNow = getEffectiveTimestamp(channelId) ?? 0;
          if (maxHighPriority > readAtNow) {
            const currentHigh =
              latestHighPriorityByChannelRef.current.get(channelId) ?? 0;
            if (maxHighPriority > currentHigh) {
              latestHighPriorityByChannelRef.current.set(
                channelId,
                maxHighPriority,
              );
              didAdvance = true;
            }
          }
        }
      }
      if (allThreadReplies.length > 0) {
        const existingIds = new Set(threadActivityRef.current.map((e) => e.id));
        const newItems = allThreadReplies.filter(
          (item) => !existingIds.has(item.id),
        );
        if (newItems.length > 0) {
          const merged = [...threadActivityRef.current, ...newItems];
          const capped =
            merged.length > MAX_ACTIVITY_ITEMS
              ? merged.slice(merged.length - MAX_ACTIVITY_ITEMS)
              : merged;
          threadActivityRef.current = capped;
          if (normalizedPubkey) {
            writeActivityToStorage(normalizedPubkey, capped);
          }
          didAdvance = true;
        }
      }
      if (didAdvance) bumpLatestVersion();
      if (
        participatedRootIdsRef.current.size !== participatedSizeBefore ||
        authoredRootIdsRef.current.size !== authoredSizeBefore ||
        mentionedRootIdsRef.current.size !== mentionedSizeBefore
      ) {
        bumpMembershipVersion();
      }
    });

    return () => {
      isCancelled = true;
      // Release the claims so the next effect run can retry these channels.
      // The identity-reset effect replaces the Set entirely, so this is a
      // no-op in that case (harmless).
      for (const id of toFetch) {
        caughtUpChannelsRef.current.delete(id);
      }
    };
  }, [
    channelIdsKey,
    getEffectiveTimestamp,
    isReadStateReady,
    normalizedPubkey,
    relayClient,
    resolveThreadInterestsFromRelay,
  ]);

  // Unread = channels (excluding active) that have either been manually
  // marked unread this session, or whose observed latest external main-channel
  // timestamp is strictly newer than their NIP-RS read marker.
  // High-priority unread = DMs or channels with a mention/broadcast newer
  // than the read marker. Forced-unread channels are dot tier only (not
  // high-priority). Both sets share identical deps and always invalidate
  // together, so they are computed in a single memo.
  const rawUnread =
    // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion and latestVersion are intentional invalidation signals
    React.useMemo(() => {
      if (!isReadStateReady) {
        return {
          unreadChannelIds: new Set<string>(),
          highPriorityUnreadChannelIds: new Set<string>(),
          unreadChannelNotificationCount: 0,
        };
      }

      const unread = new Set<string>();
      const highPriority = new Set<string>();
      let notificationCount = 0;

      for (const channel of channels) {
        if (channel.id === activeChannelId) continue;

        if (forcedUnreadRef.current.has(channel.id)) {
          // Forced-unread is a manual notification bucket; there is no event
          // count to recover, so count the channel once.
          unread.add(channel.id);
          notificationCount += 1;
          continue;
        }

        const latest = latestByChannelRef.current.get(channel.id);
        if (latest === undefined) continue;

        const readAt = getEffectiveTimestamp(channel.id);
        if (readAt !== null && latest <= readAt) continue;

        unread.add(channel.id);
        const channelEvents = unreadEventsByChannelRef.current.get(channel.id);
        const unreadEventCount =
          channelEvents === undefined
            ? 0
            : [...channelEvents.values()].filter(
                (createdAt) => readAt === null || createdAt > readAt,
              ).length;
        notificationCount += Math.max(1, unreadEventCount);

        // DM channels: any unread DM is high-priority.
        if (channel.channelType === "dm") {
          highPriority.add(channel.id);
        } else {
          // Non-DM: high-priority only if there's a mention/broadcast newer than read marker.
          const latestHigh = latestHighPriorityByChannelRef.current.get(
            channel.id,
          );
          if (
            latestHigh !== undefined &&
            (readAt === null || latestHigh > readAt)
          ) {
            highPriority.add(channel.id);
          }
        }
      }

      return {
        unreadChannelIds: unread,
        highPriorityUnreadChannelIds: highPriority,
        unreadChannelNotificationCount: notificationCount,
      };
    }, [
      activeChannelId,
      channels,
      getEffectiveTimestamp,
      isReadStateReady,
      latestVersion,
      readStateVersion,
    ]);

  // Stabilize Set references: only replace when contents actually change,
  // so downstream memos don't re-run on every render when sets are equal.
  const prevUnreadRef = React.useRef<ReadonlySet<string>>(new Set());
  const prevHighPriorityRef = React.useRef<ReadonlySet<string>>(new Set());

  const unreadChannelIds = setsEqual(
    rawUnread.unreadChannelIds,
    prevUnreadRef.current,
  )
    ? prevUnreadRef.current
    : rawUnread.unreadChannelIds;
  prevUnreadRef.current = unreadChannelIds;

  const highPriorityUnreadChannelIds = setsEqual(
    rawUnread.highPriorityUnreadChannelIds,
    prevHighPriorityRef.current,
  )
    ? prevHighPriorityRef.current
    : rawUnread.highPriorityUnreadChannelIds;
  prevHighPriorityRef.current = highPriorityUnreadChannelIds;
  const unreadChannelNotificationCount =
    rawUnread.unreadChannelNotificationCount;

  const unreadChannelIdsRef = React.useRef(unreadChannelIds);
  unreadChannelIdsRef.current = unreadChannelIds;

  const markAllChannelsRead = React.useCallback(() => {
    for (const channelId of unreadChannelIdsRef.current) {
      forcedUnreadRef.current.delete(channelId);
      const unixSeconds =
        latestByChannelRef.current.get(channelId) ??
        getEffectiveTimestamp(channelId) ??
        null;
      if (unixSeconds !== null) {
        markContextRead(channelId, unixSeconds);
      }
      latestByChannelRef.current.delete(channelId);
      latestHighPriorityByChannelRef.current.delete(channelId);
      unreadEventsByChannelRef.current.delete(channelId);
    }
    bumpLatestVersion();
  }, [getEffectiveTimestamp, markContextRead]);

  // Identity-stable snapshots of the membership sets for the notify gate.
  // Re-derived only when membershipVersion bumps (a set actually changed), so
  // `isNotifiedForThread`'s useCallback deps invalidate on async discovery
  // while live consumers keep reading the mutable refs directly.
  // biome-ignore lint/correctness/useExhaustiveDependencies: membershipVersion is the intentional re-derivation signal
  const participatedRootIds = React.useMemo(
    () => new Set(participatedRootIdsRef.current) as ReadonlySet<string>,
    [membershipVersion],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: membershipVersion is the intentional re-derivation signal
  const authoredRootIds = React.useMemo(
    () => new Set(authoredRootIdsRef.current) as ReadonlySet<string>,
    [membershipVersion],
  );
  // biome-ignore lint/correctness/useExhaustiveDependencies: membershipVersion is the intentional re-derivation signal
  const mentionedRootIds = React.useMemo(
    () => new Set(mentionedRootIdsRef.current) as ReadonlySet<string>,
    [membershipVersion],
  );

  return {
    unreadChannelIds,
    highPriorityUnreadChannelIds,
    unreadChannelNotificationCount,
    markAllChannelsRead,
    markChannelRead,
    markChannelUnread,
    // Exposed so other surfaces (e.g. Home) can project per-item read state
    // off the same NIP-RS read marker without instantiating a second
    // ReadStateManager. readStateVersion is the invalidation signal callers
    // should include in memo deps.
    getEffectiveTimestamp,
    getOwnTimestamp,
    readStateVersion,
    setContextParentResolver,
    participatedRootIds,
    authoredRootIds,
    mentionedRootIds,
    threadActivityItems: threadActivityRef.current,
    mutedRootIds: mutedRootIdsRef.current as ReadonlySet<string>,
    muteThread,
    unmuteThread,
  };
}
