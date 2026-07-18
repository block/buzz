import * as React from "react";
import { useNavigate } from "@tanstack/react-router";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";

import { useOpenDmMutation } from "@/features/channels/hooks";
import { useToggleReactionMutation } from "@/features/messages/hooks";
import {
  pulseQueryKeys,
  type PulseReactionState,
  usePublishNoteMutation,
} from "@/features/pulse/hooks";
import {
  applyReactionState,
  buildNoteShareUri,
  isDuplicateReactionError,
  toggleNoteIdInSet,
} from "@/features/pulse/lib/noteActions";
import type { UserNote } from "@/shared/api/socialTypes";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

export type PulseNoteActions = {
  isReplySending: boolean;
  isUpvotePending: (noteId: string) => boolean;
  reactionCount: (noteId: string) => number;
  isUpvoted: (noteId: string) => boolean;
  reply: (
    note: UserNote,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  share: (note: UserNote) => Promise<void>;
  startDm: (pubkey: string) => Promise<void>;
  toggleUpvote: (note: UserNote, remove: boolean) => Promise<void>;
};

export function usePulseNoteActions({
  currentPubkey,
  reactionQueryKey,
  reactions,
}: {
  currentPubkey?: string;
  reactionQueryKey: ReturnType<typeof pulseQueryKeys.reactions>;
  reactions: Map<string, PulseReactionState>;
}): PulseNoteActions {
  const [pendingUpvoteNoteIds, setPendingUpvoteNoteIds] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const replyMutation = usePublishNoteMutation(currentPubkey);
  const toggleReactionMutation = useToggleReactionMutation();
  const openDmMutation = useOpenDmMutation();

  const toggleUpvote = React.useCallback(
    async (note: UserNote, remove: boolean) => {
      if (pendingUpvoteNoteIds.has(note.id)) {
        return;
      }

      setPendingUpvoteNoteIds((current) =>
        toggleNoteIdInSet(current, note.id, true),
      );
      const previousReactions =
        queryClient.getQueryData<Map<string, PulseReactionState>>(
          reactionQueryKey,
        );
      queryClient.setQueryData<Map<string, PulseReactionState>>(
        reactionQueryKey,
        (current) => applyReactionState(current, note.id, !remove),
      );

      try {
        await toggleReactionMutation.mutateAsync({
          eventId: note.id,
          emoji: "+",
          remove,
        });
        if (currentPubkey) {
          void queryClient.invalidateQueries({
            queryKey: pulseQueryKeys.likedNotes(currentPubkey),
          });
        }
      } catch (error) {
        if (isDuplicateReactionError(error)) {
          queryClient.setQueryData<Map<string, PulseReactionState>>(
            reactionQueryKey,
            (current) => applyReactionState(current, note.id, true),
          );
          if (currentPubkey) {
            void queryClient.invalidateQueries({
              queryKey: pulseQueryKeys.likedNotes(currentPubkey),
            });
          }
          return;
        }

        queryClient.setQueryData(reactionQueryKey, previousReactions);
        toast.error(
          error instanceof Error ? error.message : "Failed to update reaction",
        );
      } finally {
        setPendingUpvoteNoteIds((current) =>
          toggleNoteIdInSet(current, note.id, false),
        );
      }
    },
    [
      currentPubkey,
      pendingUpvoteNoteIds,
      queryClient,
      reactionQueryKey,
      toggleReactionMutation,
    ],
  );

  const reply = React.useCallback(
    async (
      note: UserNote,
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      const replyMentionPubkeys = [
        ...new Set([note.pubkey, ...mentionPubkeys]),
      ];

      try {
        await replyMutation.mutateAsync({
          content,
          replyTo: note.id,
          mentionPubkeys: replyMentionPubkeys,
          mediaTags,
        });
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to post reply",
        );
        throw error;
      }
    },
    [replyMutation],
  );

  const share = React.useCallback(async (note: UserNote) => {
    try {
      await writeTextToClipboard(buildNoteShareUri(note));
      toast.success("Copied note link");
    } catch {
      toast.error("Failed to copy note link");
    }
  }, []);

  const startDm = React.useCallback(
    async (pubkey: string) => {
      try {
        const directMessage = await openDmMutation.mutateAsync({
          pubkeys: [pubkey],
        });
        await navigate({
          to: "/channels/$channelId",
          params: { channelId: directMessage.id },
        });
      } catch (error) {
        toast.error(
          error instanceof Error ? error.message : "Failed to open DM",
        );
      }
    },
    [navigate, openDmMutation],
  );

  return {
    isReplySending: replyMutation.isPending,
    isUpvotePending: (noteId) => pendingUpvoteNoteIds.has(noteId),
    reactionCount: (noteId) => reactions.get(noteId)?.count ?? 0,
    isUpvoted: (noteId) => reactions.get(noteId)?.reactedByCurrentUser ?? false,
    reply,
    share,
    startDm,
    toggleUpvote,
  };
}
