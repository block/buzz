import * as React from "react";
import {
  useLiveChannelUpdates,
  type UseLiveChannelUpdatesOptions,
} from "@/features/channels/useLiveChannelUpdates";
import { useReadState } from "@/features/channels/readState/useReadState";
import type { RelayClient } from "@/shared/api/relayClientSession";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { CHANNEL_MESSAGE_EVENT_KINDS } from "@/shared/constants/kinds";

type UseUnreadChannelsOptions = UseLiveChannelUpdatesOptions & {
  pubkey?: string;
  relayClient?: RelayClient;
};

// Per-channel cap on the catch-up REQ. We only consume the *max matching*
// event per channel, but the relay can return self-authored / non-trigger
// events that we discard client-side, so we need enough head-room for the
// filter to find one external trigger message. 1000 matches the live sub's
// per-channel limit elsewhere in the app.
const CATCH_UP_LIMIT = 1000;

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

export function useUnreadChannels(
  channels: Channel[],
  activeChannel: Channel | null,
  activeReadAt?: string | null,
  options: UseUnreadChannelsOptions = {},
) {
  const { pubkey, relayClient, ...liveUpdateOptions } = options;
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
    markContextUnread,
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

  // Channels manually marked unread this session (e.g., right-click → "mark
  // unread"). The NIP-RS rollback (markContextUnread) is the cross-device
  // mechanism; this in-session flag is what makes the badge appear *now* in
  // the case where we don't yet have an observed latest timestamp to compare
  // against. Cleared when the user opens the channel.
  const forcedUnreadRef = React.useRef(new Set<string>());

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
    forcedUnreadRef.current = new Set();
    caughtUpChannelsRef.current = new Set();
    bumpLatestVersion();
  }, [pubkey, relayClient]);

  const markChannelRead = React.useCallback(
    (channelId: string, readAt: string | null | undefined) => {
      const unixSeconds = toUnixSeconds(readAt);
      if (unixSeconds === null) return;
      // Reading clears any prior manual mark-unread.
      if (forcedUnreadRef.current.delete(channelId)) {
        bumpLatestVersion();
      }
      markContextRead(channelId, unixSeconds);
    },
    [markContextRead],
  );

  // Manually mark a channel unread (e.g., right-click → "mark unread"). Sets
  // the in-session forced flag so the sidebar badge appears immediately, and
  // rolls the NIP-RS read marker back so the unread state syncs across
  // devices. The forced flag is cleared in markChannelRead when the user
  // opens the channel. If lastMessageAt is unknown we still set the forced
  // flag, but skip the NIP-RS rollback — without a target timestamp we have
  // nothing honest to publish.
  const markChannelUnread = React.useCallback(
    (channelId: string, lastMessageAt: string | null | undefined) => {
      if (!forcedUnreadRef.current.has(channelId)) {
        forcedUnreadRef.current.add(channelId);
        bumpLatestVersion();
      }
      const unixSeconds =
        toUnixSeconds(lastMessageAt) ??
        latestByChannelRef.current.get(channelId) ??
        null;
      if (unixSeconds !== null) {
        markContextUnread(channelId, unixSeconds);
      }
    },
    [markContextUnread],
  );

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
      callerOnChannelMessage?.(channelId, event);
    },
    [callerOnChannelMessage],
  );

  useLiveChannelUpdates(channels, activeChannelId, {
    ...liveUpdateOptions,
    onChannelMessage: handleChannelMessage,
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
      | { channelId: string; ok: true; maxExternal: number }
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

          let maxExternal = 0;
          for (const event of events) {
            if (
              normalizedPubkey !== null &&
              event.pubkey.toLowerCase() === normalizedPubkey
            ) {
              continue;
            }
            if (readAt !== null && event.created_at <= readAt) continue;
            if (event.created_at > maxExternal) {
              maxExternal = event.created_at;
            }
          }

          return { channelId, ok: true, maxExternal };
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
      for (const result of results) {
        if (!result.ok) {
          caughtUpChannelsRef.current.delete(result.channelId);
          continue;
        }
        const { channelId, maxExternal } = result;
        if (maxExternal === 0) continue;
        const current = latestByChannelRef.current.get(channelId) ?? 0;
        if (maxExternal > current) {
          latestByChannelRef.current.set(channelId, maxExternal);
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
  // readStateVersion and latestVersion are intentional invalidation signals.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion and latestVersion are intentional invalidation signals
  const unreadChannelIds = React.useMemo(() => {
    if (!isReadStateReady) {
      return new Set<string>();
    }

    return new Set(
      channels
        .filter((channel) => channel.id !== activeChannelId)
        .filter((channel) => {
          if (forcedUnreadRef.current.has(channel.id)) return true;
          const latest = latestByChannelRef.current.get(channel.id);
          if (latest === undefined) return false;

          const readAt = getEffectiveTimestamp(channel.id);
          return readAt === null || latest > readAt;
        })
        .map((channel) => channel.id),
    );
  }, [
    activeChannelId,
    channels,
    getEffectiveTimestamp,
    isReadStateReady,
    latestVersion,
    readStateVersion,
  ]);

  return {
    unreadChannelIds,
    markChannelRead,
    markChannelUnread,
  };
}
