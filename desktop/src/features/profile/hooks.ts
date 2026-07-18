import type {
  InfiniteData,
  QueryClient,
  UseInfiniteQueryResult,
} from "@tanstack/react-query";
import * as React from "react";
import {
  keepPreviousData,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";

import {
  getProfile,
  searchUsers,
  getUserProfile,
  getUsersBatch,
  updateProfile,
} from "@/shared/api/tauriProfiles";
import { getContactList, setContactList } from "@/shared/api/social";
import type { ContactListResponse } from "@/shared/api/socialTypes";
import type {
  Profile,
  UpdateProfileInput,
  UserSearchResult,
  UserSearchPage,
  UserProfileSummary,
  UsersBatchResponse,
} from "@/shared/api/types";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getAvatarSnapshotUrl } from "@/shared/lib/animatedAvatar";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import {
  SELF_PROFILE_CACHE_EVENT,
  type SelfProfileCache,
  fetchAvatarDataUrl,
  readSelfProfileCache,
  writeSelfProfileCache,
  shouldFetchAvatar,
  resolveAvatarDataUrl,
} from "@/features/profile/lib/selfProfileStorage";
import { useCommunities } from "@/features/communities/useCommunities";

export const profileQueryKey = ["profile"] as const;
export const contactListQueryKey = (pubkey: string) =>
  ["contact-list", pubkey] as const;
export const allPulseTimelinesQueryKey = ["pulse-timeline"] as const;

/**
 * Persists a freshly-fetched profile to localStorage as the offline fallback.
 * Reuses an existing avatar data URL when the avatar URL is unchanged to avoid
 * re-downloading the image on every ~30s background refetch.
 */
async function persistSelfProfile(
  relayUrl: string,
  pubkey: string,
  profile: Profile,
): Promise<void> {
  const existing = readSelfProfileCache(relayUrl, pubkey);
  const avatarSnapshotUrl = getAvatarSnapshotUrl(profile.avatarUrl);
  const fetched =
    shouldFetchAvatar(profile.avatarUrl, existing) && avatarSnapshotUrl !== null
      ? await fetchAvatarDataUrl(rewriteRelayUrl(avatarSnapshotUrl))
      : null;
  const avatarDataUrl = resolveAvatarDataUrl(
    profile.avatarUrl,
    fetched,
    existing,
  );
  writeSelfProfileCache(relayUrl, pubkey, {
    version: 1,
    displayName: profile.displayName,
    avatarUrl: profile.avatarUrl,
    about: profile.about,
    avatarDataUrl,
    updatedAt: Date.now(),
    // Only persist the presence bit when true — no-event fallbacks
    // (hasProfileEvent: false) must not be cached as real profiles,
    // which would cause the onboarding gate to skip on next restart.
    ...(profile.hasProfileEvent && { hasProfileEvent: true }),
  });
}

export function useProfileQuery(enabled = true) {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const queryClient = useQueryClient();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const pubkey = identityQuery.data?.pubkey ?? "";

  // Parse localStorage once per relayUrl/pubkey pair — not on every render.
  // Cached identity renders instantly and persists through fetch errors (relay
  // unreachable); initialDataUpdatedAt keeps the normal background refetch.
  const cached = React.useMemo(
    () => (relayUrl && pubkey ? readSelfProfileCache(relayUrl, pubkey) : null),
    [relayUrl, pubkey],
  );

  // Stable memo so the seeding effect below has stable deps and doesn't
  // retrigger on unrelated re-renders.
  const initialData = React.useMemo(
    () =>
      cached && cached.updatedAt > 0
        ? ({
            pubkey,
            displayName: cached.displayName,
            avatarUrl: cached.avatarUrl,
            about: cached.about,
            nip05Handle: null,
            ownerPubkey: null,
            // Only true when the cache entry was explicitly written with a
            // real kind:0-backed profile. Older entries (absent field) and
            // no-event fallbacks default to false — conservative is correct.
            hasProfileEvent: cached.hasProfileEvent === true,
          } satisfies Profile)
        : undefined,
    [cached, pubkey],
  );

  // `initialData` is only honored at query construction, which happens before
  // identity/community resolve on a fresh QueryClient — seed the cache
  // imperatively once they arrive, without ever stomping a real fetch result.
  React.useEffect(() => {
    if (!initialData || !cached) return;
    if (queryClient.getQueryData(profileQueryKey) === undefined) {
      queryClient.setQueryData(profileQueryKey, initialData, {
        updatedAt: cached.updatedAt,
      });
    }
  }, [queryClient, initialData, cached]);

  const seedOptions =
    initialData !== undefined
      ? { initialData, initialDataUpdatedAt: cached?.updatedAt }
      : {};

  return useQuery({
    enabled,
    queryKey: profileQueryKey,
    queryFn: async () => {
      const profile = await getProfile();
      if (relayUrl && pubkey) {
        void persistSelfProfile(relayUrl, pubkey, profile);
      }
      return profile;
    },
    staleTime: 30_000,
    ...seedOptions,
  });
}

