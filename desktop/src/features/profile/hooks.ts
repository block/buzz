import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

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

export const profileQueryKey = ["profile"] as const;
export const contactListQueryKey = (pubkey: string) =>
  ["contact-list", pubkey] as const;
export const allPulseTimelinesQueryKey = ["pulse-timeline"] as const;

export function useProfileQuery(enabled = true) {
  return useQuery({
    enabled,
    queryKey: profileQueryKey,
    queryFn: getProfile,
    staleTime: 30_000,
  });
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
    staleTime: 60_000,
    gcTime: 5 * 60 * 1_000,
  });

  // Seed individual "user-profile" cache entries so avatar clicks are instant
  // cache hits instead of fresh network requests.
  useEffect(() => {
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
    enabled?: boolean;
    limit?: number;
  },
) {
  const normalizedQuery = query.trim().toLowerCase();
  const enabled = (options?.enabled ?? true) && normalizedQuery.length > 0;

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

  return useMutation({
    mutationFn: (input: UpdateProfileInput) => updateProfile(input),
    onSuccess: (profile: Profile) => {
      queryClient.setQueryData(profileQueryKey, profile);
    },
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: profileQueryKey });
    },
  });
}
