import * as React from "react";

import { getMentionOffsets } from "@/features/messages/lib/hasMention";
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

function buildMentionRemovalEdits(
  text: string,
  displayNames: readonly string[],
  queryStart: number,
  cursor: number,
): AutocompleteEdit[] {
  const ranges = displayNames.flatMap((displayName) =>
    getMentionOffsets(text, displayName).map((start) => {
      let end = start + `@${displayName}`.length;
      if (text[end] === " ") end += 1;
      return { start, end };
    }),
  );
  ranges.push({
    start: Math.max(0, Math.min(queryStart, text.length)),
    end: Math.max(0, Math.min(cursor, text.length)),
  });

  const merged = ranges
    .filter(({ start, end }) => start < end)
    .sort((left, right) => left.start - right.start)
    .reduce<Array<{ start: number; end: number }>>((result, range) => {
      const previous = result.at(-1);
      if (previous && range.start <= previous.end) {
        previous.end = Math.max(previous.end, range.end);
      } else {
        result.push({ ...range });
      }
      return result;
    }, []);

  return merged.reverse().map(({ start, end }) => ({
    replaceFromOffset: start,
    replaceToOffset: end,
    insertText: "",
  }));
}

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
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      const { text, cursor } = richText.getPlainTextAndCursor();
      const matchingDisplayNames = mentions
        .getDraftMentionRefs(text)
        .filter((ref) => normalizePubkey(ref.pubkey) === pubkey)
        .map((ref) => ref.displayName);
      mentions.cancelMentionAutocomplete();
      for (const edit of buildMentionRemovalEdits(
        text,
        matchingDisplayNames,
        mentions.mentionStartIndex,
        cursor,
      )) {
        applyAutocompleteEdit(edit);
      }
      if (!lockedAgentPubkeys.has(pubkey)) {
        audience.addPubkey(pubkey);
      }
      onPulseAddressLock(pubkey);
    },
    [
      applyAutocompleteEdit,
      audience.addPubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.cancelMentionAutocomplete,
      mentions.getDraftMentionRefs,
      mentions.mentionStartIndex,
      onPulseAddressLock,
      richText.getPlainTextAndCursor,
    ],
  );

  const selectMentionSuggestion = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (suggestion.isAgent && pubkey && lockedAgentPubkeys.has(pubkey)) {
        alwaysMentionAgent(suggestion);
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
