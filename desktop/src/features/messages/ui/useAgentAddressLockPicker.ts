import * as React from "react";

import type { usePersistentAgentAudience } from "@/features/messages/lib/persistentAgentAudience";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type {
  AutocompleteEdit,
  UseRichTextEditorResult,
} from "@/features/messages/lib/useRichTextEditor";
import type { MentionSuggestion } from "./MentionAutocomplete";

export function useAgentAddressLockPicker({
  applyAutocompleteEdit,
  audience,
  audienceScope,
  mentions,
  richText,
}: {
  applyAutocompleteEdit: (edit: AutocompleteEdit) => void;
  audience: ReturnType<typeof usePersistentAgentAudience>;
  audienceScope: string | null;
  mentions: UseMentionsResult;
  richText: UseRichTextEditorResult;
}) {
  const lockedAgentPubkeys = React.useMemo(
    () => new Set(audience.pubkeys),
    [audience.pubkeys],
  );
  const toggleAgentAddressLock = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = suggestion.pubkey?.trim().toLowerCase();
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      const { text, cursor } = richText.getPlainTextAndCursor();
      if (lockedAgentPubkeys.has(pubkey)) {
        audience.removePubkey(pubkey);
        mentions.openMentionPicker(cursor);
        return;
      }

      const addressedPubkeys = new Set(mentions.extractMentionPubkeys(text));
      if (!addressedPubkeys.has(pubkey)) {
        const edit = mentions.insertResolvedMention({
          displayName: suggestion.displayName,
          pubkey,
          replaceFromOffset: cursor,
          replaceToOffset: cursor,
          isAgent: true,
        });
        const previousChar = text.slice(0, cursor).slice(-1);
        applyAutocompleteEdit({
          ...edit,
          insertText: `${previousChar && !/\s/.test(previousChar) ? " " : ""}${edit.insertText}`,
        });
      }
      audience.addPubkey(pubkey);
      const { cursor: updatedCursor } = richText.getPlainTextAndCursor();
      mentions.openMentionPicker(updatedCursor);
    },
    [
      applyAutocompleteEdit,
      audience.addPubkey,
      audience.removePubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.extractMentionPubkeys,
      mentions.insertResolvedMention,
      mentions.openMentionPicker,
      richText.getPlainTextAndCursor,
    ],
  );

  return { lockedAgentPubkeys, toggleAgentAddressLock };
}
