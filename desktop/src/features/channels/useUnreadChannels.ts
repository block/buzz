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
} from "@/features/messages/lib/threading";
import {
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

const PARTICIPATION_STORAGE_PREFIX = "sprout-thread-participation.v1";
const MAX_PARTICIPATION_ENTRIES = 1000;

function participationStorageKey(pubkey: string): string {
  return `${PARTICIPATION_STORAGE_PREFIX}:${pubkey}`;
}

function readParticipationFromStorage(pubkey: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(participationStorageKey(pubkey));
    if (!raw) {
      return new Set();
    }
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return new Set();
    }
    return new Set(parsed.filter((id): id is string => typeof id === "string"));
  } catch {
    return new Set();
  }
}

function writeParticipationToStorage(
  pubkey: string,
  rootIds: Set<string>,
): void {
  try {
    const arr = [...rootIds];
    const capped =
      arr.length > MAX_PARTICIPATION_ENTRIES
        ? arr.slice(arr.length - MAX_PARTICIPATION_ENTRIES)
        : arr;
    window.localStorage.setItem(
      participationStorageKey(pubkey),
      JSON.stringify(capped),
    );
  } catch {
    // Ignore storage errors (private browsing, quota exceeded).
  }
}

const AUTHORED_STORAGE_PREFIX = "sprout-thread-authored.v1";
const MAX_AUTHORED_ENTRIES = 1000;

function authoredStorageKey(pubkey: string): string {
  return `${AUTHORED_STORAGE_PREFIX}:${pubkey}`;
}

function readAuthoredFromStorage(pubkey: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(authoredStorageKey(pubkey));
    if (!raw) {
      return new Set();
    }
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return new Set();
    }
    return new Set(parsed.filter((id): id is string => typeof id === "string"));
  } catch {
    return new Set();
  }
}

function writeAuthoredToStorage(pubkey: string, rootIds: Set<string>): void {
  try {
    const arr = [...rootIds];
    const capped =
      arr.length > MAX_AUTHORED_ENTRIES
        ? arr.slice(arr.length - MAX_AUTHORED_ENTRIES)
        : arr;
    window.localStorage.setItem(
      authoredStorageKey(pubkey),
      JSON.stringify(capped),
    );
  } catch {
    // Ignore storage errors (private browsing, quota exceeded).
  }
}

const MUTED_STORAGE_PREFIX = "sprout-thread-muted.v1";
const MAX_MUTED_ENTRIES = 1000;

function mutedStorageKey(pubkey: string): string {
  return `${MUTED_STORAGE_PREFIX}:${pubkey}`;
}

function readMutedFromStorage(pubkey: string): Set<string> {
  try {
    const raw = window.localStorage.getItem(mutedStorageKey(pubkey));
    if (!raw) return new Set();
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return new Set();
    return new Set(parsed.filter((id): id is string => typeof id === "string"));
  } catch {
    return new Set();
  }
}

function writeMutedToStorage(pubkey: string, rootIds: Set<string>): void {
  try {
    const arr = [...rootIds];
    const capped =
      arr.length > MAX_MUTED_ENTRIES
        ? arr.slice(arr.length - MAX_MUTED_ENTRIES)
        : arr;
    window.localStorage.setItem(
      mutedStorageKey(pubkey),
      JSON.stringify(capped),
    );
  } catch {
    // Ignore storage errors (private browsing, quota exceeded).
  }
}

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

