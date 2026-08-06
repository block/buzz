import type { QueryClient } from "@tanstack/react-query";

import type { UsersBatchResponse } from "@/shared/api/types";

function normalizedPubkeySet(values: string[]): Set<string> {
  return new Set(values.map((value) => value.toLowerCase()));
}

/** Keep long-lived aggregate profile queries in sync when another batch key
 * refreshes one of the same people. */
export function syncUsersBatchQueryCaches(
  queryClient: QueryClient,
  fresh: UsersBatchResponse,
): void {
  const freshProfiles = Object.fromEntries(
    Object.entries(fresh.profiles).map(([pubkey, profile]) => [
      pubkey.toLowerCase(),
      profile,
    ]),
  );
  const freshMissing = normalizedPubkeySet(fresh.missing);
  const refreshedPubkeys = new Set([
    ...Object.keys(freshProfiles),
    ...freshMissing,
  ]);
  if (refreshedPubkeys.size === 0) return;

  const overlappingQueries = queryClient.getQueryCache().findAll({
    predicate: (query) =>
      query.queryKey[0] === "users-batch" &&
      query.queryKey.some(
        (part) =>
          typeof part === "string" && refreshedPubkeys.has(part.toLowerCase()),
      ),
  });

  for (const query of overlappingQueries) {
    const queryPubkeys = normalizedPubkeySet(
      query.queryKey.filter(
        (part, index): part is string => index > 0 && typeof part === "string",
      ),
    );
    queryClient.setQueryData<UsersBatchResponse>(
      query.queryKey,
      (current) => {
        if (!current) return current;

        const profiles = { ...current.profiles };
        const missing = normalizedPubkeySet(current.missing);
        for (const [pubkey, profile] of Object.entries(freshProfiles)) {
          if (!queryPubkeys.has(pubkey)) continue;
          profiles[pubkey] = profile;
          missing.delete(pubkey);
        }
        for (const pubkey of freshMissing) {
          if (!queryPubkeys.has(pubkey)) continue;
          delete profiles[pubkey];
          missing.add(pubkey);
        }

        return { profiles, missing: [...missing] };
      },
      // This is a partial cache merge, not a successful refresh of every
      // pubkey in the aggregate query. Preserve its freshness timestamp so
      // another participant's result cannot postpone stale entries fetching.
      { updatedAt: query.state.dataUpdatedAt },
    );
  }
}
