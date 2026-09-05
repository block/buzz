import * as React from "react";

import type { AutocompleteEdit } from "@/features/messages/lib/useRichTextEditor";
import {
  buildSkillInsertion,
  type ComposerAgentSkill,
} from "./composerAgentSkills";
import { useComposerAgentSkills } from "./useComposerAgentSkills";

type AddressedAgent = { displayName: string; pubkey: string };
type PlainTextCursor = { cursor: number; text: string };

export function useComposerInsertionActions<
  ChannelSuggestion,
  EmojiSuggestion,
>({
  addressedAgents,
  applyAutocompleteEdit,
  channelId,
  enabled,
  getPlainTextAndCursor,
  insertChannel,
  insertEmoji,
}: {
  addressedAgents: readonly AddressedAgent[];
  applyAutocompleteEdit: (edit: AutocompleteEdit) => void;
  channelId: string | null;
  enabled: boolean;
  getPlainTextAndCursor: () => PlainTextCursor;
  insertChannel: (
    suggestion: ChannelSuggestion,
    cursor: number,
  ) => AutocompleteEdit;
  insertEmoji: (
    suggestion: EmojiSuggestion,
    cursor: number,
  ) => AutocompleteEdit;
}) {
  const skillAgent =
    enabled && addressedAgents.length === 1 ? addressedAgents[0] : null;
  const skills = useComposerAgentSkills(skillAgent?.pubkey ?? null, channelId);

  const applyChannelInsert = React.useCallback(
    (suggestion: ChannelSuggestion) => {
      const { cursor } = getPlainTextAndCursor();
      applyAutocompleteEdit(insertChannel(suggestion, cursor));
    },
    [applyAutocompleteEdit, getPlainTextAndCursor, insertChannel],
  );
  const applyEmojiInsert = React.useCallback(
    (suggestion: EmojiSuggestion) => {
      const { cursor } = getPlainTextAndCursor();
      applyAutocompleteEdit(insertEmoji(suggestion, cursor));
    },
    [applyAutocompleteEdit, getPlainTextAndCursor, insertEmoji],
  );
  const insertSkill = React.useCallback(
    (skill: ComposerAgentSkill) => {
      const { cursor, text } = getPlainTextAndCursor();
      const edit = buildSkillInsertion(text, cursor, skill.name);
      if (edit) applyAutocompleteEdit(edit);
    },
    [applyAutocompleteEdit, getPlainTextAndCursor],
  );

  return {
    applyChannelInsert,
    applyEmojiInsert,
    insertSkill,
    skillAgentDisplayName: skillAgent?.displayName,
    skills,
  };
}
