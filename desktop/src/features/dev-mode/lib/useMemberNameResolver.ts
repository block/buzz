import * as React from "react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import { truncatePubkey } from "@/shared/lib/pubkey";

export type NameResolver = (pubkey: string) => string;

export function useMemberNameResolver(channelId: string): NameResolver {
  const membersQuery = useChannelMembersQuery(channelId);
  return React.useCallback<NameResolver>(
    (pubkey) => {
      const member = membersQuery.data?.find(
        (candidate) => candidate.pubkey === pubkey,
      );
      return member?.displayName || truncatePubkey(pubkey);
    },
    [membersQuery.data],
  );
}
