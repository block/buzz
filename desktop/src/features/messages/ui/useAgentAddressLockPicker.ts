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
  onAddressAgentMention,
  onAutoPinAgentMention,
  onPulseAddressLock,
  profiles,
  richText,
}: {
  applyAutocompleteEdit: (edit: AutocompleteEdit) => void;
  audience: ReturnType<typeof usePersistentAgentAudience>;
  audienceScope: string | null;
  mentions: UseMentionsResult;
  onAddressAgentMention?: (suggestion: MentionSuggestion) => void;
  onAutoPinAgentMention?: (suggestion: MentionSuggestion) => void;
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
  const visibleAgentMentionPubkeysRef = React.useRef(new Set<string>());
  const mentionSyncScopeRef = React.useRef(audienceScope);
  if (mentionSyncScopeRef.current !== audienceScope) {
    mentionSyncScopeRef.current = audienceScope;
    visibleAgentMentionPubkeysRef.current.clear();
  }
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
  const trackMentionAddressedAgent = React.useCallback(
    (pubkey: string) => {
      const normalized = normalizePubkey(pubkey);
      if (audienceScope && normalized) {
        visibleAgentMentionPubkeysRef.current.add(normalized);
      }
    },
    [audienceScope],
  );
  const syncAddressedAgentsFromText = React.useCallback(
    (text: string) => {
      if (!audienceScope) return;
      const presentAgentPubkeys = new Set(
        mentions
          .getDraftMentionRefs(text)
          .filter((ref) => ref.isAgent)
          .map((ref) => normalizePubkey(ref.pubkey)),
      );
      for (const pubkey of visibleAgentMentionPubkeysRef.current) {
        if (
          !presentAgentPubkeys.has(pubkey) &&
          lockedAgentPubkeys.has(pubkey)
        ) {
          audience.removePubkey(pubkey);
        }
      }
      visibleAgentMentionPubkeysRef.current = presentAgentPubkeys;
    },
    [
      audience.removePubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.getDraftMentionRefs,
    ],
  );

  const removeAddressedAgent = React.useCallback(
    (pubkey: string) => {
      const normalized = normalizePubkey(pubkey);
      if (!audienceScope || !normalized) return;
      unpinnedAgentPubkeysRef.current.add(normalized);
      audience.removePubkey(normalized);
    },
    [audience.removePubkey, audienceScope],
  );
  const removeAddressedAgentMentions = React.useCallback(
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
      removeAddressedAgent(normalized);
    },
    [
      applyAutocompleteEdit,
      audienceScope,
      mentions.getDraftMentionRefs,
      removeAddressedAgent,
      richText.getPlainTextAndCursor,
    ],
  );
  const toggleAlwaysAddressAgent = React.useCallback(
    (suggestion: MentionSuggestion) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      if (lockedAgentPubkeys.has(pubkey)) {
        removeAddressedAgentMentions(pubkey);
        setAnnouncement(
          `Stopped automatically mentioning ${suggestion.displayName}`,
        );
      } else {
        unpinnedAgentPubkeysRef.current.delete(pubkey);
        mentions.registerMentionPubkey(suggestion.displayName, pubkey, {
          isAgent: true,
        });
        const { text } = richText.getPlainTextAndCursor();
        if (getMentionOffsets(text, suggestion.displayName).length === 0) {
          applyAutocompleteEdit({
            replaceFromOffset: 0,
            replaceToOffset: 0,
            insertText: `@${suggestion.displayName} `,
            preserveSelection: true,
          });
        }
        trackMentionAddressedAgent(pubkey);
        if (onAddressAgentMention) {
          onAddressAgentMention(suggestion);
        } else {
          audience.addPubkey(pubkey);
          onPulseAddressLock(pubkey);
        }
        setAnnouncement(`Automatically mentioning ${suggestion.displayName}`);
      }

      if (mentions.isMentionOpen && mentions.isInlineMentionSelection()) {
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
      mentions.isInlineMentionSelection,
      mentions.isMentionOpen,
      mentions.mentionStartIndex,
      mentions.openMentionPicker,
      mentions.registerMentionPubkey,
      onAddressAgentMention,
      onPulseAddressLock,
      removeAddressedAgentMentions,
      richText.getPlainTextAndCursor,
      trackMentionAddressedAgent,
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
          if (wasUnpinned) unpinnedAgentPubkeysRef.current.delete(pubkey);
          trackMentionAddressedAgent(pubkey);
          onAutoPinAgentMention?.(suggestion);
          return;
        }

        applyAutocompleteEdit(mentions.insertMention(suggestion, cursor));
        if (!lockedAgentPubkeys.has(pubkey)) {
          trackMentionAddressedAgent(pubkey);
          if (onAddressAgentMention) {
            onAddressAgentMention(suggestion);
          } else {
            audience.addPubkey(pubkey);
            onPulseAddressLock(pubkey);
          }
          setAnnouncement(`Automatically mentioning ${suggestion.displayName}`);
        } else {
          onPulseAddressLock(pubkey);
        }
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
      onAddressAgentMention,
      onAutoPinAgentMention,
      onPulseAddressLock,
      richText.getPlainTextAndCursor,
      trackMentionAddressedAgent,
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
      for (const agent of targetAgents) {
        if (getMentionOffsets(text, agent.displayName).length > 0) {
          visibleAgentMentionPubkeysRef.current.add(agent.pubkey);
        }
      }
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
        visibleAgentMentionPubkeysRef.current.add(agent.pubkey);
      }
      const insertedText = `${missingAgents
        .map((agent) => `@${agent.displayName}`)
        .join(" ")} `;
      applyAutocompleteEdit({
        replaceFromOffset: 0,
        replaceToOffset: 0,
        insertText: insertedText,
        preserveSelection: true,
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
    syncAddressedAgentsFromText,
    toggleAlwaysAddressAgent,
    trackMentionAddressedAgent,
  };
}
