import * as React from "react";

import { getMentionOffsets } from "@/features/messages/lib/hasMention";
import type { usePersistentAgentAudience } from "@/features/messages/lib/persistentAgentAudience";
import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import type {
  AutocompleteEdit,
  UseRichTextEditorResult,
} from "@/features/messages/lib/useRichTextEditor";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { detectPrefixQuery } from "@/shared/lib/detectPrefixQuery";
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
  const lockedAgentNamesRef = React.useRef(new Map<string, string>());
  const [announcement, setAnnouncement] = React.useState("");
  const lockedAgents = React.useMemo<ComposerAddressLock[]>(
    () =>
      audience.pubkeys.map((pubkey) => {
        const normalized = normalizePubkey(pubkey);
        const profile = profiles?.[normalized];
        const resolvedDisplayName =
          profile?.displayName?.trim() ||
          profile?.name?.trim() ||
          profile?.nip05Handle?.trim() ||
          mentions.getMentionDisplayName(normalized)?.trim();
        if (resolvedDisplayName) {
          lockedAgentNamesRef.current.set(normalized, resolvedDisplayName);
        }
        return {
          pubkey: normalized,
          displayName:
            resolvedDisplayName ??
            lockedAgentNamesRef.current.get(normalized) ??
            truncatePubkey(normalized),
          avatarUrl: profile?.avatarUrl ?? null,
        };
      }),
    [audience.pubkeys, mentions.getMentionDisplayName, profiles],
  );
  const consumeAddressSuggestion = React.useCallback(
    (
      suggestion: MentionSuggestion,
      { removeInlineMentions }: { removeInlineMentions: boolean },
    ): string | null => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (!audienceScope || !pubkey || !suggestion.isAgent) return null;

      const { text, cursor } = richText.getPlainTextAndCursor();
      const matchingDisplayNames = removeInlineMentions
        ? mentions
            .getDraftMentionRefs(text)
            .filter((ref) => normalizePubkey(ref.pubkey) === pubkey)
            .map((ref) => ref.displayName)
        : [];
      mentions.cancelMentionAutocomplete();
      for (const edit of buildMentionRemovalEdits(
        text,
        matchingDisplayNames,
        mentions.mentionStartIndex,
        cursor,
      )) {
        applyAutocompleteEdit(edit);
      }
      return pubkey;
    },
    [
      applyAutocompleteEdit,
      audienceScope,
      mentions.cancelMentionAutocomplete,
      mentions.getDraftMentionRefs,
      mentions.mentionStartIndex,
      richText.getPlainTextAndCursor,
    ],
  );
  const toggleAlwaysAddressAgent = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      if (lockedAgentPubkeys.has(pubkey)) {
        audience.removePubkey(pubkey);
        setAnnouncement(`Stopped always addressing ${suggestion.displayName}`);
      } else {
        audience.addPubkey(pubkey);
        onPulseAddressLock(pubkey);
        setAnnouncement(`Always addressing ${suggestion.displayName}`);
      }

      if (mentions.isMentionOpen) {
        const { text, cursor } = richText.getPlainTextAndCursor();
        const activeMention = detectPrefixQuery("@", text, cursor, [
          suggestion.displayName.toLowerCase(),
        ]);
        const queryStart = Math.max(
          0,
          Math.min(
            activeMention?.startIndex ?? mentions.mentionStartIndex,
            text.length,
          ),
        );
        applyAutocompleteEdit({
          replaceFromOffset: queryStart,
          replaceToOffset: Math.max(queryStart, Math.min(cursor, text.length)),
          insertText: "",
        });
        mentions.openMentionPicker(queryStart, "preserve");
      }
    },
    [
      applyAutocompleteEdit,
      audience.addPubkey,
      audience.removePubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.isMentionOpen,
      mentions.mentionStartIndex,
      mentions.openMentionPicker,
      onPulseAddressLock,
      richText.getPlainTextAndCursor,
    ],
  );

  const selectMentionSuggestion = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (suggestion.isAgent && pubkey && lockedAgentPubkeys.has(pubkey)) {
        consumeAddressSuggestion(suggestion, { removeInlineMentions: false });
        onPulseAddressLock(pubkey);
        return;
      }

      const { cursor } = richText.getPlainTextAndCursor();
      applyAutocompleteEdit(mentions.insertMention(suggestion, cursor));
    },
    [
      applyAutocompleteEdit,
      consumeAddressSuggestion,
      lockedAgentPubkeys,
      mentions.insertMention,
      onPulseAddressLock,
      richText.getPlainTextAndCursor,
    ],
  );

  return {
    announcement,
    lockedAgents,
    lockedAgentPubkeys,
    selectMentionSuggestion,
    toggleAlwaysAddressAgent,
  };
}