/**
 * Reactive hook for the locally-cached self-profile.
 *
 * localStorage isn't reactive — the storage module dispatches
 * SELF_PROFILE_CACHE_EVENT after writes so this hook re-reads without polling.
 */
export function useSelfProfileCache(): SelfProfileCache | null {
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const relayUrl = activeCommunity?.relayUrl ?? "";
  const pubkey = identityQuery.data?.pubkey ?? "";

  const [cache, setCache] = React.useState<SelfProfileCache | null>(() =>
    relayUrl && pubkey ? readSelfProfileCache(relayUrl, pubkey) : null,
  );

  // Track whether this is the initial mount so we can skip re-reading the same
  // localStorage value the useState initializer already parsed.
  const isFirstRun = React.useRef(true);

  React.useEffect(() => {
    // Skip the redundant read only on the very first run — it sees the same
    // relayUrl/pubkey the useState initializer already parsed. Consume the
    // flag before the guard below: if the first run bails out (e.g. identity
    // still resolving), the run that later receives the values must read.
    // Accepted: a sub-millisecond unsubscribed window on mount. It is
    // self-healing — the next SELF_PROFILE_CACHE_EVENT or dep change re-syncs;
    // with the no-op write skip the event only fires on real changes.
    const firstRun = isFirstRun.current;
    isFirstRun.current = false;

    if (!relayUrl || !pubkey) {
      setCache(null);
      return;
    }

    if (!firstRun) {
      setCache(readSelfProfileCache(relayUrl, pubkey));
    }

    function handleCacheEvent() {
      setCache(readSelfProfileCache(relayUrl, pubkey));
    }

    window.addEventListener(SELF_PROFILE_CACHE_EVENT, handleCacheEvent);
    return () => {
      window.removeEventListener(SELF_PROFILE_CACHE_EVENT, handleCacheEvent);
    };
  }, [relayUrl, pubkey]);

  return cache;
}

export function useContactListQuery(pubkey?: string) {
  return useQuery<ContactListResponse>({
    queryKey: contactListQueryKey(pubkey ?? ""),
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
    queryFn: () => getContactList(pubkey!),
    enabled: !!pubkey,
    staleTime: 60_000,
    gcTime: 5 * 60_000,
  });
}

/**
 * Follow mutation re-fetches the contact list inside the mutationFn to prevent
 * race conditions when clicking Follow on multiple users quickly. The kind:3
 * contact list is a full-snapshot replaceable event — stale reads cause data loss.
 */
export function useFollowMutation(currentPubkey?: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (targetPubkey: string) => {
      if (!currentPubkey) throw new Error("No identity");
      const current = await getContactList(currentPubkey);
      if (current.contacts.some((c) => c.pubkey === targetPubkey)) {
        return;
      }
      const updated = [...current.contacts, { pubkey: targetPubkey }];
      return setContactList(updated);
    },
    onSuccess: () => {
      if (currentPubkey) {
        void queryClient.invalidateQueries({
          queryKey: contactListQueryKey(currentPubkey),
        });
        void queryClient.invalidateQueries({
          queryKey: allPulseTimelinesQueryKey,
        });
      }
    },
  });
}

export function useUnfollowMutation(currentPubkey?: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (targetPubkey: string) => {
      if (!currentPubkey) throw new Error("No identity");
      const current = await getContactList(currentPubkey);
      const updated = current.contacts.filter((c) => c.pubkey !== targetPubkey);
      return setContactList(updated);
    },
    onSuccess: () => {
      if (currentPubkey) {
        void queryClient.invalidateQueries({
          queryKey: contactListQueryKey(currentPubkey),
        });
        void queryClient.invalidateQueries({
          queryKey: allPulseTimelinesQueryKey,
        });
      }
    },
  });
}

