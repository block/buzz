import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  getGlobalNotes,
  getLikedNotes,
  getNote,
  getNoteReactions,
  getNotesTimeline,
  getUserNotes,
  publishNote,
} from "@/shared/api/social";
import { allPulseTimelinesQueryKey } from "@/features/profile/hooks";
import { withoutProjectComments } from "@/features/pulse/lib/projectComments";
import type { UserNote, UserNotesResponse } from "@/shared/api/socialTypes";

function isDocumentVisible() {
  return typeof document === "undefined"
    ? true
    : document.visibilityState === "visible";
}

function useDocumentVisible() {
  const [visible, setVisible] = React.useState(isDocumentVisible);

  React.useEffect(() => {
    if (typeof document === "undefined") {
      return;
    }

    function handleVisibilityChange() {
      setVisible(isDocumentVisible());
    }

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, []);

  return visible;
}

function useVisibleRefetchInterval(intervalMs: number) {
  return useDocumentVisible() ? intervalMs : false;
}

// ── Query keys ──────────────────────────────────────────────────────────────

export const pulseQueryKeys = {
  globalNotes: ["global-notes"] as const,
  likedNotes: (pubkey: string) => ["liked-notes", pubkey] as const,
  myNotes: (pubkey: string) => ["my-notes", pubkey] as const,
  note: (noteId: string) => ["pulse-note", noteId] as const,
  reactions: (noteIds: string[]) =>
    ["pulse-reactions", [...noteIds].sort().join(",")] as const,
  // Use a stable sorted string key to avoid reference-equality refetch churn.
  timeline: (pubkeys: string[]) =>
    ["pulse-timeline", [...pubkeys].sort().join(",")] as const,
  allTimelines: allPulseTimelinesQueryKey,
};

// ── Own notes ───────────────────────────────────────────────────────────────

export function useLikedNotesQuery(pubkey?: string, enabled = true) {
  const refetchInterval = useVisibleRefetchInterval(30_000);

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.likedNotes(pubkey ?? ""),
    queryFn: async () =>
      // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
      withoutProjectComments(await getLikedNotes(pubkey!, 50)),
    enabled: enabled && !!pubkey,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval,
  });
}

export function useMyNotesQuery(pubkey?: string) {
  const refetchInterval = useVisibleRefetchInterval(30_000);

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.myNotes(pubkey ?? ""),
    queryFn: async () =>
      // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
      withoutProjectComments(await getUserNotes(pubkey!, { limit: 50 })),
    enabled: !!pubkey,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval,
  });
}

// ── Timeline (notes from contacts) ─────────────────────────────────────────

export function useTimelineQuery(contactPubkeys: string[], enabled: boolean) {
  const refetchInterval = useVisibleRefetchInterval(30_000);

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.timeline(contactPubkeys),
    queryFn: async () =>
      withoutProjectComments(await getNotesTimeline(contactPubkeys, 10)),
    enabled: enabled && contactPubkeys.length > 0,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval,
  });
}

export type PulseReactionState = {
  count: number;
  reactedByCurrentUser: boolean;
};

export function usePulseReactionsQuery(
  noteIds: string[],
  currentPubkey?: string,
) {
  const refetchInterval = useVisibleRefetchInterval(60_000);

  return useQuery<Map<string, PulseReactionState>>({
    queryKey: pulseQueryKeys.reactions(noteIds),
    queryFn: async () => {
      const summaries = await getNoteReactions(noteIds);
      const result = new Map<string, PulseReactionState>();
      for (const summary of summaries) {
        if (summary.emoji !== "+") {
          continue;
        }
        result.set(summary.noteId, {
          count: summary.count,
          reactedByCurrentUser: currentPubkey
            ? summary.pubkeys.includes(currentPubkey)
            : false,
        });
      }
      return result;
    },
    enabled: noteIds.length > 0,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval,
  });
}

export function useNoteByIdQuery(noteId: string | null) {
  return useQuery<UserNote | null>({
    queryKey: pulseQueryKeys.note(noteId ?? ""),
    queryFn: () => getNote(noteId ?? ""),
    enabled: !!noteId,
    staleTime: 5 * 60_000,
    gcTime: 10 * 60_000,
  });
}

export function useGlobalNotesQuery(enabled: boolean) {
  const refetchInterval = useVisibleRefetchInterval(30_000);

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.globalNotes,
    queryFn: async () =>
      withoutProjectComments(await getGlobalNotes({ limit: 50 })),
    enabled,
    staleTime: 15_000,
    gcTime: 5 * 60_000,
    refetchInterval,
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
      void queryClient.invalidateQueries({
        queryKey: pulseQueryKeys.globalNotes,
      });
    },
  });
}
