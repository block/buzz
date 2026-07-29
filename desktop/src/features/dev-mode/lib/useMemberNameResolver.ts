import * as React from "react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { truncatePubkey } from "@/shared/lib/pubkey";

export type NameResolver = (pubkey: string) => string;

export type AgentResolver = (pubkey: string) => boolean;

/** Whether a pubkey belongs to an agent member of the channel. */
export function useMemberAgentResolver(channelId: string): AgentResolver {
  const membersQuery = useChannelMembersQuery(channelId);
  return React.useCallback<AgentResolver>(
    (pubkey) =>
      membersQuery.data?.some(
        (candidate) =>
          candidate.pubkey === pubkey &&
          (candidate.isAgent || candidate.role === "bot"),
      ) ?? false,
    [membersQuery.data],
  );
}

/**
 * Resolves names from the channel's member list, with a batch profile
 * lookup as fallback for `extraPubkeys` that are no longer members —
 * membership rows name people who already left the channel.
 */
export function useMemberNameResolver(
  channelId: string,
  extraPubkeys: readonly string[] = [],
): NameResolver {
  const membersQuery = useChannelMembersQuery(channelId);
  const members = membersQuery.data;

  const unknownPubkeys = React.useMemo(
    () =>
      extraPubkeys.filter(
        (pubkey) => !members?.some((candidate) => candidate.pubkey === pubkey),
      ),
    [extraPubkeys, members],
  );
  const profilesQuery = useUsersBatchQuery(unknownPubkeys);

  return React.useCallback<NameResolver>(
    (pubkey) => {
      const member = members?.find((candidate) => candidate.pubkey === pubkey);
      if (member?.displayName) return member.displayName;
      const profile = profilesQuery.data?.profiles[pubkey.toLowerCase()];
      return profile?.displayName || profile?.name || truncatePubkey(pubkey);
    },
    [members, profilesQuery.data],
  );
}
