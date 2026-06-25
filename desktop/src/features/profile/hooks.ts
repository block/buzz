import * as React from "react";
import {
  keepPreviousData,
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
} from "@/shared/api/tauri";
import { getContactList, setContactList } from "@/shared/api/social";
import type { ContactListResponse } from "@/shared/api/socialTypes";
import type {
  Profile,
  UpdateProfileInput,
  UserSearchResult,
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
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";

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
    avatarDataUrl,
    updatedAt: Date.now(),
  });
}

export function useProfileQuery(enabled = true) {
  const { activeWorkspace } = useWorkspaces();
  const identityQuery = useIdentityQuery();
  const queryClient = useQueryClient();
  const relayUrl = activeWorkspace?.relayUrl ?? "";
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
            about: null,
            nip05Handle: null,
            ownerPubkey: null,
          } satisfies Profile)
        : undefined,
    [cached, pubkey],
  );

  // `initialData` is only honored at query construction, which happens before
  // identity/workspace resolve on a fresh QueryClient — seed the cache
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
  const { activeWorkspace } = useWorkspaces();
  const identityQuery = useIdentityQuery();
  const relayUrl = activeWorkspace?.relayUrl ?? "";
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
    queryFn: () => getUsersBatch(normalizedPubkeys),
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
        (existing) => existing ?? { pubkey, about: null, ...summary },
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
    queryFn: () => searchUsers(normalizedQuery, options?.limit ?? 8),
    staleTime: 30_000,
    gcTime: 5 * 60 * 1_000,
  });
}

export function useUpdateProfileMutation() {
  const queryClient = useQueryClient();
  const { activeWorkspace } = useWorkspaces();
  const identityQuery = useIdentityQuery();

  return useMutation({
    mutationFn: (input: UpdateProfileInput) => updateProfile(input),
    onSuccess: (profile: Profile) => {
      queryClient.setQueryData(profileQueryKey, profile);
      const relayUrl = activeWorkspace?.relayUrl ?? "";
      const pubkey = identityQuery.data?.pubkey ?? profile.pubkey;
      if (relayUrl && pubkey) {
        void persistSelfProfile(relayUrl, pubkey, profile);
      }
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: profileQueryKey });
    },
  });
}
