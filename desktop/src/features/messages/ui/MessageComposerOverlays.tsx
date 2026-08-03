import * as React from "react";
import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import type { useChannelLinks } from "@/features/messages/lib/useChannelLinks";
import type { EmojiSuggestion } from "@/features/messages/lib/useEmojiAutocomplete";
import type { useEmojiAutocomplete } from "@/features/messages/lib/useEmojiAutocomplete";
import type { useMentions } from "@/features/messages/lib/useMentions";
import { ChannelAutocomplete } from "./ChannelAutocomplete";
import { EmojiAutocomplete } from "./EmojiAutocomplete";
import {
  MentionAutocomplete,
  type MentionSuggestion,
} from "./MentionAutocomplete";

export function useComposerScrollToBottom(
  composerScrollRef: React.RefObject<HTMLDivElement | null>,
) {
  return React.useCallback(() => {
    window.requestAnimationFrame(() => {
      const scrollElement = composerScrollRef.current;
      if (!scrollElement) return;
      scrollElement.scrollTop = scrollElement.scrollHeight;
    });
  }, [composerScrollRef]);
}

export function MessageComposerAutocompletes({
  applyChannelInsert,
  applyEmojiInsert,
  applyMentionInsert,
  channelLinks,
  emojiAutocomplete,
  mentions,
}: {
  applyChannelInsert: (suggestion: ChannelSuggestion) => void;
  applyEmojiInsert: (suggestion: EmojiSuggestion) => void;
  applyMentionInsert: (suggestion: MentionSuggestion) => void;
  channelLinks: ReturnType<typeof useChannelLinks>;
  emojiAutocomplete: ReturnType<typeof useEmojiAutocomplete>;
  mentions: ReturnType<typeof useMentions>;
}) {
  return (
    <>
      <EmojiAutocomplete
        onSelect={applyEmojiInsert}
        selectedIndex={emojiAutocomplete.emojiSelectedIndex}
        suggestions={
          emojiAutocomplete.isEmojiAutocompleteOpen
            ? emojiAutocomplete.emojiSuggestions
            : []
        }
      />
      <ChannelAutocomplete
        onSelect={applyChannelInsert}
        selectedIndex={channelLinks.channelSelectedIndex}
        suggestions={
          channelLinks.isChannelOpen ? channelLinks.channelSuggestions : []
        }
      />
      <MentionAutocomplete
        onFetchMore={mentions.fetchMoreSuggestions}
        onSelect={applyMentionInsert}
        selectedIndex={mentions.mentionSelectedIndex}
        suggestions={mentions.isMentionOpen ? mentions.suggestions : []}
      />
    </>
  );
}

export function MessageComposerUploadError({
  message,
  onDismiss,
}: {
  message: string | null | undefined;
  onDismiss: () => void;
}) {
  if (message == null) return null;
  return (
    <div className="mb-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
      Upload failed: {message}
      <button className="ml-2 underline" onClick={onDismiss} type="button">
        Dismiss
      </button>
    </div>
  );
}
