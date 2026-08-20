import * as React from "react";

import type { AutocompleteEdit } from "@/features/messages/lib/useRichTextEditor";
import type { ChannelSuggestion } from "@/features/messages/lib/useChannelLinks";
import type { EmojiSuggestion } from "@/features/messages/lib/useEmojiAutocomplete";
import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";

/**
 * The three autocomplete inserters (mention, channel link, custom emoji).
 *
 * They are identical in shape — read the cursor, ask the relevant
 * autocomplete for an edit, apply it — and differ only in which autocomplete
 * they consult. Extracted from `MessageComposer` as a sibling so the composer
 * stays under the file-size guard, matching the pattern the other composer
 * siblings already follow.
 */
export function useAutocompleteInserts(input: {
  replacePlainTextRange: (
    from: number,
    to: number,
    text: string,
    customEmojiShortcode?: string,
  ) => void;
  getPlainTextAndCursor: () => { cursor: number };
  insertMention: (
    suggestion: MentionSuggestion,
    cursor: number,
  ) => AutocompleteEdit;
  insertChannel: (
    suggestion: ChannelSuggestion,
    cursor: number,
  ) => AutocompleteEdit;
  insertEmoji: (
    suggestion: EmojiSuggestion,
    cursor: number,
  ) => AutocompleteEdit;
}) {
  const {
    replacePlainTextRange,
    getPlainTextAndCursor,
    insertMention,
    insertChannel,
    insertEmoji,
  } = input;

  const applyAutocompleteEdit = React.useCallback(
    (edit: AutocompleteEdit) => {
      replacePlainTextRange(
        edit.replaceFromOffset,
        edit.replaceToOffset,
        edit.insertText,
        edit.customEmojiShortcode,
      );
    },
    [replacePlainTextRange],
  );

  const applyMentionInsert = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const { cursor } = getPlainTextAndCursor();
      applyAutocompleteEdit(insertMention(suggestion, cursor));
    },
    [applyAutocompleteEdit, insertMention, getPlainTextAndCursor],
  );

  const applyChannelInsert = React.useCallback(
    (suggestion: ChannelSuggestion) => {
      const { cursor } = getPlainTextAndCursor();
      applyAutocompleteEdit(insertChannel(suggestion, cursor));
    },
    [applyAutocompleteEdit, insertChannel, getPlainTextAndCursor],
  );

  const applyEmojiInsert = React.useCallback(
    (suggestion: EmojiSuggestion) => {
      const { cursor } = getPlainTextAndCursor();
      applyAutocompleteEdit(insertEmoji(suggestion, cursor));
    },
    [applyAutocompleteEdit, insertEmoji, getPlainTextAndCursor],
  );

  return {
    applyAutocompleteEdit,
    applyMentionInsert,
    applyChannelInsert,
    applyEmojiInsert,
  };
}
