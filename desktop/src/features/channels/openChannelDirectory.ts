import * as React from "react";
import { useQueries, useQuery } from "@tanstack/react-query";

import { getChannelDetails, getOpenChannelDirectory } from "@/shared/api/tauri";
import type { Channel, ChannelDetail } from "@/shared/api/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  useStableArrayShallow,
  useStableMap,
} from "@/shared/hooks/useStableReference";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  canFetchChannelsForIdentity,
  channelsQueryKey,
  sortChannels,
  useChannelsQuery,
} from "@/features/channels/hooks";

/**
 * Discovery superset: every joinable open channel plus this identity's own
 * channels. Distinct from {@link channelsQueryKey} (member-only) so the browser
 * and search can hold the wider list without it entering the 60s poll cache.
 * Nested under {@link channelsQueryKey}, so channel mutations that invalidate
 * the member list (join, leave, archive) also refresh a mounted directory.
 */
export const openChannelDirectoryQueryKey = [
  ...channelsQueryKey,
  "open-directory",
] as const;

/** Suppresses redundant directory scans while a browse/search session is open. */
export const OPEN_CHANNEL_DIRECTORY_STALE_TIME_MS = 5 * 60_000;

/**
 * Reconstructs the pre-split merged shape: the member list (authoritative for
 * shared ids, since it carries optimistic mutations and poll timestamps) plus
 * every open channel the member list omits. Callers feed this to the discovery
 * surfaces so no non-member open channel is silently lost when the directory is
 * fetched separately from the 60s poll. Exported for regression coverage.
 */
export function mergeOpenChannelDirectory(
  memberChannels: Channel[],
  directoryChannels: Channel[] | undefined,
): Channel[] {
  if (!directoryChannels || directoryChannels.length === 0) {
    return memberChannels;
  }
  const memberIds = new Set(memberChannels.map((channel) => channel.id));
  const directoryOnly = directoryChannels.filter(
    (channel) => !memberIds.has(channel.id),
  );
  return directoryOnly.length === 0
    ? memberChannels
    : sortChannels([...memberChannels, ...directoryOnly]);
}

/**
 * Fetches the open-channel directory on demand — the discovery superset that
 * `useChannelsQuery` intentionally omits from the 60s poll. Callers pass
 * `enabled` so the unbounded all-open relay scan runs only while the channel
 * browser is open or a global search is active.
 *
 * When no consumer is mounted, a mutation's invalidation only marks the shared
 * key stale, deferring the scan until it is next needed.
 */
export function useOpenChannelDirectoryQuery(options?: { enabled?: boolean }) {
  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? null;
  const identityQuery = useIdentityQuery();
  const ownerPubkey = identityQuery.data?.pubkey ?? null;

  return useQuery({
    enabled:
      (options?.enabled ?? true) &&
      relayUrl !== null &&
      canFetchChannelsForIdentity(ownerPubkey, identityQuery.isError),
    queryKey: openChannelDirectoryQueryKey,
    queryFn: async () => sortChannels(await getOpenChannelDirectory()),
    staleTime: OPEN_CHANNEL_DIRECTORY_STALE_TIME_MS,
  });
}

/**
 * Observes the open-channel directory cache without ever triggering the
 * all-open scan (`enabled: false`). Returns the directory only when a
 * discovery surface (browser, global search, route preview) has already
 * fetched it this session; otherwise `undefined`. This is the "warm cache
 * only" seam: reference resolution reads a directory populated by active
 * discovery but never initiates it while composing or rendering messages.
 */
export function useWarmOpenChannelDirectory(): Channel[] | undefined {
  return useQuery({
    enabled: false,
    queryKey: openChannelDirectoryQueryKey,
    queryFn: async () => sortChannels(await getOpenChannelDirectory()),
    staleTime: OPEN_CHANNEL_DIRECTORY_STALE_TIME_MS,
  }).data;
}

/**
 * The channels resolvable without any network fetch: the member list unioned
 * with a warm open-channel directory. Multi-id and name-bearing consumers use
 * this — a non-member open channel resolves once the reader has browsed or
 * searched channels this session, and stays inert (safe) on a cold cache,
 * which is the ruled product boundary for name references.
 */
export function useChannelSources(options?: { enabled?: boolean }): {
  memberChannels: Channel[];
  warmDirectory: Channel[] | undefined;
  isReady: boolean;
} {
  const channelsQuery = useChannelsQuery(options);
  return {
    memberChannels: channelsQuery.data ?? [],
    warmDirectory: useWarmOpenChannelDirectory(),
    isReady: channelsQuery.isSuccess,
  };
}

export function useResolvedChannelDirectory(options?: { enabled?: boolean }): {
  channels: Channel[];
  isReady: boolean;
} {
  const { memberChannels, warmDirectory, isReady } = useChannelSources(options);
  const channels = React.useMemo(
    () => mergeOpenChannelDirectory(memberChannels, warmDirectory),
    [memberChannels, warmDirectory],
  );
  return { channels, isReady };
}

/** Holds a resolved reference (or a cached miss) across a browse session. */
export const CHANNEL_REFERENCE_STALE_TIME_MS = 5 * 60_000;

