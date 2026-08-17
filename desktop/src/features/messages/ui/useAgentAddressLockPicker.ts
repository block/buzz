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
  profiles,
  richText,
}: {
  applyAutocompleteEdit: (edit: AutocompleteEdit) => void;
  audience: ReturnType<typeof usePersistentAgentAudience>;
  audienceScope: string | null;
  mentions: UseMentionsResult;
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
  const toggleAgentAddressLock = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = suggestion.pubkey?.trim().toLowerCase();
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      const { cursor } = richText.getPlainTextAndCursor();
      if (lockedAgentPubkeys.has(pubkey)) {
        audience.removePubkey(pubkey);
        mentions.openMentionPicker(cursor);
        return;
      }

      const edit = mentions.insertMention(suggestion, cursor);
      applyAutocompleteEdit({ ...edit, insertText: "" });
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
      mentions.insertMention,
      mentions.openMentionPicker,
      richText.getPlainTextAndCursor,
    ],
  );

  return { lockedAgents, lockedAgentPubkeys, toggleAgentAddressLock };
}
