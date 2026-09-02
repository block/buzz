import { useQuery, useQueryClient } from "@tanstack/react-query";
import { getChannelMembers } from "@/shared/api/tauri";
import type { ChannelMember } from "@/shared/api/types";
import {
  CHANNEL_MEMBERS_STALE_TIME_MS,
  channelMembersQueryKey,
} from "./rosterFreshness";
import { refreshDirectoryForRosterChange } from "./membershipDirectorySync";

/** Read the authoritative destination roster and reconcile directory freshness. */
export function useChannelMembersQuery(
  channelId: string | null,
  enabled = true,
) {
  const queryClient = useQueryClient();
  return useQuery({
    enabled: enabled && channelId !== null,
    queryKey: ["channels", channelId ?? "none", "members"],
    queryFn: async ({ signal }) => {
      if (!channelId) {
        throw new Error("No channel selected.");
      }

      const members = await getChannelMembers(channelId);
      if (!signal.aborted) {
        refreshDirectoryForRosterChange(
          queryClient,
          queryClient.getQueryData<ChannelMember[]>(
            channelMembersQueryKey(channelId),
          ),
          members,
        );
      }
      return members;
    },
    staleTime: CHANNEL_MEMBERS_STALE_TIME_MS,
  });
}
