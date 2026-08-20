import { useQuery } from "@tanstack/react-query";

import { getOpenChannelDirectory } from "@/shared/api/tauri";
import type { Channel } from "@/shared/api/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import {
  canFetchChannelsForIdentity,
  channelsQueryKey,
  sortChannels,
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