const ACTIVITY_STORAGE_PREFIX = "sprout-thread-activity.v1";
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
  activeReadAt?: string | null,
  options: UseUnreadChannelsOptions = {},
) {
  const {
    pubkey,
    relayClient,
    mutedChannelIds: mutedChannelIdsOption,
    ...liveUpdateOptions
  } = options;
  const activeChannelId = activeChannel?.id ?? null;
  const activeChannelLastMessageAt = activeChannel?.lastMessageAt ?? null;
  const normalizedPubkey = pubkey?.toLowerCase() ?? null;

  // Let callers pass `null` to intentionally suppress the optimistic
  // channel-metadata fallback until a real timeline position is known.
  const effectiveActiveReadAt =
    activeReadAt === undefined ? activeChannelLastMessageAt : activeReadAt;

  const {
    getEffectiveTimestamp,
    isReady: isReadStateReady,
    markContextRead,
    drainSyncedAdvances,
    readStateVersion,
  } = useReadState(pubkey, relayClient);

  // Observed "latest external trigger event" per channel (unix seconds). This
  // is *derived relay evidence*, not source-of-truth: it's populated from a
  // one-shot catch-up REQ per channel (keyed on the NIP-RS read marker) plus
  // ongoing live events. The only thing we ever do with it is compare against
  // the NIP-RS read marker — see the unread memo below. Reset on identity
  // change. Stale entries for channels the user has left are silently
  // ignored by the memo (it iterates the current channels list, not the map).
  const latestByChannelRef = React.useRef(new Map<string, number>());
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

  // Reset all in-session state when the identity or relay changes. Unread
  // tracking depends only on NIP-RS read markers + observed relay events for
  // this user; nothing here is persisted across restarts.
  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey/relayClient are intentional reset signals
  React.useEffect(() => {
    latestByChannelRef.current = new Map();
    latestHighPriorityByChannelRef.current = new Map();
    forcedUnreadRef.current = new Set();
    caughtUpChannelsRef.current = new Set();
    participatedRootIdsRef.current = pubkey
      ? readParticipationFromStorage(pubkey)
      : new Set();
    authoredRootIdsRef.current = pubkey
      ? readAuthoredFromStorage(pubkey)
      : new Set();
    mutedRootIdsRef.current = pubkey ? readMutedFromStorage(pubkey) : new Set();
    threadActivityRef.current = pubkey ? readActivityFromStorage(pubkey) : [];
    bumpLatestVersion();
  }, [pubkey, relayClient]);

  const markChannelRead = React.useCallback(
    (channelId: string, readAt: string | null | undefined) => {
      if (forcedUnreadRef.current.delete(channelId)) {
        bumpLatestVersion();
      }
      const callerUnix = toUnixSeconds(readAt);
      const observedLatest = latestByChannelRef.current.get(channelId);
      const unixSeconds =
        Math.max(callerUnix ?? 0, observedLatest ?? 0) || null;
      if (unixSeconds === null) return;
      markContextRead(channelId, unixSeconds);
      // Clear observed-latest refs when the read marker covers them so the
      // unread memo sees `latest === undefined` until a genuinely new event
      // arrives. Without this, `latest > readAt` resolves to `T > T` (false)
      // but the channel lingers in the set when advanceContext's monotonic
      // guard suppresses the readStateVersion bump.
      if (observedLatest !== undefined && observedLatest <= unixSeconds) {
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

  // Mark the active channel as read when it changes or new messages arrive.
  // Honours the caller's contract that a null activeReadAt suppresses
  // read-marking until the timeline reports a real position. Manual
  // mark-unread state is cleared inside markChannelRead, not here.
  React.useEffect(() => {
    if (!isReadStateReady) return;
    if (!activeChannelId) return;
    markChannelRead(activeChannelId, effectiveActiveReadAt);
  }, [
    activeChannelId,
    effectiveActiveReadAt,
    isReadStateReady,
    markChannelRead,
  ]);

  // Feed the in-session "latest external trigger" map from live channel
  // events. Composes with any caller-supplied onChannelMessage handler.
  // useLiveChannelUpdates already filters this callback to trigger kinds
  // and external authors, so the map is always a strict subset of "newest
  // external trigger message this client has observed."
  const callerOnChannelMessage = liveUpdateOptions.onChannelMessage;
  const handleChannelMessage = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      const current = latestByChannelRef.current.get(channelId) ?? 0;
      if (event.created_at > current) {
        latestByChannelRef.current.set(channelId, event.created_at);
        bumpLatestVersion();
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
    [callerOnChannelMessage, normalizedPubkey],
  );

  const handleSelfChannelMessage = React.useCallback(
    (event: RelayEvent) => {
      const ref = getThreadReference(event.tags);
      if (ref.rootId !== null) {
        participatedRootIdsRef.current.add(ref.rootId);
        if (normalizedPubkey !== null) {
          writeParticipationToStorage(
            normalizedPubkey,
            participatedRootIdsRef.current,
          );
        }
      } else {
        authoredRootIdsRef.current.add(event.id);
        if (normalizedPubkey !== null) {
          writeAuthoredToStorage(normalizedPubkey, authoredRootIdsRef.current);
        }
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
        writeMutedToStorage(normalizedPubkey, mutedRootIdsRef.current);
      }
      bumpLatestVersion();
    },
    [normalizedPubkey],
  );

  const unmuteThread = React.useCallback(
    (rootId: string) => {
      mutedRootIdsRef.current.delete(rootId);
      if (normalizedPubkey !== null) {
        writeMutedToStorage(normalizedPubkey, mutedRootIdsRef.current);
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

    type CatchUpResult =
      | {
          channelId: string;
          ok: true;
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

          // Pass 1: build participation from self-authored thread replies
          // and track self-authored top-level messages for author notifications
          for (const event of events) {
            if (
              normalizedPubkey !== null &&
              event.pubkey.toLowerCase() === normalizedPubkey
            ) {
              const ref = getThreadReference(event.tags);
              if (ref.rootId !== null) {
                participatedRootIdsRef.current.add(ref.rootId);
              } else {
                authoredRootIdsRef.current.add(event.id);
              }
            }
          }

          if (normalizedPubkey !== null) {
            writeParticipationToStorage(
              normalizedPubkey,
              participatedRootIdsRef.current,
            );
            writeAuthoredToStorage(
              normalizedPubkey,
              authoredRootIdsRef.current,
            );
          }

          // Pass 2: compute maxExternal and collect thread reply activity,
          // applying the notification filter to both.
          let maxExternal = 0;
          let maxHighPriority = 0;
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
        const { channelId, maxExternal, maxHighPriority, threadReplies } =
          result;
        allThreadReplies.push(...threadReplies);
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
        };
      }

      const unread = new Set<string>();
      const highPriority = new Set<string>();

      for (const channel of channels) {
        if (channel.id === activeChannelId) continue;

        if (forcedUnreadRef.current.has(channel.id)) {
          // Forced-unread is dot tier only — not high-priority.
          unread.add(channel.id);
          continue;
        }

        const latest = latestByChannelRef.current.get(channel.id);
        if (latest === undefined) continue;

        const readAt = getEffectiveTimestamp(channel.id);
        if (readAt !== null && latest <= readAt) continue;

        unread.add(channel.id);

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

  return {
    unreadChannelIds,
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
    participatedRootIds: participatedRootIdsRef.current as ReadonlySet<string>,
    authoredRootIds: authoredRootIdsRef.current as ReadonlySet<string>,
    threadActivityItems: threadActivityRef.current,
    mutedRootIds: mutedRootIdsRef.current as ReadonlySet<string>,
    muteThread,
    unmuteThread,
  };
}
