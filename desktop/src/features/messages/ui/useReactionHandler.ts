import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { customEmojiQueryKey } from "@/features/custom-emoji/hooks";
import type {
  TimelineMessage,
  TimelineReaction,
} from "@/features/messages/types";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";

type ReactionHandler = {
  /** Reactions in chronological order (earliest first) as emitted by the formatter. */
  reactions: TimelineReaction[];
  /** Whether the user can currently toggle reactions. */
  canToggle: boolean;
  /** Whether a reaction toggle is in flight. */
  pending: boolean;
  /** Error message from the last failed toggle, if any. */
  errorMessage: string | null;
  /** Call to toggle an emoji reaction. Safe to fire-and-forget. */
  select: (emoji: string) => Promise<void>;
};

/** @visibleForTesting */
export function applyOptimisticReaction(
  reactions: TimelineReaction[],
  emoji: string,
  remove: boolean,
  emojiUrl?: string,
): TimelineReaction[] {
  const existing = reactions.find((reaction) => reaction.emoji === emoji);

  if (remove) {
    if (!existing?.reactedByCurrentUser) return reactions;

    const nextCount = Math.max(0, existing.count - 1);
    if (nextCount === 0) {
      return reactions.filter((reaction) => reaction.emoji !== emoji);
    }

    return reactions.map((reaction) =>
      reaction.emoji === emoji
        ? {
            ...reaction,
            count: nextCount,
            reactedByCurrentUser: false,
            users: reaction.users.filter((user) => user.displayName !== "You"),
          }
        : reaction,
    );
  }

  if (existing) {
    if (existing.reactedByCurrentUser) return reactions;

    return reactions.map((reaction) =>
      reaction.emoji === emoji
        ? {
            ...reaction,
            count: reaction.count + 1,
            reactedByCurrentUser: true,
          }
        : reaction,
    );
  }

  return [
    ...reactions,
    {
      emoji,
      emojiUrl,
      count: 1,
      reactedByCurrentUser: true,
      users: [{ pubkey: "", displayName: "You", avatarUrl: null }],
    },
  ];
}

/**
 * Selects the reactions to display: optimistic state when it is still valid
 * (source has not changed under us), otherwise the formatter-emitted source
 * order. Chronological ordering is the formatter's responsibility; this helper
 * must not re-sort.
 *
 * @visibleForTesting
 */
export function selectDisplayReactions(
  optimisticReactions: TimelineReaction[] | null,
  sourceReactions: TimelineReaction[] | undefined,
): TimelineReaction[] {
  return optimisticReactions ?? sourceReactions ?? [];
}

/**
 * Shared reaction state + toggle logic used by both MessageRow and
 * SystemMessageRow. Keeps the pending/error/optimistic-update concerns in one place.
 */
export function useReactionHandler(
  message: TimelineMessage,
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>,
): ReactionHandler {
  const queryClient = useQueryClient();
  const [pending, setPending] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);
  const sourceReactions = message.reactions;
  const [optimisticState, setOptimisticState] = React.useState<{
    reactions: TimelineReaction[];
    sourceReactions: TimelineReaction[] | undefined;
  } | null>(null);
  const optimisticReactions =
    optimisticState && optimisticState.sourceReactions === sourceReactions
      ? optimisticState.reactions
      : null;

  const reactions = React.useMemo(() => {
    return selectDisplayReactions(optimisticReactions, sourceReactions);
  }, [sourceReactions, optimisticReactions]);

  const canToggle = Boolean(onToggleReaction && !message.pending);

  const select = React.useCallback(
    async (emoji: string) => {
      if (!onToggleReaction || pending) {
        return;
      }

      const remove = reactions.some(
        (reaction) => reaction.emoji === emoji && reaction.reactedByCurrentUser,
      );

      setErrorMessage(null);
      setPending(true);
      const emojiUrl = reactionEmojiUrl(
        emoji,
        queryClient.getQueryData<CustomEmoji[]>(customEmojiQueryKey),
      );
      setOptimisticState((current) => {
        const baseReactions =
          current && current.sourceReactions === sourceReactions
            ? current.reactions
            : reactions;

        return {
          reactions: applyOptimisticReaction(
            baseReactions,
            emoji,
            remove,
            emojiUrl,
          ),
          sourceReactions,
        };
      });
      try {
        await onToggleReaction(message, emoji, remove);
      } catch (error) {
        setOptimisticState(null);
        const nextMessage =
          error instanceof Error
            ? error.message
            : "Failed to update the reaction.";
        setErrorMessage(nextMessage);
        throw error;
      } finally {
        setPending(false);
      }
    },
    [
      message,
      onToggleReaction,
      pending,
      queryClient,
      reactions,
      sourceReactions,
    ],
  );

  return { reactions, canToggle, pending, errorMessage, select };
}
