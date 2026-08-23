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
import type { ComposerAddressAgent } from "./ComposerAddressControls";
import type { MentionSuggestion } from "./MentionAutocomplete";

function buildMentionRemovalEdits(
  text: string,
  displayNames: readonly string[],
  queryRange?: { start: number; end: number },
): AutocompleteEdit[] {
  const ranges = displayNames.flatMap((displayName) =>
    getMentionOffsets(text, displayName).map((start) => {
      let end = start + `@${displayName}`.length;
      if (text[end] === " ") end += 1;
      return { start, end };
    }),
  );
  if (queryRange) {
    ranges.push({
      start: Math.max(0, Math.min(queryRange.start, text.length)),
      end: Math.max(0, Math.min(queryRange.end, text.length)),
    });
  }

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
  const unpinnedAgentPubkeysRef = React.useRef(new Set<string>());
  const unpinnedAudienceScopeRef = React.useRef(audienceScope);
  if (unpinnedAudienceScopeRef.current !== audienceScope) {
    unpinnedAudienceScopeRef.current = audienceScope;
    unpinnedAgentPubkeysRef.current.clear();
  }
  const lockedAgentNamesRef = React.useRef(new Map<string, string>());
  const [announcement, setAnnouncement] = React.useState("");
  const lockedAgents = React.useMemo<ComposerAddressAgent[]>(
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
  const removeAddressedAgent = React.useCallback(
    (pubkey: string) => {
      const normalized = normalizePubkey(pubkey);
      if (!audienceScope || !normalized) return;
      const { text } = richText.getPlainTextAndCursor();
      const matchingDisplayNames = mentions
        .getDraftMentionRefs(text)
        .filter((ref) => normalizePubkey(ref.pubkey) === normalized)
        .map((ref) => ref.displayName);
      for (const edit of buildMentionRemovalEdits(text, matchingDisplayNames)) {
        applyAutocompleteEdit(edit);
      }
      unpinnedAgentPubkeysRef.current.add(normalized);
      audience.removePubkey(normalized);
    },
    [
      applyAutocompleteEdit,
      audience.removePubkey,
      audienceScope,
      mentions.getDraftMentionRefs,
      richText.getPlainTextAndCursor,
    ],
  );
  const toggleAlwaysAddressAgent = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      if (lockedAgentPubkeys.has(pubkey)) {
        removeAddressedAgent(pubkey);
        setAnnouncement(
          `Stopped automatically mentioning ${suggestion.displayName}`,
        );
      } else {
        unpinnedAgentPubkeysRef.current.delete(pubkey);
        audience.addPubkey(pubkey);
        mentions.registerMentionPubkey(suggestion.displayName, pubkey, {
          isAgent: true,
        });
        const { text } = richText.getPlainTextAndCursor();
        if (getMentionOffsets(text, suggestion.displayName).length === 0) {
          applyAutocompleteEdit({
            replaceFromOffset: 0,
            replaceToOffset: 0,
            insertText: `@${suggestion.displayName} `,
          });
        }
        onPulseAddressLock(pubkey);
        setAnnouncement(`Automatically mentioning ${suggestion.displayName}`);
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
      audienceScope,
      lockedAgentPubkeys,
      mentions.isMentionOpen,
      mentions.mentionStartIndex,
      mentions.openMentionPicker,
      mentions.registerMentionPubkey,
      onPulseAddressLock,
      removeAddressedAgent,
      richText.getPlainTextAndCursor,
    ],
  );

  const selectMentionSuggestion = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (suggestion.isAgent && pubkey && audienceScope) {
        const { cursor } = richText.getPlainTextAndCursor();
        const wasUnpinned =
          !lockedAgentPubkeys.has(pubkey) &&
          unpinnedAgentPubkeysRef.current.has(pubkey);
        if (mentions.isInlineMentionSelection() || wasUnpinned) {
          applyAutocompleteEdit(mentions.insertMention(suggestion, cursor));
          return;
        }

        applyAutocompleteEdit(mentions.insertMention(suggestion, cursor));
        if (!lockedAgentPubkeys.has(pubkey)) {
          audience.addPubkey(pubkey);
          setAnnouncement(`Automatically mentioning ${suggestion.displayName}`);
        }
        onPulseAddressLock(pubkey);
        return;
      }

      const { cursor } = richText.getPlainTextAndCursor();
      applyAutocompleteEdit(mentions.insertMention(suggestion, cursor));
    },
    [
      applyAutocompleteEdit,
      audience.addPubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.isInlineMentionSelection,
      mentions.insertMention,
      onPulseAddressLock,
      richText.getPlainTextAndCursor,
    ],
  );

  const restoreAddressedAgentMentions = React.useCallback(
    (
      pubkeys?: readonly string[],
      allowedUnpinnedPubkeys: readonly string[] = [],
    ) => {
      const restorePubkeys = pubkeys
        ? new Set(pubkeys.map(normalizePubkey))
        : null;
      const allowedUnpinned = new Set(
        allowedUnpinnedPubkeys.map(normalizePubkey),
      );
      const currentAudiencePubkeys = new Set(
        audience.pubkeys.map(normalizePubkey),
      );
      const targetAgents = [...(restorePubkeys ?? currentAudiencePubkeys)]
        .filter(
          (pubkey) =>
            currentAudiencePubkeys.has(pubkey) || allowedUnpinned.has(pubkey),
        )
        .map((pubkey) => {
          const profile = profiles?.[pubkey];
          const displayName =
            profile?.displayName?.trim() ||
            profile?.name?.trim() ||
            profile?.nip05Handle?.trim() ||
            mentions.getMentionDisplayName(pubkey)?.trim() ||
            lockedAgentNamesRef.current.get(pubkey) ||
            truncatePubkey(pubkey);
          return { pubkey, displayName };
        });
      const { text } = richText.getPlainTextAndCursor();
      const missingAgents = targetAgents.filter(
        (agent) =>
          (!unpinnedAgentPubkeysRef.current.has(agent.pubkey) ||
            allowedUnpinned.has(agent.pubkey)) &&
          getMentionOffsets(text, agent.displayName).length === 0,
      );
      if (missingAgents.length === 0) return text;
      for (const agent of missingAgents) {
        mentions.registerMentionPubkey(agent.displayName, agent.pubkey, {
          isAgent: true,
        });
      }
      const insertedText = `${missingAgents
        .map((agent) => `@${agent.displayName}`)
        .join(" ")} `;
      applyAutocompleteEdit({
        replaceFromOffset: 0,
        replaceToOffset: 0,
        insertText: insertedText,
      });
      return `${insertedText}${text}`;
    },
    [
      applyAutocompleteEdit,
      audience.pubkeys,
      mentions.getMentionDisplayName,
      mentions.registerMentionPubkey,
      profiles,
      richText.getPlainTextAndCursor,
    ],
  );

  return {
    announcement,
    lockedAgents,
    lockedAgentPubkeys,
    removeAddressedAgent,
    restoreAddressedAgentMentions,
    selectMentionSuggestion,
    toggleAlwaysAddressAgent,
  };
}
