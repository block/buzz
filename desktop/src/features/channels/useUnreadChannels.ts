import * as React from "react";
import {
  useLiveChannelUpdates,
  type UseLiveChannelUpdatesOptions,
} from "@/features/channels/useLiveChannelUpdates";
import {
  countUnreadAppBadgeObservedEvents,
  countUnreadBadgeObservedEvents,
  countUnreadHighPriorityObservedEvents,
  countUnreadObservedEvents,
  hasUnreadTopLevelObservedEvent,
  makeObservedUnreadEvent,
  observedUnreadEventReadAt,
  recordObservedUnreadEvent,
  type ObservedUnreadEvent,
} from "@/features/channels/unreadChannelCounts";
import { useReadState } from "@/features/channels/readState/useReadState";
import {
  forcedUnreadStore,
  type ForcedUnreadMap,
  useForcedUnreadActions,
} from "@/features/channels/forcedUnreadStore";
import {
  getThreadReference,
  isBroadcastReply,
} from "@/features/messages/lib/threading";
import {
  hasMentionForEvent,
  isHighPriorityEventForUser,
} from "@/features/notifications/lib/shouldNotify";
import type { RelayClient } from "@/shared/api/relayClientSession";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";
import { useStableMap, useStableSet } from "@/shared/hooks/useStableReference";
import { normalizeRelayUrl } from "@/features/profile/lib/selfProfileStorage";
import { DM_NOTIFIABLE_EVENT_KINDS } from "./isDmNotifiableKind";
import {
  addThreadActivityItems,
  projectActivityForScope,
  type ThreadActivityItem,
} from "@/features/channels/threadActivityStorage";
export type { ThreadActivityItem } from "@/features/channels/threadActivityStorage";
export {
  activityScopeKey,
  activityStorageKey,
  addThreadActivityItems,
  projectActivityForScope,
  readActivityFromStorage,
  writeActivityToStorage,
} from "@/features/channels/threadActivityStorage";
import { useObservedUnreadPersistence } from "@/features/channels/useObservedUnreadPersistence";
import {
  authoredStore,
  mentionedStore,
  mutedStore,
  participationStore,
  useObservedUnreadMembershipSeed,
} from "@/features/channels/unreadMembership";
import { useThreadActivityPersistence } from "@/features/channels/useThreadActivityPersistence";
import { unreadCatchUp } from "@/shared/api/tauriUnreadCatchUp";

type UseUnreadChannelsOptions = UseLiveChannelUpdatesOptions & {
  pubkey?: string;
  relayClient?: RelayClient;
  relayUrl?: string;
  mutedChannelIds?: ReadonlySet<string>;
};

// Per-channel cap on the catch-up REQ. We only consume the *max matching*
// event per channel, but the relay can return self-authored / non-trigger
// events that we discard client-side, so we need enough head-room for the
// filter to find one external trigger message. 1000 matches the live sub's
// per-channel limit elsewhere in the app.
const CATCH_UP_LIMIT = 1000;
const EMPTY_ROOT_IDS: ReadonlySet<string> = new Set();

