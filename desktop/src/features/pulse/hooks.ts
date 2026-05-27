import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  getContactList,
  getGlobalNotes,
  getNotesTimeline,
  getUserNotes,
  publishNote,
  setContactList,
} from "@/shared/api/social";
import type {
  ContactListResponse,
  UserNotesResponse,
} from "@/shared/api/socialTypes";

// ── Query keys ──────────────────────────────────────────────────────────────

export const pulseQueryKeys = {
  contactList: (pubkey: string) => ["contact-list", pubkey] as const,
  globalNotes: ["global-notes"] as const,
  myNotes: (pubkey: string) => ["my-notes", pubkey] as const,
  // Use a stable sorted string key to avoid reference-equality refetch churn.
  timeline: (pubkeys: string[]) =>
    ["pulse-timeline", [...pubkeys].sort().join(",")] as const,
  allTimelines: ["pulse-timeline"] as const,
};

// ── Contact list ────────────────────────────────────────────────────────────

export function useContactListQuery(pubkey?: string) {
  return useQuery<ContactListResponse>({
    queryKey: pulseQueryKeys.contactList(pubkey ?? ""),
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
    queryFn: () => getContactList(pubkey!),
    enabled: !!pubkey,
    staleTime: 60_000,
    gcTime: 5 * 60_000,
  });
}

// ── Own notes ───────────────────────────────────────────────────────────────

export function useMyNotesQuery(pubkey?: string) {
  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.myNotes(pubkey ?? ""),
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
    queryFn: () => getUserNotes(pubkey!, { limit: 50 }),
    enabled: !!pubkey,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval: 30_000,
  });
}

// ── Timeline (notes from contacts) ─────────────────────────────────────────

export function useTimelineQuery(contactPubkeys: string[], enabled: boolean) {
  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.timeline(contactPubkeys),
    queryFn: () => getNotesTimeline(contactPubkeys, 10),
    enabled: enabled && contactPubkeys.length > 0,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval: 30_000,
  });
}

export function useGlobalNotesQuery(enabled: boolean) {
  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.globalNotes,
    queryFn: () => getGlobalNotes({ limit: 50 }),
    enabled,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval: 30_000,
  });
}

// ── Publish note mutation ───────────────────────────────────────────────────

export function usePublishNoteMutation(currentPubkey?: string) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      content,
      replyTo,
      mentionPubkeys,
      mediaTags,
    }: {
      content: string;
      replyTo?: string;
      mentionPubkeys?: string[];
      mediaTags?: string[][];
    }) => publishNote(content, replyTo, mentionPubkeys, mediaTags),
    onSuccess: () => {
      if (currentPubkey) {
        void queryClient.invalidateQueries({
          queryKey: pulseQueryKeys.myNotes(currentPubkey),
        });
      }
      // Also invalidate timeline queries so the new note appears immediately.
      void queryClient.invalidateQueries({
        queryKey: pulseQueryKeys.allTimelines,
      });
    },
  });
}

// ── Follow / unfollow mutations ─────────────────────────────────────────────

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
      // Fresh read to avoid overwriting concurrent mutations.
      const current = await getContactList(currentPubkey);
      if (current.contacts.some((c) => c.pubkey === targetPubkey)) {
        return; // already following
      }
      const updated = [...current.contacts, { pubkey: targetPubkey }];
      return setContactList(updated);
    },
    onSuccess: () => {
      if (currentPubkey) {
        void queryClient.invalidateQueries({
          queryKey: pulseQueryKeys.contactList(currentPubkey),
        });
        void queryClient.invalidateQueries({
          queryKey: pulseQueryKeys.allTimelines,
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
      // Fresh read to avoid overwriting concurrent mutations.
      const current = await getContactList(currentPubkey);
      const updated = current.contacts.filter((c) => c.pubkey !== targetPubkey);
      return setContactList(updated);
    },
    onSuccess: () => {
      if (currentPubkey) {
        void queryClient.invalidateQueries({
          queryKey: pulseQueryKeys.contactList(currentPubkey),
        });
        void queryClient.invalidateQueries({
          queryKey: pulseQueryKeys.allTimelines,
        });
      }
    },
  });
}
