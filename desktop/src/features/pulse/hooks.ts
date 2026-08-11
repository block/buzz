import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

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
import { useFocusedRefetchInterval } from "@/shared/lib/useDocumentVisible";

/** Keeps focused polling at the established 30-second cadence for note feeds. */
export const PULSE_NOTES_REFETCH_INTERVAL_MS = 30_000;
/** Keeps focused polling at the established 60-second cadence for reactions. */
export const PULSE_REACTIONS_REFETCH_INTERVAL_MS = 60_000;
/** Suppresses the focus refetch until pulse data is genuinely stale.
 * Polls every 30s while focused — the focus refetch would duplicate that work. */
export const PULSE_FOCUS_STALE_TIME_MS = 5 * 60_000;

/** Focus-refetch policy shared by all pulse queries; consumed by focusRefetchPolicy.test.mjs. */
export const pulseFocusRefetchPolicy = {
  staleTime: PULSE_FOCUS_STALE_TIME_MS,
  refetchOnWindowFocus: true,
} as const;

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
  const refetchInterval = useFocusedRefetchInterval(
    PULSE_NOTES_REFETCH_INTERVAL_MS,
  );

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.likedNotes(pubkey ?? ""),
    queryFn: async () =>
      // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
      withoutProjectComments(await getLikedNotes(pubkey!, 50)),
    enabled: enabled && !!pubkey,
    gcTime: 5 * 60_000,
    refetchInterval,
    ...pulseFocusRefetchPolicy,
  });
}

export function useMyNotesQuery(pubkey?: string) {
  const refetchInterval = useFocusedRefetchInterval(
    PULSE_NOTES_REFETCH_INTERVAL_MS,
  );

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.myNotes(pubkey ?? ""),
    queryFn: async () =>
      // biome-ignore lint/style/noNonNullAssertion: guarded by enabled: !!pubkey
      withoutProjectComments(await getUserNotes(pubkey!, { limit: 50 })),
    enabled: !!pubkey,
    gcTime: 5 * 60_000,
    refetchInterval,
    ...pulseFocusRefetchPolicy,
  });
}

// ── Timeline (notes from contacts) ─────────────────────────────────────────

export function useTimelineQuery(contactPubkeys: string[], enabled: boolean) {
  const refetchInterval = useFocusedRefetchInterval(
    PULSE_NOTES_REFETCH_INTERVAL_MS,
  );

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.timeline(contactPubkeys),
    queryFn: async () =>
      withoutProjectComments(await getNotesTimeline(contactPubkeys, 10)),
    enabled: enabled && contactPubkeys.length > 0,
    gcTime: 5 * 60_000,
    refetchInterval,
    ...pulseFocusRefetchPolicy,
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
  const refetchInterval = useFocusedRefetchInterval(
    PULSE_REACTIONS_REFETCH_INTERVAL_MS,
  );

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
    gcTime: 5 * 60_000,
    refetchInterval,
    ...pulseFocusRefetchPolicy,
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
  const refetchInterval = useFocusedRefetchInterval(
    PULSE_NOTES_REFETCH_INTERVAL_MS,
  );

  return useQuery<UserNotesResponse>({
    queryKey: pulseQueryKeys.globalNotes,
    queryFn: async () =>
      withoutProjectComments(await getGlobalNotes({ limit: 50 })),
    enabled,
    gcTime: 5 * 60_000,
    refetchInterval,
    ...pulseFocusRefetchPolicy,
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