export function useUserProfileQuery(pubkey?: string) {
  return useQuery({
    enabled: typeof pubkey === "string" && pubkey.length > 0,
    queryKey: ["user-profile", pubkey?.toLowerCase() ?? ""],
    queryFn: () => getUserProfile(pubkey),
    staleTime: 60_000,
  });
}

// Per-pubkey resolution cache backing `useUsersBatchQuery`'s delta fetch.
// `summary: null` records a relay-confirmed miss so unknown pubkeys aren't
// re-requested every page. Entries older than the hook's 60s staleTime are
// treated as unresolved and refetched.
type UsersBatchEntry = {
  summary: UserProfileSummary | null;
  fetchedAt: number;
};

const usersBatchEntryKey = (pubkey: string) => ["users-batch-entry", pubkey];

/**
 * Drop the per-pubkey delta-fetch entries so the next `useUsersBatchQuery`
 * run re-fetches these profiles from the relay. Must be called anywhere a
 * specific profile (or a containing `users-batch` query) is invalidated —
 * otherwise the re-run resolves from the still-fresh-looking entry and
 * renders the stale name/avatar for up to the entry's 60s freshness window.
 * Synchronous, so callers can evict before awaiting aggregate invalidations.
 */
export function evictUsersBatchEntries(
  queryClient: QueryClient,
  pubkeys: string[],
) {
  for (const pubkey of pubkeys) {
    queryClient.removeQueries({
      queryKey: usersBatchEntryKey(pubkey.toLowerCase()),
      exact: true,
    });
  }
}

export function useUsersBatchQuery(
  pubkeys: string[],
  options?: {
    enabled?: boolean;
  },
) {
  const queryClient = useQueryClient();
  const normalizedPubkeys = [
    ...new Set(pubkeys.map((pubkey) => pubkey.toLowerCase())),
  ]
    .filter((pubkey) => pubkey.length > 0)
    .sort();
  const enabled = (options?.enabled ?? true) && normalizedPubkeys.length > 0;

  const query = useQuery<UsersBatchResponse>({
    enabled,
    queryKey: ["users-batch", ...normalizedPubkeys],
    // Delta fetch: scroll-back grows the author set one page at a time, and
    // keying on the full sorted list means every growth re-runs the query.
    // Requesting the accumulated set re-downloaded every already-resolved
    // profile (kind-0 payloads embed avatars — ~800KB per scroll page on
    // staging; RESEARCH/PERF_STAGING_SCROLLBACK.md). Resolve from the
    // per-pubkey entry cache first and hit the network only for pubkeys not
    // freshly resolved.
    queryFn: async () => {
      const now = Date.now();
      const profiles: UsersBatchResponse["profiles"] = {};
      const missing: string[] = [];
      const toFetch: string[] = [];
      for (const pubkey of normalizedPubkeys) {
        const entry = queryClient.getQueryData<UsersBatchEntry>(
          usersBatchEntryKey(pubkey),
        );
        if (entry && now - entry.fetchedAt < 60_000) {
          if (entry.summary) profiles[pubkey] = entry.summary;
          else missing.push(pubkey);
        } else {
          toFetch.push(pubkey);
        }
      }
      if (toFetch.length > 0) {
        const fresh = await getUsersBatch(toFetch);
        for (const pubkey of toFetch) {
          const summary = fresh.profiles[pubkey] ?? null;
          queryClient.setQueryData<UsersBatchEntry>(
            usersBatchEntryKey(pubkey),
            { summary, fetchedAt: now },
          );
          if (summary) profiles[pubkey] = summary;
          else missing.push(pubkey);
        }
      }
      return { profiles, missing };
    },
    // Loading older messages grows the pubkey set, which changes this query's
    // key entirely. Without this, already-resolved authors would flash back
    // to their raw pubkey while the larger batch refetches.
    placeholderData: keepPreviousData,
    staleTime: 60_000,
    gcTime: 5 * 60 * 1_000,
  });

  // Seed individual "user-profile" cache entries so avatar clicks are instant
  // cache hits instead of fresh network requests.
  React.useEffect(() => {
    const profiles = query.data?.profiles;
    if (!profiles) return;
    for (const [pubkey, summary] of Object.entries(profiles)) {
      queryClient.setQueryData<Profile>(
        ["user-profile", pubkey],
        (existing) =>
          existing ?? {
            pubkey,
            about: null,
            // Batch endpoint gives UserProfileSummary (no event-presence flag).
            // These cached summaries are never used for the onboarding gate.
            hasProfileEvent: false,
            ...summary,
          },
      );
    }
  }, [query.data, queryClient]);

  return query;
}

