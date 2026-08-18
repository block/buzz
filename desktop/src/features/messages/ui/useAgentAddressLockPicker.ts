import * as React from "react";

import type { usePersistentAgentAudience } from "@/features/messages/lib/persistentAgentAudience";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type {
  AutocompleteEdit,
  UseRichTextEditorResult,
} from "@/features/messages/lib/useRichTextEditor";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import type { ComposerAddressLock } from "./ComposerAddressLocks";
import type { MentionSuggestion } from "./MentionAutocomplete";

export function useAgentAddressLockPicker({
  applyAutocompleteEdit,
  audience,
  audienceScope,
  mentions,
  onPulseAddressLock,
  profiles,
  richText,
}: {
  applyAutocompleteEdit: (edit: AutocompleteEdit) => void;
  audience: ReturnType<typeof usePersistentAgentAudience>;
  audienceScope: string | null;
  mentions: UseMentionsResult;
  onPulseAddressLock: (pubkey: string) => void;
  profiles?: UserProfileLookup;
  richText: UseRichTextEditorResult;
}) {
  const lockedAgentPubkeys = React.useMemo(
    () => new Set(audience.pubkeys),
    [audience.pubkeys],
  );
  const lockedAgents = React.useMemo<ComposerAddressLock[]>(
    () =>
      audience.pubkeys.map((pubkey) => {
        const normalized = normalizePubkey(pubkey);
        return {
          pubkey: normalized,
          displayName:
            mentions.getMentionDisplayName(normalized) ??
            truncatePubkey(normalized),
          avatarUrl: profiles?.[normalized]?.avatarUrl ?? null,
        };
      }),
    [audience.pubkeys, mentions.getMentionDisplayName, profiles],
  );
  const alwaysMentionAgent = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = suggestion.pubkey?.trim().toLowerCase();
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      const { cursor } = richText.getPlainTextAndCursor();
      mentions.cancelMentionAutocomplete();
      applyAutocompleteEdit({
        replaceFromOffset: mentions.mentionStartIndex,
        replaceToOffset: cursor,
        insertText: "",
      });
      if (!lockedAgentPubkeys.has(pubkey)) {
        audience.addPubkey(pubkey);
      }
    },
    [
      applyAutocompleteEdit,
      audience.addPubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.cancelMentionAutocomplete,
      mentions.mentionStartIndex,
      richText.getPlainTextAndCursor,
    ],
  );

  const selectMentionSuggestion = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (suggestion.isAgent && pubkey && lockedAgentPubkeys.has(pubkey)) {
        alwaysMentionAgent(suggestion);
        onPulseAddressLock(pubkey);
        return;
      }

      const { cursor } = richText.getPlainTextAndCursor();
      applyAutocompleteEdit(mentions.insertMention(suggestion, cursor));
    },
    [
      alwaysMentionAgent,
      applyAutocompleteEdit,
      lockedAgentPubkeys,
      mentions.insertMention,
      onPulseAddressLock,
      richText.getPlainTextAndCursor,
    ],
  );

  return {
    alwaysMentionAgent,
    lockedAgents,
    lockedAgentPubkeys,
    selectMentionSuggestion,
  };
}
