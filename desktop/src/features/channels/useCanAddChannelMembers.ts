import * as React from "react";

import { canAddChannelMembers } from "@/features/channels/lib/channelMemberAdmission";
import {
  useChannelMembersQuery,
  useChannelsQuery,
} from "@/features/channels/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";

// Split from hooks.ts to keep that file under the per-file line cap.

/**
 * Whether the signed-in identity may add *another* identity to this channel,
 * per {@link canAddChannelMembers}. Both queries are the ones the channel UI
 * already holds, so this shares their cache rather than fetching again.
 */
export function useCanAddChannelMembers(channelId: string | null) {
  const channelsQuery = useChannelsQuery();
  const membersQuery = useChannelMembersQuery(channelId);
  const identityQuery = useIdentityQuery();

  const channels = channelsQuery.data;
  const members = membersQuery.data;
  const selfPubkey = identityQuery.data?.pubkey ?? null;
  // Memoized: this hook sits in the composer (useMentionSendFlow), which
  // re-renders per keystroke, and these scans are O(channels) + O(members)
  // — rosters can be 10k+. Structural sharing keeps the inputs' identities
  // stable, so the scans run once per data change instead of per keystroke.
  return React.useMemo(() => {
    const channel =
      channels?.find((candidate) => candidate.id === channelId) ?? null;
    const selfRole = selfPubkey
      ? (members?.find(
          (member) => member.pubkey.toLowerCase() === selfPubkey.toLowerCase(),
        )?.role ?? null)
      : null;

    return canAddChannelMembers({
      channelType: channel?.channelType,
      visibility: channel?.visibility,
      selfRole,
    });
  }, [channelId, channels, members, selfPubkey]);
}