export function useUserSearchQuery(
  query: string,
  options?: {
    allowEmpty?: boolean;
    enabled?: boolean;
    limit?: number;
  },
) {
  const normalizedQuery = query.trim().toLowerCase();
  const enabled =
    (options?.enabled ?? true) &&
    (options?.allowEmpty === true || normalizedQuery.length > 0);

  return useQuery<UserSearchResult[]>({
    enabled,
    queryKey: ["user-search", normalizedQuery, options?.limit ?? 8],
    queryFn: async () =>
      (await searchUsers(normalizedQuery, options?.limit ?? 8)).users,
    staleTime: 30_000,
    gcTime: 5 * 60 * 1_000,
  });
}

export function useInfiniteUserSearchQuery(
  query: string,
  options?: {
    allowEmpty?: boolean;
    enabled?: boolean;
    limit?: number;
  },
) {
  const normalizedQuery = query.trim().toLowerCase();
  const enabled =
    (options?.enabled ?? true) &&
    (options?.allowEmpty === true || normalizedQuery.length > 0);

  return useInfiniteQuery<UserSearchPage>({
    enabled,
    queryKey: [
      "user-search",
      "infinite",
      normalizedQuery,
      options?.limit ?? 50,
    ],
    queryFn: ({ pageParam }) =>
      searchUsers(
        normalizedQuery,
        options?.limit ?? 50,
        typeof pageParam === "string" ? pageParam : null,
      ),
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    initialPageParam: null,
    staleTime: 30_000,
    gcTime: 5 * 60 * 1_000,
  });
}

export function useFlattenedUserSearchResults(
  data: InfiniteData<UserSearchPage> | undefined,
) {
  return React.useMemo(
    () => data?.pages.flatMap((page) => page.users) ?? [],
    [data],
  );
}

export function useUserSearchFetchMoreOnScroll(
  query: UseInfiniteQueryResult<InfiniteData<UserSearchPage>>,
  enabled = true,
) {
  return React.useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      if (!enabled) {
        return;
      }

      const list = event.currentTarget;
      if (list.scrollHeight - list.scrollTop - list.clientHeight >= 64) {
        return;
      }

      if (query.hasNextPage && !query.isFetchingNextPage) {
        void query.fetchNextPage();
      }
    },
    [enabled, query],
  );
}

export function useUpdateProfileMutation() {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();

  return useMutation({
    mutationFn: (input: UpdateProfileInput) => updateProfile(input),
    onMutate: async () => {
      // Discard any in-flight profile fetch: a background refetch started
      // before the update (e.g. by a route mount) can resolve AFTER
      // onSuccess writes the fresh profile below and clobber it with the
      // pre-update snapshot — the avatar/name then silently reverts until
      // some later refetch. (Masked historically by users-batch refetch
      // churn from per-keystroke app renders; exposed when those renders
      // were removed.)
      await queryClient.cancelQueries({ queryKey: profileQueryKey });
    },
    onSuccess: async (profile: Profile) => {
      // Cancel again: a refetch may have started while mutationFn awaited.
      await queryClient.cancelQueries({ queryKey: profileQueryKey });
      queryClient.setQueryData(profileQueryKey, profile);
      const relayUrl = activeCommunity?.relayUrl ?? "";
      const pubkey = identityQuery.data?.pubkey ?? profile.pubkey;
      if (relayUrl && pubkey) {
        void persistSelfProfile(relayUrl, pubkey, profile);
      }
      if (pubkey) {
        // Own author labels/avatars render through the users-batch delta
        // cache too — evict so the next batch run picks up the new profile
        // instead of the fresh-looking stale entry, then poke the aggregates.
        evictUsersBatchEntries(queryClient, [pubkey]);
        void queryClient.invalidateQueries({
          queryKey: ["user-profile", pubkey.toLowerCase()],
        });
        void queryClient.invalidateQueries({
          predicate: (query) =>
            query.queryKey[0] === "users-batch" &&
            query.queryKey.includes(pubkey.toLowerCase()),
        });
      }
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: profileQueryKey });
    },
  });
}
