import * as React from "react";
import {
  useLiveChannelUpdates,
  type UseLiveChannelUpdatesOptions,
} from "@/features/channels/useLiveChannelUpdates";
import {
  readStoredLatestMessages,
  writeStoredLatestMessages,
} from "@/features/channels/latestMessageStorage";
import { useReadState } from "@/features/channels/readState/useReadState";
import type { RelayClient } from "@/shared/api/relayClientSession";
import type { Channel, RelayEvent } from "@/shared/api/types";

type UseUnreadChannelsOptions = UseLiveChannelUpdatesOptions & {
  pubkey?: string;
  relayClient?: RelayClient;
};

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
    seedContextRead,
  } = useReadState(pubkey, relayClient);

  // In-session "latest message at" per channel (unix seconds), driven by the
  // live subscription. The backend doesn't populate Channel.lastMessageAt, so
  // unread state cannot rely on it; this map is the authoritative source for
  // sidebar badges. Monotonic: only advances. Reset when the identity or
  // relay changes. Stale entries for channels the user has left are silently
  // ignored by the memo (it iterates the current channels list, not the map).
  const latestByChannelRef = React.useRef(new Map<string, number>());

  // Channels manually marked unread this session (e.g., right-click → "mark
  // unread"). Tracked separately from latestByChannelRef so we don't have to
  // synthesise a "latest message" timestamp and risk the corresponding read
  // marker becoming sticky. Cleared when the user opens the channel.
  const forcedUnreadRef = React.useRef(new Set<string>());

  const [latestVersion, bumpLatestVersion] = React.useReducer(
    (x: number) => x + 1,
    0,
  );

  // Track whether channels have been initialized (for first-load seeding)
  const hasInitializedChannelsRef = React.useRef(false);

  // Reset the in-session state when the identity or relay changes, then
  // hydrate the per-channel "latest message at" map from localStorage so
  // unread badges survive an app restart. Without this, the only source for
  // `latest` is the live subscription (which uses `since: now` and so
  // doesn't backfill), meaning every channel's latest would be undefined on
  // startup and the unread comparison would always be false.
  // biome-ignore lint/correctness/useExhaustiveDependencies: pubkey/relayClient are intentional reset signals
  React.useEffect(() => {
    latestByChannelRef.current = pubkey
      ? readStoredLatestMessages(pubkey)
      : new Map();
    forcedUnreadRef.current = new Set();
    hasInitializedChannelsRef.current = false;
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
  // the in-session forced-unread flag so the sidebar badge appears immediately
  // without us inventing a synthetic latest-message timestamp, and rolls the
  // NIP-RS read marker back so the unread state syncs across devices. The
  // forced flag is cleared when the user opens the channel (markChannelRead).
  const markChannelUnread = React.useCallback(
    (channelId: string, lastMessageAt: string | null | undefined) => {
      if (!forcedUnreadRef.current.has(channelId)) {
        forcedUnreadRef.current.add(channelId);
        bumpLatestVersion();
      }
      const unixSeconds = toUnixSeconds(lastMessageAt);
      if (unixSeconds !== null) {
        markContextUnread(channelId, unixSeconds);
      }
    },
    [markContextUnread],
  );

  // Opportunistic backfill: if Channel.lastMessageAt is ever populated by the
  // backend (today it isn't), seed the in-session map. Strict max — never
  // overwrites a more recent live value.
  React.useEffect(() => {
    if (channels.length === 0) return;
    let didAdvance = false;
    for (const channel of channels) {
      const seedUnix = toUnixSeconds(channel.lastMessageAt);
      if (seedUnix === null) continue;
      const current = latestByChannelRef.current.get(channel.id) ?? 0;
      if (seedUnix > current) {
        latestByChannelRef.current.set(channel.id, seedUnix);
        didAdvance = true;
      }
    }
    if (didAdvance) {
      if (pubkey) {
        writeStoredLatestMessages(pubkey, latestByChannelRef.current);
      }
      bumpLatestVersion();
    }
  }, [channels, pubkey]);

  // Seed read state on first load so existing channels don't flash as unread
  // when the backend reports a non-null Channel.lastMessageAt. We deliberately
  // seed from the backend-provided value (not from the live map), so a live
  // event that races ahead of NIP-RS readiness can't be silently swallowed as
  // already-read. Today the backend always returns null and this is a no-op;
  // it becomes meaningful once last_message_at is wired up server-side.
  React.useEffect(() => {
    if (!isReadStateReady) return;
    if (channels.length === 0) return;
    if (hasInitializedChannelsRef.current) return;

    for (const channel of channels) {
      const existing = getEffectiveTimestamp(channel.id);
      if (existing !== null) continue;

      const seedUnix = toUnixSeconds(channel.lastMessageAt);
      if (seedUnix !== null) {
        seedContextRead(channel.id, seedUnix);
      }
    }

    hasInitializedChannelsRef.current = true;
  }, [channels, getEffectiveTimestamp, isReadStateReady, seedContextRead]);

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

  // Feed the in-session "latest message at" map from live channel events.
  // Composes with any caller-supplied onChannelMessage handler. Persists to
  // localStorage so unread badges survive an app restart — see the hydrate
  // effect above.
  const callerOnChannelMessage = liveUpdateOptions.onChannelMessage;
  const handleChannelMessage = React.useCallback(
    (channelId: string, event: RelayEvent) => {
      const current = latestByChannelRef.current.get(channelId) ?? 0;
      if (event.created_at > current) {
        latestByChannelRef.current.set(channelId, event.created_at);
        if (pubkey) {
          writeStoredLatestMessages(pubkey, latestByChannelRef.current);
        }
        bumpLatestVersion();
      }
      callerOnChannelMessage?.(channelId, event);
    },
    [callerOnChannelMessage, pubkey],
  );

  useLiveChannelUpdates(channels, activeChannelId, {
    ...liveUpdateOptions,
    onChannelMessage: handleChannelMessage,
  });

  // Unread = channels (excluding active) that have either been manually
  // marked unread this session, or whose in-session latest message timestamp
  // is strictly newer than their NIP-RS read marker.
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
