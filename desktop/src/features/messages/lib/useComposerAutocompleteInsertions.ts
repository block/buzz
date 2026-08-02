import * as React from "react";

import type { AgentAvailableCommand } from "@/features/agents/lib/agentAvailableCommands";
import type { ChannelSuggestion } from "./useChannelLinks";
import type { EmojiSuggestion } from "./useEmojiAutocomplete";
import type { AutocompleteEdit } from "./useRichTextEditor";
import type { MentionSuggestion } from "../ui/MentionAutocomplete";

type InsertSuggestion<T> = (
  suggestion: T,
  selectionEnd: number,
) => AutocompleteEdit;

type ComposerAutocompleteInsertionsOptions = {
  getCursor: () => { cursor: number };
  replacePlainTextRange: (
    replaceFromOffset: number,
    replaceToOffset: number,
    insertText: string,
    customEmojiShortcode?: string,
  ) => void;
  insertMention: InsertSuggestion<MentionSuggestion>;
  insertChannel: InsertSuggestion<ChannelSuggestion>;
  insertEmoji: InsertSuggestion<EmojiSuggestion>;
  insertSlashCommand: InsertSuggestion<AgentAvailableCommand>;
};

export function useComposerAutocompleteInsertions({
  getCursor,
  replacePlainTextRange,
  insertMention,
  insertChannel,
  insertEmoji,
  insertSlashCommand,
}: ComposerAutocompleteInsertionsOptions) {
  const applyEdit = React.useCallback(
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
    (suggestion: MentionSuggestion) =>
      applyEdit(insertMention(suggestion, getCursor().cursor)),
    [applyEdit, getCursor, insertMention],
  );
  const applyChannelInsert = React.useCallback(
    (suggestion: ChannelSuggestion) =>
      applyEdit(insertChannel(suggestion, getCursor().cursor)),
    [applyEdit, getCursor, insertChannel],
  );
  const applyEmojiInsert = React.useCallback(
    (suggestion: EmojiSuggestion) =>
      applyEdit(insertEmoji(suggestion, getCursor().cursor)),
    [applyEdit, getCursor, insertEmoji],
  );
  const applySlashCommandInsert = React.useCallback(
    (suggestion: AgentAvailableCommand) =>
      applyEdit(insertSlashCommand(suggestion, getCursor().cursor)),
    [applyEdit, getCursor, insertSlashCommand],
  );

  return {
    applyMentionInsert,
    applyChannelInsert,
    applyEmojiInsert,
    applySlashCommandInsert,
  };
}
