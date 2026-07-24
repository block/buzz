import type { Query, QueryClient } from "@tanstack/react-query";

import type { ChannelMember } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

function isChannelMembersQuery(query: Query): boolean {
  const { queryKey } = query;
  return (
    queryKey.length === 3 &&
    queryKey[0] === "channels" &&
    typeof queryKey[1] === "string" &&
    queryKey[2] === "members"
  );
}

function queryContainsMember(query: Query, normalizedPubkey: string): boolean {
  if (!isChannelMembersQuery(query) || !Array.isArray(query.state.data)) {
    return false;
  }

  return (query.state.data as ChannelMember[]).some(
    (member) => normalizePubkey(member.pubkey) === normalizedPubkey,
  );
}

/**
 * Applies a profile rename to every cached channel-member row for one pubkey.
 * Matching queries are marked stale without refetching, so active channels
 * update immediately while a rename never fans out into one relay query per
 * cached channel.
 */
export async function updateCachedChannelMemberDisplayName(
  queryClient: QueryClient,
  pubkey: string,
  displayName: string | null,
): Promise<void> {
  const normalizedPubkey = normalizePubkey(pubkey);
  const matchingMemberPredicate = (query: Query) =>
    queryContainsMember(query, normalizedPubkey);
  const matchingMemberQueryHashes = new Set(
    queryClient
      .getQueryCache()
      .findAll({ predicate: matchingMemberPredicate })
      .map((query) => query.queryHash),
  );
  const fetchingMemberQueryHashes = new Set(
    queryClient
      .getQueryCache()
      .findAll({
        predicate: (query) =>
          isChannelMembersQuery(query) &&
          query.state.fetchStatus === "fetching",
      })
      .map((query) => query.queryHash),
  );

  // A first-load query has no rows to inspect yet, so cancel every member
  // fetch already in flight. Restart only requests without a matching cached
  // row after applying the local rename; this bounds network work to requests
  // that were already underway while preventing pre-rename snapshots from
  // winning.
  await queryClient.cancelQueries({
    predicate: (query) => fetchingMemberQueryHashes.has(query.queryHash),
  });

  queryClient.setQueriesData<ChannelMember[]>(
    { predicate: matchingMemberPredicate },
    (current) => {
      if (!current) {
        return current;
      }

      return current.map((member) =>
        normalizePubkey(member.pubkey) === normalizedPubkey &&
        member.displayName !== displayName
          ? { ...member, displayName }
          : member,
      );
    },
  );

  await queryClient.invalidateQueries({
    predicate: matchingMemberPredicate,
    refetchType: "none",
  });

  void queryClient.refetchQueries({
    predicate: (query) =>
      fetchingMemberQueryHashes.has(query.queryHash) &&
      !matchingMemberQueryHashes.has(query.queryHash),
    type: "all",
  });
}
