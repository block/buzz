import * as React from "react";

import { getMentionOffsets } from "@/features/messages/lib/hasMention";
import { stripImplicitAgentMentionPrefix } from "@/features/messages/lib/stripImplicitAgentMentions";
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

export function useAgentAddressLockPicker({
  applyAutocompleteEdit,
  audience,
  audienceScope,
  mentions,
  onAddressAgentMention,
  onAutoPinAgentMention,
  onImplicitPrefixInserted,
  onImplicitPrefixRemoved,
  onPulseAddressLock,
  profiles,
  richText,
}: {
  applyAutocompleteEdit: (edit: AutocompleteEdit) => void;
  audience: ReturnType<typeof usePersistentAgentAudience>;
  audienceScope: string | null;
  mentions: UseMentionsResult;
  onAddressAgentMention?: (suggestion: MentionSuggestion) => void;
  onAutoPinAgentMention?: (
    suggestion: MentionSuggestion,
    options: { reinstateExcluded: boolean },
  ) => void;
  /** Records generated mention provenance at the insertion boundary. */
  onImplicitPrefixInserted?: (
    mentions: readonly { pubkey: string; prefix: string }[],
  ) => void;
  /** Removes generated mention provenance by its stable identity. */
  onImplicitPrefixRemoved?: (pubkey: string) => void;
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
  React.useEffect(() => {
    for (const pubkey of lockedAgentPubkeys) {
      unpinnedAgentPubkeysRef.current.delete(pubkey);
    }
  }, [lockedAgentPubkeys]);
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
  React.useLayoutEffect(() => {
    richText.syncAddressedAgentMentionNames?.(
      lockedAgents.map((agent) => agent.displayName),
    );
  }, [lockedAgents, richText.syncAddressedAgentMentionNames]);
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
          const excludePubkey = audience.excludePubkey ?? audience.removePubkey;
          onImplicitPrefixRemoved?.(pubkey);
          excludePubkey(pubkey);
        }
      }
      visibleAgentMentionPubkeysRef.current = presentAgentPubkeys;
    },
    [
      audience.excludePubkey,
      audience.removePubkey,
      audienceScope,
      lockedAgentPubkeys,
      mentions.getDraftMentionRefs,
      onImplicitPrefixRemoved,
    ],
  );

  const unpinAddressedAgent = React.useCallback(
    (pubkey: string) => {
      const normalized = normalizePubkey(pubkey);
      if (!audienceScope || !normalized) return;
      unpinnedAgentPubkeysRef.current.add(normalized);
      const excludePubkey = audience.excludePubkey ?? audience.removePubkey;
      excludePubkey(normalized);
    },
    [audience.excludePubkey, audience.removePubkey, audienceScope],
  );
  const removeAddressedAgent = React.useCallback(
    (pubkey: string) => {
      const normalized = normalizePubkey(pubkey);
      if (!audienceScope || !normalized) return;
      unpinAddressedAgent(normalized);
      const displayName = lockedAgents.find(
        (agent) => agent.pubkey === normalized,
      )?.displayName;
      if (displayName) {
        const text = richText.getPlainTextAndCursor().text;
        const implicitPrefix = `@${displayName}${text === `@${displayName}` ? "" : " "}`;
        const strippedText = stripImplicitAgentMentionPrefix(
          text,
          implicitPrefix,
        );
        if (strippedText !== text) {
          onImplicitPrefixRemoved?.(normalized);
          applyAutocompleteEdit({
            replaceFromOffset: 0,
            replaceToOffset: text.length - strippedText.length,
            insertText: "",
          });
        }
      }
    },
    [
      applyAutocompleteEdit,
      audienceScope,
      lockedAgents,
      onImplicitPrefixRemoved,
      richText.getPlainTextAndCursor,
      unpinAddressedAgent,
    ],
  );
  const toggleAlwaysAddressAgent = React.useCallback(
    (
      suggestion: MentionSuggestion,
      options: { preserveMention?: boolean } = {},
    ) => {
      const pubkey = normalizePubkey(suggestion.pubkey ?? "");
      if (!audienceScope || !pubkey || !suggestion.isAgent) return;

      if (lockedAgentPubkeys.has(pubkey)) {
        if (options.preserveMention) {
          unpinAddressedAgent(pubkey);
        } else {
          removeAddressedAgent(pubkey);
        }
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
          const insertedText = `@${suggestion.displayName} `;
          onImplicitPrefixInserted?.([{ pubkey, prefix: insertedText }]);
          applyAutocompleteEdit({
            replaceFromOffset: 0,
            replaceToOffset: 0,
            insertText: insertedText,
            preserveSelection: text.length > 0,
            reassertMentionCaret: false,
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

      if (mentions.isMentionOpen) {
        const { text, cursor } = richText.getPlainTextAndCursor();
        if (mentions.isInlineMentionSelection()) {
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
            replaceToOffset: Math.max(
              queryStart,
              Math.min(cursor, text.length),
            ),
            insertText: "",
          });
          mentions.openMentionPicker(queryStart, "preserve");
        } else {
          mentions.openMentionPicker(cursor, "preserve");
        }
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
      onImplicitPrefixInserted,
      onPulseAddressLock,
      removeAddressedAgent,
      richText.getPlainTextAndCursor,
      trackMentionAddressedAgent,
      unpinAddressedAgent,
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
          trackMentionAddressedAgent(pubkey);
          onAutoPinAgentMention?.(suggestion, {
            reinstateExcluded: !wasUnpinned,
          });
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
      // A profile can rename an agent while this draft is off-screen. Mention
      // refs retain the identity of its already-inserted automatic prefix, so
      // use that identity as well as the current display name when deciding
      // whether restoration is needed.
      const presentAgentPubkeys = new Set(
        mentions
          .getDraftMentionRefs(text)
          .filter((ref) => ref.isAgent)
          .map((ref) => normalizePubkey(ref.pubkey)),
      );
      for (const agent of targetAgents) {
        if (
          presentAgentPubkeys.has(agent.pubkey) ||
          getMentionOffsets(text, agent.displayName).length > 0
        ) {
          mentions.registerMentionPubkey(agent.displayName, agent.pubkey, {
            isAgent: true,
          });
          visibleAgentMentionPubkeysRef.current.add(agent.pubkey);
        }
      }
      const missingAgents = targetAgents.filter(
        (agent) =>
          (!unpinnedAgentPubkeysRef.current.has(agent.pubkey) ||
            allowedUnpinned.has(agent.pubkey)) &&
          !presentAgentPubkeys.has(agent.pubkey) &&
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
      onImplicitPrefixInserted?.(
        missingAgents.map((agent) => ({
          pubkey: agent.pubkey,
          prefix: `@${agent.displayName} `,
        })),
      );
      applyAutocompleteEdit({
        replaceFromOffset: 0,
        replaceToOffset: 0,
        insertText: insertedText,
        preserveSelection: true,
      });
      // A restored empty composer has no authored caret to preserve. Move it
      // to the real document end after insertion so WebKit places it after
      // the trailing space, without routing the multi-word name back through
      // autocomplete settlement (which would lose its mention decoration).
      if (text.length === 0) richText.focusEnd();
      return `${insertedText}${text}`;
    },
    [
      applyAutocompleteEdit,
      audience.pubkeys,
      mentions.getDraftMentionRefs,
      mentions.getMentionDisplayName,
      mentions.registerMentionPubkey,
      onImplicitPrefixInserted,
      profiles,
      richText.focusEnd,
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
