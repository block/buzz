/**
 * Member-roster freshness policy and the invalidation helper for write paths
 * that bypass the member mutations. Split from hooks.ts to keep that file
 * under the per-file line cap; behavior unchanged.
 */

import type { useQueryClient } from "@tanstack/react-query";

import { getChannelMembers } from "@/shared/api/tauriChannels";

/** Single source for the members cache key; hooks.ts imports it from here. */
export const channelMembersQueryKey = (channelId: string) =>
  ["channels", channelId, "members"] as const;

/**
 * Freshness window for the full member roster. Kept long because every
 * membership change the client can observe invalidates this key explicitly:
 * live join/leave/removed system messages for the active channel
 * (useChannelSubscription), member-added/removed notifications targeting the
 * current identity (useMembershipNotifications), and every membership
 * mutation (add/remove/join/leave, template apply). The residual staleness is
 * a third party joining a channel the viewer is not currently subscribed to,
 * which corrects within this window. The previous 30s window put a full
 * roster fetch (kind:39002 + a kind:0 batch over every member) on nearly
 * every channel switch.
 */
export const CHANNEL_MEMBERS_STALE_TIME_MS = 5 * 60_000;

/**
 * Channels whose membership this client just changed. Their next roster
 * fetches read from the relay's writer (`readYourWrites`) until one completes
 * unaborted, because the replica can still return the pre-change roster, and
 * that result would then count as fresh for CHANNEL_MEMBERS_STALE_TIME_MS.
 * Clearing only on a completed strong read means a strong fetch cancelled by
 * a later refetch (e.g. a broad `["channels"]` invalidation) still leaves the
 * replacement fetch on the writer. Ordinary roster reads stay on the replica.
 */
const channelsAwaitingWriterRead = new Set<string>();

/** Query function for the channel roster; see channelsAwaitingWriterRead. */
export async function fetchChannelMembers(
  channelId: string,
  signal?: AbortSignal,
) {
  const readYourWrites = channelsAwaitingWriterRead.has(channelId);
  const members = await getChannelMembers(
    channelId,
    readYourWrites ? { readYourWrites } : undefined,
  );
  if (readYourWrites && !signal?.aborted) {
    channelsAwaitingWriterRead.delete(channelId);
  }
  return members;
}

/**
 * Refreshes cached rosters after this client changed their membership: the
 * member mutations, template apply, live join/leave/removed events, and
 * direct `removeChannelMember` writes (moderation kick, agent deletion).
 * Marks each channel for a writer read, cancels any replica fetch already in
 * flight so it cannot land over the fresh roster, then refetches. Accepts a
 * minimal client shape so node unit tests can stub it.
 */
export async function invalidateChannelMembersRosters(
  queryClient: Pick<
    ReturnType<typeof useQueryClient>,
    "cancelQueries" | "invalidateQueries"
  >,
  channelIds: Iterable<string>,
) {
  const uniqueChannelIds = [...new Set(channelIds)];
  for (const channelId of uniqueChannelIds) {
    const queryKey = channelMembersQueryKey(channelId);
    channelsAwaitingWriterRead.add(channelId);
    await queryClient.cancelQueries({ queryKey, exact: true });
    await queryClient.invalidateQueries({ queryKey, exact: true });
  }
}
