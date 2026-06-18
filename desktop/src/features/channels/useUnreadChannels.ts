import * as React from "react";
import {
  EMPTY_SET,
  useLiveChannelUpdates,
  type UseLiveChannelUpdatesOptions,
} from "@/features/channels/useLiveChannelUpdates";
import {
  buildChannelThreadRoots,
  channelUnreadFrontier,
  countUnreadObservedEvents,
  mapsEqual,
  recordObservedUnreadEvent,
  type ObservedUnreadEvent,
} from "@/features/channels/unreadChannelCounts";
import { useReadState } from "@/features/channels/readState/useReadState";
import {
  getThreadReference,
  isBroadcastReply,
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
// Folds the caller's timeline position together with the newest event this
// client has observed live (`observedLatest`), so an explicit "mark read" still
// covers messages that arrived faster than channel metadata — this fold is
// load-bearing for the Esc shortcut, sidebar mark-read, and empty-channel open,
// all of which pass a null/stale caller value. `clearObserved` reports whether
// the resulting marker covers the observed timestamp, signalling the caller to
// drop its observed refs so the unread memo sees `latest === undefined` until a
// genuinely newer event arrives.
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
    isReady: isReadStateReady,
    markContextRead,
    drainSyncedAdvances,
    setContextParentResolver,
    readStateVersion,
    getOwnTimestamp,
  } = useReadState(pubkey, relayClient);

  // Observed "latest external trigger event" per channel (unix seconds). This
  // is *derived relay evidence*, not source-of-truth: it's populated from a
  // one-shot catch-up REQ per channel (keyed on the NIP-RS read marker) plus
  // ongoing live events. The only thing we ever do with it is compare against
  // the NIP-RS read marker — see the unread memo below. Reset on identity
  // change. Stale entries for channels the user has left are silently
  // ignored by the memo (it iterates the current channels list, not the map).
  const latestByChannelRef = React.useRef(new Map<string, number>());
  const observedUnreadEventsByChannelRef = React.useRef(
    new Map<string, Map<string, number>>(),
  );
  const latestHighPriorityByChannelRef = React.useRef(
    new Map<string, number>(),
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

  // Reset all in-session state when the identity or relay changes. Unread
  // tracking depends only on NIP-RS read markers + observed relay events for
  // this user; nothing here is persisted across restarts.
  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey/relayClient are intentional reset signals
  React.useEffect(() => {
    latestByChannelRef.current = new Map();
    observedUnreadEventsByChannelRef.current = new Map();
    latestHighPriorityByChannelRef.current = new Map();
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
    bumpLatestVersion();
    bumpMembershipVersion();
  }, [pubkey, relayClient]);

  // `topLevelOnly` is the passive channel-open path (NIP-RS Option 1): the
  // caller's `readAt` is already the newest TOP-LEVEL message, so the marker
  // must land exactly there without folding in `observedLatest` (which counts
  // thread replies) and without clearing observed refs. Leaving the refs intact
  // keeps the sidebar dot lit for a channel whose only unread is an unopened
  // thread reply — viewing the channel no longer absorbs the reply, so the dot
  // persists until an explicit mark-read (Esc, sidebar, mark-all) or a newer
  // top-level message advances the channel marker past it. Those explicit
  // "mark read" actions omit the flag and keep the fold, since they mean
  // "clear everything in this channel."
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
      // Clear observed-latest refs when the read marker covers them so the
      // unread memo sees `latest === undefined` until a genuinely new event
      // arrives. Without this, `latest > readAt` resolves to `T > T` (false)
      // but the channel lingers in the set when advanceContext's monotonic
      // guard suppresses the readStateVersion bump.
      if (clearObserved) {
        latestByChannelRef.current.delete(channelId);
        observedUnreadEventsByChannelRef.current.delete(channelId);
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

  // Feed the in-session "latest external trigger" map from live channel
  // events. Composes with any caller-supplied onChannelMessage handler.
  // useLiveChannelUpdates already filters this callback to trigger kinds
  // and external authors, so the map is always a strict subset of "newest
  // external trigger message this client has observed."
  const callerOnChannelMessage = liveUpdateOptions.onChannelMessage;
  const recordUnreadEvent = React.useCallback(
    (channelId: string, event: ObservedUnreadEvent) => {
      recordObservedUnreadEvent(
        observedUnreadEventsByChannelRef.current,
        channelId,
        event,
        CATCH_UP_LIMIT,
      );
    },
    [],
  );
  const handleChannelMessage = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      recordUnreadEvent(channelId, {
        id: event.id,
        createdAt: event.created_at,
      });
      const current = latestByChannelRef.current.get(channelId) ?? 0;
      if (event.created_at > current) {
        latestByChannelRef.current.set(channelId, event.created_at);
        bumpLatestVersion();
      }

      // A mention on a reply makes its thread badge-eligible even when the
      // user never participated/authored/followed (the gate's missing term).
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
    [
      callerOnChannelMessage,
      normalizedPubkey,
      recordMentionedRoot,
      recordUnreadEvent,
    ],
  );

  const handleSelfChannelMessage = React.useCallback(
    (event: RelayEvent) => {
      const ref = getThreadReference(event.tags);
      // Participation roots key on the thread root; authored roots (no thread
      // ref) key on the event id itself.
      const isParticipation = ref.rootId !== null;
      const targetSet = isParticipation
        ? participatedRootIdsRef.current
        : authoredRootIdsRef.current;
      const sizeBefore = targetSet.size;
      targetSet.add(ref.rootId ?? event.id);
      if (normalizedPubkey !== null) {
        const write = isParticipation
          ? participationStore.write
          : authoredStore.write;
        write(normalizedPubkey, targetSet);
      }
      // Only re-derive the gate snapshot when the set actually grew; a self-post
      // to an already-tracked root is a no-op for the notify gate, so skipping
      // the bump avoids a wasted snapshot re-allocation + gate recompute.
      if (targetSet.size !== sizeBefore) {
        bumpMembershipVersion();
      }
      bumpLatestVersion();
    },
    [normalizedPubkey],
  );

  const handleThreadReplyNotification = React.useCallback(
    (channelId: string, event: RelayEvent) => {
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
    [channels, normalizedPubkey],
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
          maxExternal: number;
          maxHighPriority: number;
          unreadEvents: ObservedUnreadEvent[];
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
              const ref = getThreadReference(event.tags);
              if (ref.rootId !== null) {
                participatedRootIdsRef.current.add(ref.rootId);
              } else {
                authoredRootIdsRef.current.add(event.id);
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

          // Pass 2: compute maxExternal and collect thread reply activity,
          // applying the notification filter to both.
          let maxExternal = 0;
          let maxHighPriority = 0;
          const unreadEvents: ObservedUnreadEvent[] = [];
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
            if (
              !shouldNotifyForEvent(event, normalizedPubkey ?? "", {
                participatedRootIds: participatedRootIdsRef.current,
                followedRootIds: options.followedRootIds ?? EMPTY_SET,
                authoredRootIds: authoredRootIdsRef.current,
                mutedRootIds: mutedRootIdsRef.current,
                mutedChannelIds: mutedChannelIdsRef.current,
                channelId: eventChannelId,
              })
            ) {
              continue;
            }
            if (event.created_at > maxExternal) {
              maxExternal = event.created_at;
            }
            unreadEvents.push({ id: event.id, createdAt: event.created_at });
            if (
              chType === "dm" ||
              (normalizedPubkey !== null &&
                isHighPriorityEventForUser(event, normalizedPubkey))
            ) {
              if (event.created_at > maxHighPriority) {
                maxHighPriority = event.created_at;
              }
            }
            const evtRef = getThreadReference(event.tags);
            if (evtRef.parentId !== null && !isBroadcastReply(event.tags)) {
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
            maxExternal,
            maxHighPriority,
            unreadEvents,
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
          maxExternal,
          maxHighPriority,
          unreadEvents,
          threadReplies,
        } = result;
        allThreadReplies.push(...threadReplies);
        if (unreadEvents.length > 0) {
          for (const event of unreadEvents) {
            recordUnreadEvent(channelId, event);
          }
          didAdvance = true;
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
    recordUnreadEvent,
    relayClient,
  ]);

  // Unread = channels (excluding active) that have either been manually
  // marked unread this session, or whose observed latest external trigger
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
          unreadChannelCounts: new Map<string, number>(),
        };
      }

      const unread = new Set<string>();
      const highPriority = new Set<string>();
      const counts = new Map<string, number>();

      // Map each channel to the thread roots observed in it, so a channel's
      // frontier can fold in per-thread read markers (Option A): opening a
      // thread advances thread:<root> and must clear the channel dot even
      // though markChannelRead only advances the channel marker to the newest
      // TOP-LEVEL message.
      const threadRootsByChannel = buildChannelThreadRoots(
        threadActivityRef.current,
        (tags) => getThreadReference(tags).rootId,
      );

      for (const channel of channels) {
        if (channel.id === activeChannelId) continue;

        if (forcedUnreadRef.current.has(channel.id)) {
          // Forced-unread is dot tier only — not high-priority.
          unread.add(channel.id);
          counts.set(channel.id, 1);
          continue;
        }

        const latest = latestByChannelRef.current.get(channel.id);
        if (latest === undefined) continue;

        const readAt = channelUnreadFrontier(
          getEffectiveTimestamp(channel.id),
          threadRootsByChannel.get(channel.id),
          (rootId) => getOwnTimestamp(`thread:${rootId}`),
        );
        if (readAt !== null && latest <= readAt) continue;

        unread.add(channel.id);
        const observedEvents = observedUnreadEventsByChannelRef.current.get(
          channel.id,
        );
        const unreadCount = countUnreadObservedEvents(observedEvents, readAt);
        counts.set(channel.id, Math.max(unreadCount, 1));

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
        unreadChannelCounts: counts,
      };
    }, [
      activeChannelId,
      channels,
      getEffectiveTimestamp,
      getOwnTimestamp,
      isReadStateReady,
      latestVersion,
      readStateVersion,
    ]);

  // Stabilize Set references: only replace when contents actually change,
  // so downstream memos don't re-run on every render when sets are equal.
  const prevUnreadRef = React.useRef<ReadonlySet<string>>(new Set());
  const prevHighPriorityRef = React.useRef<ReadonlySet<string>>(new Set());
  const prevUnreadCountsRef = React.useRef<ReadonlyMap<string, number>>(
    new Map(),
  );

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

  const unreadChannelCounts = mapsEqual(
    rawUnread.unreadChannelCounts,
    prevUnreadCountsRef.current,
  )
    ? prevUnreadCountsRef.current
    : rawUnread.unreadChannelCounts;
  prevUnreadCountsRef.current = unreadChannelCounts;

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
    unreadChannelCounts,
    highPriorityUnreadChannelIds,
    markAllChannelsRead,
    markChannelRead,
    markChannelUnread,
    // Exposed so other surfaces (e.g. Home) can project per-item read state
    // off the same NIP-RS read marker without instantiating a second
    // ReadStateManager. readStateVersion is the invalidation signal callers
    // should include in memo deps.
    getEffectiveTimestamp,
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
