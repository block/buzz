/**
 * Member-roster freshness policy and the invalidation helper for write paths
 * that bypass the member mutations. Split from hooks.ts to keep that file
 * under the per-file line cap; behavior unchanged.
 */

import type { useQueryClient } from "@tanstack/react-query";

import { onChannelMembershipChange } from "@/shared/api/channelMembershipWrites";

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
 * every channel switch. Refetches just after a change read from the relay
 * writer (channelMembershipWrites), so none can cache the pre-change roster.
 */
export const CHANNEL_MEMBERS_STALE_TIME_MS = 5 * 60_000;

/**
 * Invalidates cached rosters for channels whose membership was written
 * through direct `removeChannelMember` calls that bypass the member
 * mutations (moderation kick, agent-deletion cleanup). The roster's long
 * freshness window (CHANNEL_MEMBERS_STALE_TIME_MS) means any direct write
 * path that skips this leaves the removed identity visible until the window
 * lapses. Accepts a minimal client shape so node unit tests can stub it.
 */
export async function invalidateChannelMembersRosters(
  queryClient: Pick<ReturnType<typeof useQueryClient>, "invalidateQueries">,
  channelIds: Iterable<string>,
) {
  const uniqueChannelIds = [...new Set(channelIds)];
  for (const channelId of uniqueChannelIds) {
    await queryClient.invalidateQueries({
      queryKey: channelMembersQueryKey(channelId),
    });
  }
}

/**
 * Refreshes a channel's cached roster whenever its membership changes (see
 * channelMembershipWrites). Cancels a fetch already in flight first: it may
 * have been routed to the lagging replica before the change was recorded.
 */
export function refreshRostersOnMembershipChange(
  queryClient: Pick<
    ReturnType<typeof useQueryClient>,
    "cancelQueries" | "invalidateQueries"
  >,
) {
  return onChannelMembershipChange((channelId) => {
    const queryKey = channelMembersQueryKey(channelId);
    void queryClient
      .cancelQueries({ queryKey, exact: true })
      .then(() => queryClient.invalidateQueries({ queryKey, exact: true }));
  });
}
