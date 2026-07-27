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
  /** Replace the current user's choice among a mutually exclusive reaction set. */
  chooseExclusive: (choices: readonly string[], emoji: string) => Promise<void>;
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

/** @visibleForTesting */
export function replaceOwnChoiceReactions(
  reactions: TimelineReaction[],
  choices: readonly string[],
  selectedEmoji: string,
): TimelineReaction[] {
  const choiceSet = new Set(choices);
  let next = reactions;

  for (const reaction of reactions) {
    if (
      reaction.reactedByCurrentUser &&
      choiceSet.has(reaction.emoji) &&
      reaction.emoji !== selectedEmoji
    ) {
      next = applyOptimisticReaction(next, reaction.emoji, true);
    }
  }

  return next.some(
    (reaction) =>
      reaction.emoji === selectedEmoji && reaction.reactedByCurrentUser,
  )
    ? next
    : applyOptimisticReaction(next, selectedEmoji, false);
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

  const chooseExclusive = React.useCallback(
    async (choices: readonly string[], emoji: string) => {
      if (!onToggleReaction || pending || !choices.includes(emoji)) {
        return;
      }

      const activeChoices = reactions.filter(
        (reaction) =>
          reaction.reactedByCurrentUser && choices.includes(reaction.emoji),
      );
      const alreadySelected = activeChoices.some(
        (reaction) => reaction.emoji === emoji,
      );
      const otherActiveChoices = activeChoices.filter(
        (reaction) => reaction.emoji !== emoji,
      );
      if (alreadySelected && otherActiveChoices.length === 0) {
        return;
      }

      setErrorMessage(null);
      setPending(true);
      setOptimisticState((current) => {
        const baseReactions =
          current && current.sourceReactions === sourceReactions
            ? current.reactions
            : reactions;

        return {
          reactions: replaceOwnChoiceReactions(baseReactions, choices, emoji),
          sourceReactions,
        };
      });
      try {
        for (const reaction of otherActiveChoices) {
          await onToggleReaction(message, reaction.emoji, true);
        }
        if (!alreadySelected) {
          await onToggleReaction(message, emoji, false);
        }
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
      reactions,
      sourceReactions,
    ],
  );

  return {
    reactions,
    canToggle,
    pending,
    errorMessage,
    select,
    chooseExclusive,
  };
}