export function channelCatchUpEventKinds(
  channelType: Channel["channelType"] | undefined,
) {
  return channelType === "dm"
    ? DM_NOTIFIABLE_EVENT_KINDS
    : CHANNEL_MESSAGE_EVENT_KINDS;
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

export function resolveObservedUnreadRootId(tags: string[][]): string | null {
  return isBroadcastReply(tags) ? null : getThreadReference(tags).rootId;
}

export function useUnreadChannels(
  channels: Channel[],
  activeChannel: Channel | null,
  options: UseUnreadChannelsOptions = {},
) {
  const {
    pubkey,
    relayClient,
    relayUrl: relayUrlOption,
    mutedChannelIds: mutedChannelIdsOption,
    ...liveUpdateOptions
  } = options;
  const activeChannelId = activeChannel?.id ?? null;
  const normalizedPubkey = pubkey?.toLowerCase() ?? null;
  // Scoped relay key for activity storage; empty string when relay not yet known
  // so rows from an unknown relay never load into the wrong community.
  const normalizedRelayUrl = relayUrlOption
    ? normalizeRelayUrl(relayUrlOption)
    : "";

  const {
    getEffectiveTimestamp,
    isReady: isReadStateReady,
    markContextRead,
    drainSyncedAdvances,
    setContextParentResolver,
    readStateVersion,
    getOwnTimestamp,
  } = useReadState(pubkey, relayClient);

  // Per-channel latest observed external trigger timestamp (unix seconds) and
  // per-event metadata. Derived relay evidence, not source-of-truth; the unread
  // memo compares these against NIP-RS markers. Hydrated/reset by the persistence hook.
  const latestByChannelRef = React.useRef(new Map<string, number>());
  const observedUnreadEventsByChannelRef = React.useRef(
    new Map<string, Map<string, ObservedUnreadEvent>>(),
  );

  const channelsRef = React.useRef(channels);
  channelsRef.current = channels;

  // Channels manually marked unread this session. NIP-RS markers are monotonic,
  // so this flag creates the badge without lowering synced read state.
  // Persisted to buzz-forced-unread.v1 for cross-reload and rail-observer visibility.
  const forcedUnreadRef = React.useRef<ForcedUnreadMap>(
    pubkey ? forcedUnreadStore.read(pubkey) : {},
  );

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
  const mutedChannelIdsRef = React.useRef<ReadonlySet<string>>(EMPTY_ROOT_IDS);
  mutedChannelIdsRef.current = mutedChannelIdsOption ?? EMPTY_ROOT_IDS;

  // Thread reply events that triggered notifications — surfaced in the Home
  // activity feed as synthetic FeedItems. The buffer is the source of truth
  // between coalesced writes; useThreadActivityPersistence owns the loaded
  // scope, the write timer, flush, and hydration.
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

  // Reset all in-session state when the identity or relay changes. In-memory
  // caches are cleared; persisted stores are loaded for the new pubkey (so
  // forced-unread, participation, etc. are correct for the new identity).
  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey/relayClient are intentional reset signals
  React.useEffect(() => {
    // Load persisted forced-unread map for the new pubkey (do NOT clear the
    // store — another device's data should survive identity switches here).
    forcedUnreadRef.current = pubkey ? forcedUnreadStore.read(pubkey) : {};
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
    bumpLatestVersion();
    bumpMembershipVersion();
  }, [pubkey, relayClient, normalizedRelayUrl]);

  const membershipSeed = useObservedUnreadMembershipSeed(
    `${normalizedPubkey ?? ""}\u0000${normalizedRelayUrl}`,
    pubkey,
    options.followedRootIds,
    mutedChannelIdsRef.current,
  );

  const observedPersistence = useObservedUnreadPersistence(
    normalizedPubkey,
    normalizedRelayUrl,
    isReadStateReady,
    readStateVersion,
    getEffectiveTimestamp,
    getOwnTimestamp,
    observedUnreadEventsByChannelRef,
    latestByChannelRef,
    {
      onPruned: bumpLatestVersion,
      membershipSeed,
    },
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion is the intentional drain trigger
  React.useEffect(() => {
    const advanced = drainSyncedAdvances();
    observedPersistence.syncMarkers(advanced);
    let anyNew = false;
    for (const channelId of advanced) {
      if (
        !channelId.startsWith("thread:") &&
        !channelId.startsWith("msg:") &&
        Object.hasOwn(forcedUnreadRef.current, channelId)
      ) {
        delete forcedUnreadRef.current[channelId];
        anyNew = true;
      }
    }
    if (anyNew) {
      if (pubkey) forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
      bumpLatestVersion();
    }
  }, [readStateVersion, drainSyncedAdvances, observedPersistence, pubkey]);

  const followedMembershipRef = React.useRef(new Set<string>());
  React.useEffect(() => {
    const desired = options.followedRootIds ?? EMPTY_ROOT_IDS;
    if (!observedPersistence.isScopeLoaded()) {
      followedMembershipRef.current = new Set(desired);
      return;
    }
    for (const rootId of desired) {
      if (!followedMembershipRef.current.has(rootId)) {
        observedPersistence.updateMembership("followed", rootId, true);
      }
    }
    for (const rootId of followedMembershipRef.current) {
      if (!desired.has(rootId)) {
        observedPersistence.updateMembership("followed", rootId, false);
      }
    }
    followedMembershipRef.current = new Set(desired);
  }, [observedPersistence, options.followedRootIds]);

  // Thread-activity persistence: coalesced writes, pagehide/visibility flush,
  // hydration + legacy-key cleanup. Owns the loaded scope for the buffer above.
  const activityPersistence = useThreadActivityPersistence(
    normalizedPubkey,
    normalizedRelayUrl,
    threadActivityRef,
  );
  const currentActivityScope = activityPersistence.currentScope;

  // `topLevelOnly`: passive channel-open path (NIP-RS Option 1) — marker lands at newest
  // top-level msg without folding observed replies; leaves refs intact so the dot persists
  // until an explicit mark-read. Explicit reads omit this flag and clear the refs.
  const markChannelRead = React.useCallback(
    (
      channelId: string,
      readAt: string | null | undefined,
      {
        preserveForcedUnread = false,
        topLevelOnly = false,
      }: {
        preserveForcedUnread?: boolean;
        topLevelOnly?: boolean;
      } = {},
    ) => {
      if (
        !preserveForcedUnread &&
        Object.hasOwn(forcedUnreadRef.current, channelId)
      ) {
        delete forcedUnreadRef.current[channelId];
        if (pubkey) {
          forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
        }
        bumpLatestVersion();
      }
      const observedLatest = topLevelOnly
        ? undefined
        : observedPersistence.latestForChannel(channelId);
      const { markAt, clearObserved } = resolveChannelReadMarker(
        readAt,
        observedLatest,
      );
      if (markAt === null) return;
      markContextRead(channelId, markAt);
      observedPersistence.syncMarkers(
        [channelId],
        new Map([[channelId, markAt]]),
      );
      // the parent must not delete from latestByChannelRef or
      // observedUnreadEventsByChannelRef directly on the clear-observed path,
      // or a stale scope-A callback could corrupt scope B before the fence rejects.
      if (clearObserved) {
        observedPersistence.removeChannel(channelId);
        bumpLatestVersion();
      }
    },
    [markContextRead, observedPersistence, pubkey],
  );

  const { clearChannelUnreadSource, markChannelUnread } =
    useForcedUnreadActions(
      forcedUnreadRef,
      getOwnTimestamp,
      pubkey,
      bumpLatestVersion,
    );

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
      observedPersistence.updateMembership("mentioned", rootId, true);
      mentionedStore.write(normalizedPubkey, target);
      return true;
    },
    [normalizedPubkey, observedPersistence],
  );

  // Records an external trigger event and schedules persistence.
  const callerOnChannelMessage = liveUpdateOptions.onChannelMessage;
  const recordUnreadEvent = React.useCallback(
    (channelId: string, event: ObservedUnreadEvent): boolean => {
      if (!observedPersistence.isScopeLoaded()) return false;
      if (observedPersistence.isNative()) {
        observedPersistence.schedule(
          observedPersistence.currentScope,
          channelId,
          event,
        );
        return true;
      }
      const didRecord = recordObservedUnreadEvent(
        observedUnreadEventsByChannelRef.current,
        channelId,
        event,
        CATCH_UP_LIMIT,
      );
      if (didRecord)
        observedPersistence.schedule(
          observedPersistence.currentScope,
          channelId,
          event,
        );
      return didRecord;
    },
    [observedPersistence],
  );
  const handleChannelMessage = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      const channel = channelsRef.current.find((ch) => ch.id === channelId);
      const isHighPriority =
        channel?.channelType === "dm" ||
        (normalizedPubkey !== null &&
          isHighPriorityEventForUser(event, normalizedPubkey));
      const isThreadedReply =
        getThreadReference(event.tags).parentId !== null &&
        !isBroadcastReply(event.tags);
      const didRecordUnreadEvent = recordUnreadEvent(
        channelId,
        makeObservedUnreadEvent({
          id: event.id,
          createdAt: event.created_at,
          rootId: resolveObservedUnreadRootId(event.tags),
          highPriority: isHighPriority,
          channelType: channel?.channelType,
          isThreadedReply,
        }),
      );
      // Fence latestByChannelRef on the scope guard — a stale live callback
      // during A→B drift must not write A's timestamp into B's hydrated ref.
      const scopeOk = observedPersistence.isScopeLoaded();
      const current = observedPersistence.latestForChannel(channelId) ?? 0;
      if (
        scopeOk &&
        !observedPersistence.isNative() &&
        event.created_at > current
      ) {
        latestByChannelRef.current.set(channelId, event.created_at);
      }
      if (didRecordUnreadEvent || (scopeOk && event.created_at > current)) {
        bumpLatestVersion();
      }

      // A mention on a reply makes its thread badge-eligible even when the
      // user never participated/authored/followed (the gate's missing term).
      if (recordMentionedRoot(event)) {
        bumpMembershipVersion();
      }

      // A high-priority event can be older than the channel's latest observed
      // normal unread, so it may not advance latestByChannelRef. Still bump so
      // highPriorityUnreadChannelIds re-reads the per-event priority flag.
      if (isHighPriority) {
        bumpLatestVersion();
      }

      callerOnChannelMessage?.(channelId, event);
    },
    [
      callerOnChannelMessage,
      normalizedPubkey,
      observedPersistence,
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
        observedPersistence.updateMembership(
          isParticipation ? "participated" : "authored",
          ref.rootId ?? event.id,
          true,
        );
        bumpMembershipVersion();
      }
      bumpLatestVersion();
    },
    [normalizedPubkey, observedPersistence],
  );

  const recordThreadInteraction = React.useCallback(
    (rootId: string) => {
      const normalizedRootId = rootId.trim();
      if (!normalizedRootId) return;
      const target = participatedRootIdsRef.current;
      const sizeBefore = target.size;
      target.add(normalizedRootId);
      if (target.size === sizeBefore) return;
      observedPersistence.updateMembership(
        "participated",
        normalizedRootId,
        true,
      );
      if (normalizedPubkey !== null) {
        participationStore.write(normalizedPubkey, target);
      }
      bumpMembershipVersion();
    },
    [normalizedPubkey, observedPersistence],
  );

  const handleThreadReplyNotification = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      // Guard: don't merge into a buffer whose scope has drifted from the
      // current identity. isScopeLoaded() also rejects an empty scope, so a
      // writer can never fire before the first valid scope is seeded.
      if (!activityPersistence.isScopeLoaded()) return;

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
      const added = addThreadActivityItems(threadActivityRef.current, [item]);
      if (!added.didAdd) return;
      const didRecordMentionedRoot = recordMentionedRoot(event);
      threadActivityRef.current = added.items;
      activityPersistence.schedule(currentActivityScope);
      if (didRecordMentionedRoot) {
        bumpMembershipVersion();
      }
      bumpLatestVersion();
    },
    [channels, currentActivityScope, activityPersistence, recordMentionedRoot],
  );

  const muteThread = React.useCallback(
    (rootId: string) => {
      mutedRootIdsRef.current.add(rootId);
      observedPersistence.updateMembership("muted_root", rootId, true);
      if (normalizedPubkey !== null) {
        mutedStore.write(normalizedPubkey, mutedRootIdsRef.current);
      }
      bumpLatestVersion();
    },
    [normalizedPubkey, observedPersistence],
  );

  const unmuteThread = React.useCallback(
    (rootId: string) => {
      mutedRootIdsRef.current.delete(rootId);
      observedPersistence.updateMembership("muted_root", rootId, false);
      if (normalizedPubkey !== null) {
        mutedStore.write(normalizedPubkey, mutedRootIdsRef.current);
      }
      bumpLatestVersion();
    },
    [normalizedPubkey, observedPersistence],
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

    // E's native observed-unread store owns notification membership, so this
    // request scales with channels being caught up rather than five 1,000-id
    // sets. The prior 332 KiB-at-cap payload is retired at this boundary.
    void unreadCatchUp({
      channels: toFetch.map((channelId) => {
        const channel = channels.find(
          (candidate) => candidate.id === channelId,
        );
        return {
          id: channelId,
          type: channel?.channelType ?? "stream",
          name: channel?.name ?? "",
          readAt: getEffectiveTimestamp(channelId),
        };
      }),
      selfPubkey: normalizedPubkey ?? "",
      mutedChannelIds: [...mutedChannelIdsRef.current],
    })
      .then(({ channels: results }) => {
        if (isCancelled) return;
        // The command rejects if relay/pubkey scope changes in flight. Keep the
        // renderer fence too: it also covers effect cleanup before merge.
        if (!observedPersistence.isScopeLoaded()) return;

        let didAdvance = false;
        let didDiscover = false;
        const allThreadReplies: ThreadActivityItem[] = [];
        for (const result of results) {
          if (result.status === "error") {
            // The error arm carries only this identity; releasing its claim is
            // what lets a failed channel retry on the next effect run.
            caughtUpChannelsRef.current.delete(result.channelId);
            continue;
          }
          for (const rootId of result.discovered.participated) {
            const before = participatedRootIdsRef.current.size;
            participatedRootIdsRef.current.add(rootId);
            if (participatedRootIdsRef.current.size !== before) {
              observedPersistence.updateMembership(
                "participated",
                rootId,
                true,
              );
              didDiscover = true;
            }
          }
          for (const rootId of result.discovered.authored) {
            const before = authoredRootIdsRef.current.size;
            authoredRootIdsRef.current.add(rootId);
            if (authoredRootIdsRef.current.size !== before) {
              observedPersistence.updateMembership("authored", rootId, true);
              didDiscover = true;
            }
          }
          for (const rootId of result.discovered.mentioned) {
            const before = mentionedRootIdsRef.current.size;
            mentionedRootIdsRef.current.add(rootId);
            if (mentionedRootIdsRef.current.size !== before) {
              observedPersistence.updateMembership("mentioned", rootId, true);
              didDiscover = true;
            }
          }
          allThreadReplies.push(...result.activityRows);
          for (const event of result.observedEvents) {
            recordUnreadEvent(result.channelId, event);
            didAdvance = true;
          }
          if (
            result.maxTrigger > (getEffectiveTimestamp(result.channelId) ?? 0)
          ) {
            const current =
              observedPersistence.latestForChannel(result.channelId) ?? 0;
            if (result.maxTrigger > current) {
              observedPersistence.advanceLatest(
                result.channelId,
                result.maxTrigger,
              );
              didAdvance = true;
            }
          }
        }

        if (normalizedPubkey !== null && didDiscover) {
          participationStore.write(
            normalizedPubkey,
            participatedRootIdsRef.current,
          );
          authoredStore.write(normalizedPubkey, authoredRootIdsRef.current);
          mentionedStore.write(normalizedPubkey, mentionedRootIdsRef.current);
        }
        if (allThreadReplies.length > 0) {
          const added = addThreadActivityItems(
            threadActivityRef.current,
            allThreadReplies,
          );
          if (added.didAdd) {
            threadActivityRef.current = added.items;
            activityPersistence.schedule(currentActivityScope);
            didAdvance = true;
          }
        }
        if (didAdvance) bumpLatestVersion();
        if (
          didDiscover ||
          participatedRootIdsRef.current.size !== participatedSizeBefore ||
          authoredRootIdsRef.current.size !== authoredSizeBefore ||
          mentionedRootIdsRef.current.size !== mentionedSizeBefore
        ) {
          bumpMembershipVersion();
        }
      })
      .catch(() => {
        if (isCancelled) return;
        for (const id of toFetch) caughtUpChannelsRef.current.delete(id);
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
    normalizedRelayUrl,
    recordUnreadEvent,
    relayClient,
  ]);

  // Unread = inactive channels, plus any channel manually marked unread this
  // session. A manually marked active channel must remain visible as unread
  // until the user explicitly marks it read again.
  // High-priority unread = DMs or channels with a mention/broadcast newer
  // than the read marker. Forced-unread channels are dot tier only (not
  // high-priority). Both sets share identical deps and always invalidate
  // together, so they are computed in a single memo.
  const rawUnread =
    // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion and latestVersion are intentional invalidation signals
    React.useMemo(() => {
      if (!isReadStateReady || !observedPersistence.isScopeLoaded()) {
        return {
          unreadChannelIds: new Set<string>(),
          topLevelUnreadChannelIds: new Set<string>(),
          highPriorityUnreadChannelIds: new Set<string>(),
          unreadChannelCounts: new Map<string, number>(),
          unreadChannelNotificationCount: 0,
        };
      }

      const unread = new Set<string>();
      const topLevelUnread = new Set<string>();
      const highPriority = new Set<string>();
      const counts = new Map<string, number>();
      let unreadChannelNotificationCount = 0;

      for (const channel of channels) {
        const isForcedUnread = Object.hasOwn(
          forcedUnreadRef.current,
          channel.id,
        );
        if (channel.id === activeChannelId && !isForcedUnread) continue;

        const observedEvents = observedUnreadEventsByChannelRef.current.get(
          channel.id,
        );
        const channelReadAt = getEffectiveTimestamp(channel.id);
        const readAtForObservedEvent = (event: ObservedUnreadEvent) =>
          observedUnreadEventReadAt(
            event,
            channelReadAt,
            (rootId) => getOwnTimestamp(`thread:${rootId}`),
            (messageId) => getOwnTimestamp(`msg:${messageId}`),
          );

        const nativeProjection = observedPersistence.isNative()
          ? observedPersistence.projectionsRef.current.get(channel.id)
          : undefined;
        const unreadCount = observedPersistence.isNative()
          ? (nativeProjection?.count ?? 0)
          : latestByChannelRef.current.get(channel.id) === undefined
            ? 0
            : countUnreadObservedEvents(observedEvents, readAtForObservedEvent);
        if (unreadCount === 0) {
          if (!isForcedUnread) continue;
          unread.add(channel.id);
          topLevelUnread.add(channel.id);
          counts.set(channel.id, 1);
          unreadChannelNotificationCount += 1;
          continue;
        }

        unread.add(channel.id);
        if (
          nativeProjection?.topLevelUnread ||
          (!nativeProjection &&
            hasUnreadTopLevelObservedEvent(
              observedEvents,
              readAtForObservedEvent,
            ))
        ) {
          topLevelUnread.add(channel.id);
        }
        const badgeCount =
          nativeProjection?.badgeCount ??
          countUnreadBadgeObservedEvents(
            observedEvents,
            readAtForObservedEvent,
          );
        counts.set(channel.id, badgeCount);
        unreadChannelNotificationCount +=
          nativeProjection?.appBadgeCount ??
          countUnreadAppBadgeObservedEvents(
            observedEvents,
            readAtForObservedEvent,
          );

        // DM channels: any unread DM is high-priority.
        if (channel.channelType === "dm") {
          highPriority.add(channel.id);
        } else if (
          nativeProjection?.highPriorityUnread ||
          (!nativeProjection &&
            countUnreadHighPriorityObservedEvents(
              observedEvents,
              readAtForObservedEvent,
            ) > 0)
        ) {
          // Non-DM: high-priority only if at least one mention/broadcast
          // remains unread in its own channel/thread context.
          highPriority.add(channel.id);
        }
      }

      return {
        unreadChannelIds: unread,
        topLevelUnreadChannelIds: topLevelUnread,
        highPriorityUnreadChannelIds: highPriority,
        unreadChannelCounts: counts,
        unreadChannelNotificationCount,
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

  const unreadChannelIds = useStableSet(rawUnread.unreadChannelIds);
  const topLevelUnreadChannelIds = useStableSet(
    rawUnread.topLevelUnreadChannelIds,
  );
  const highPriorityUnreadChannelIds = useStableSet(
    rawUnread.highPriorityUnreadChannelIds,
  );
  const unreadChannelCounts = useStableMap(rawUnread.unreadChannelCounts);
  const unreadChannelNotificationCount =
    rawUnread.unreadChannelNotificationCount;

  const unreadChannelIdsRef = React.useRef(unreadChannelIds);
  unreadChannelIdsRef.current = unreadChannelIds;

  const markAllChannelsRead = React.useCallback(() => {
    const marked = new Map<string, number>();
    for (const channelId of unreadChannelIdsRef.current) {
      delete forcedUnreadRef.current[channelId];
      const unixSeconds =
        observedPersistence.latestForChannel(channelId) ??
        getEffectiveTimestamp(channelId) ??
        null;
      if (unixSeconds !== null) {
        markContextRead(channelId, unixSeconds);
        marked.set(channelId, unixSeconds);
      }
    }
    observedPersistence.syncMarkers(marked.keys(), marked);
    if (pubkey) {
      forcedUnreadStore.write(pubkey, forcedUnreadRef.current);
    }
    // the parent must not reset the observed Maps directly on this path, or a
    // stale scope-A callback could corrupt scope B before the fence rejects.
    // (Fenced record writes in handleChannelMessage and catch-up remain in the parent.)
    observedPersistence.clearAll();
    bumpLatestVersion();
  }, [getEffectiveTimestamp, markContextRead, observedPersistence, pubkey]);

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
    topLevelUnreadChannelIds,
    unreadChannelCounts,
    highPriorityUnreadChannelIds,
    unreadChannelNotificationCount,
    markAllChannelsRead,
    markChannelRead,
    markChannelUnread,
    clearChannelUnreadSource,
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
    recordThreadInteraction,
    threadActivityItems: projectActivityForScope(
      activityPersistence.scopeLoadedRef.current,
      currentActivityScope,
      threadActivityRef.current,
    ),
    mutedRootIds: mutedRootIdsRef.current as ReadonlySet<string>,
    muteThread,
    unmuteThread,
  };
}