/**
 * Returns a reference-query key nested under {@link channelsQueryKey}, so a
 * membership mutation's channel invalidation also drops a cached miss once the
 * channel becomes visible. Exported for mounted-hook regressions that prove
 * channel-reference misses never use the all-open directory key.
 */
export function channelReferenceQueryKey(channelId: string) {
  return [...channelsQueryKey, "reference", channelId] as const;
}

/**
 * Detail metadata does not establish membership. Only member channels and
 * non-member open channels may be navigated to from a resolved reference.
 */
export function isChannelReferenceOpenable(
  channel: Channel | undefined,
): channel is Channel {
  return (
    channel !== undefined && (channel.isMember || channel.visibility === "open")
  );
}

/**
 * A channel detail event carries no membership tag, so `fromRawChannel`
 * defaults `isMember` to true. A reference only reaches the bounded fetch
 * when the id is absent from the member list, so it is by definition not a
 * member: force `isMember: false` here so `isChannelOpenable` keeps a fetched
 * private channel non-openable.
 */
function channelFromFetchedDetail(detail: ChannelDetail): Channel {
  return { ...detail, isMember: false };
}

/**
 * Shared bounded detail query for one unresolved channel id. Both single- and
 * multi-reference consumers use this exact key, fetch, and miss-cache policy,
 * so concurrent surfaces dedupe in React Query rather than creating parallel
 * reference caches.
 */
function channelReferenceQueryOptions({
  channelId,
  enabled,
}: {
  channelId: string;
  enabled: boolean;
}) {
  return {
    enabled,
    queryKey: channelReferenceQueryKey(channelId),
    queryFn: async (): Promise<Channel | null> => {
      try {
        return channelFromFetchedDetail(await getChannelDetails(channelId));
      } catch (error) {
        if (String(error).includes("channel not found")) {
          return null;
        }
        throw error;
      }
    },
    retry: false,
    staleTime: CHANNEL_REFERENCE_STALE_TIME_MS,
  };
}

function uniqueChannelIds(
  channelIds: readonly (string | null | undefined)[],
): string[] {
  return [
    ...new Set(
      channelIds.filter((channelId): channelId is string => Boolean(channelId)),
    ),
  ];
}

/**
 * Resolves a finite set of channel ids without ever initiating directory
 * discovery. Known member/warm-directory entries win immediately; only the
 * remaining ids issue bounded `get_channel_details` requests. Per-id query
 * keys intentionally match `useChannelReference`, which shares in-flight
 * work and five-minute misses across every consumer.
 */
export function useChannelReferences(
  channelIds: readonly (string | null | undefined)[],
  options?: { enabled?: boolean },
): { channelsById: ReadonlyMap<string, Channel>; isReady: boolean } {
  const ids = useStableArrayShallow(
    React.useMemo(() => uniqueChannelIds(channelIds), [channelIds]),
  );
  const { memberChannels, warmDirectory, isReady } = useChannelSources(options);
  const knownById = React.useMemo(() => {
    const channelsById = new Map<string, Channel>();
    for (const channel of warmDirectory ?? []) {
      channelsById.set(channel.id, channel);
    }
    for (const channel of memberChannels) {
      channelsById.set(channel.id, channel);
    }
    return channelsById;
  }, [memberChannels, warmDirectory]);

  const { activeCommunity } = useCommunities();
  const relayUrl = activeCommunity?.relayUrl ?? null;
  const identityQuery = useIdentityQuery();
  const ownerPubkey = identityQuery.data?.pubkey ?? null;
  const canFetch =
    (options?.enabled ?? true) &&
    isReady &&
    relayUrl !== null &&
    canFetchChannelsForIdentity(ownerPubkey, identityQuery.isError);
  const fetchQueries = useQueries({
    queries: ids.map((channelId) =>
      channelReferenceQueryOptions({
        channelId,
        enabled: canFetch && !knownById.has(channelId),
      }),
    ),
  });
  const channelsById = React.useMemo(() => {
    const resolved = new Map(knownById);
    for (let index = 0; index < ids.length; index += 1) {
      const channel = fetchQueries[index]?.data;
      if (channel) {
        resolved.set(ids[index], channel);
      }
    }
    return resolved;
  }, [fetchQueries, ids, knownById]);

  return { channelsById: useStableMap(channelsById), isReady };
}

/**
 * Resolves a single channel id to its metadata (name + visibility) for a
 * reference surface — a permalink chip, project origin, repo-access channel.
 * Resolution order: the member list, then a warm open directory, then a
 * bounded per-id `get_channel_details` fetch after the member list settles
 * (one addressable kind:39000 event, no all-open scan). A genuine "not found"
 * is cached as a resolved miss so an inaccessible id does not refetch on every
 * render; a transient relay error stays unresolved (retryable) rather than
 * caching a false miss.
 */
export function useChannelReference(
  channelId: string | null | undefined,
): Channel | undefined {
  const ids = React.useMemo(() => (channelId ? [channelId] : []), [channelId]);
  const { channelsById } = useChannelReferences(ids);
  return channelId ? channelsById.get(channelId) : undefined;
}
