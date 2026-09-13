import * as React from "react";

import { useAgentCommandCatalog } from "@/features/agents/useAgentCommandCatalog";
import { useChannelMembersQuery } from "@/features/channels/hooks";
import { normalizePubkey, truncateNpub } from "@/shared/lib/pubkey";
import type { AutocompleteEdit } from "./useRichTextEditor";
import type { UseMentionsResult } from "./useMentions";
import {
  buildSlashCommandInsertText,
  buildSlashCommandGroups,
  detectSlashCommandQuery,
  resolveLeadingAgentMentionPubkeys,
  type SlashCommandQuery,
  type SlashCommandSuggestion,
} from "./slashCommandAutocomplete";

type ActiveQuery = {
  detected: SlashCommandQuery;
  selectedAgentPubkeys: readonly string[] | null;
  signature: string;
};

/** Complete advertised agent commands without changing command execution semantics. */
export function useSlashCommandAutocomplete({
  channelId,
  ownerPubkey,
  mentions,
}: {
  channelId: string | null;
  ownerPubkey: string | null;
  mentions: Pick<
    UseMentionsResult,
    "getMentionIdentities" | "extractMentionPubkeys" | "registerMentionPubkey"
  >;
}) {
  const membersQuery = useChannelMembersQuery(channelId, Boolean(channelId));
  const catalog = useAgentCommandCatalog(ownerPubkey);
  const [activeQuery, setActiveQuery] = React.useState<ActiveQuery | null>(
    null,
  );
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const dismissedSignatureRef = React.useRef<string | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: a composer can be reused for a different channel or identity without unmounting
  React.useEffect(() => {
    setActiveQuery(null);
    setSelectedIndex(0);
    dismissedSignatureRef.current = null;
  }, [channelId, ownerPubkey]);

  const providers = React.useMemo(
    () =>
      (membersQuery.data ?? [])
        .filter((member) => member.isAgent || member.role === "bot")
        .map((member) => ({
          pubkey: normalizePubkey(member.pubkey),
          displayName:
            member.displayName?.trim() || truncateNpub(member.pubkey),
        })),
    [membersQuery.data],
  );

  const groups = React.useMemo(
    () =>
      activeQuery
        ? buildSlashCommandGroups({
            catalog,
            providers,
            query: activeQuery.detected.query,
            selectedAgentPubkeys: activeQuery.selectedAgentPubkeys,
          })
        : [],
    [activeQuery, catalog, providers],
  );
  const suggestions = React.useMemo(
    () => groups.flatMap((group) => group.commands),
    [groups],
  );
  const isOpen = activeQuery !== null && suggestions.length > 0;

  React.useEffect(() => {
    setSelectedIndex((current) =>
      suggestions.length === 0 ? 0 : Math.min(current, suggestions.length - 1),
    );
  }, [suggestions.length]);

  const updateQuery = React.useCallback(
    (value: string, cursorPosition: number, isCodeContext = false) => {
      const detected = isCodeContext
        ? null
        : detectSlashCommandQuery(value, cursorPosition);
      if (!detected) {
        dismissedSignatureRef.current = null;
        setActiveQuery(null);
        setSelectedIndex(0);
        return;
      }

      let selectedAgentPubkeys: readonly string[] | null = null;
      if (detected.leadingText) {
        const candidates = mentions.getMentionIdentities().map((identity) => ({
          displayName: identity.label,
          pubkey: identity.pubkey,
        }));
        const leadingPubkeys = resolveLeadingAgentMentionPubkeys(
          detected.leadingText,
          candidates,
        );
        try {
          selectedAgentPubkeys = leadingPubkeys.length
            ? mentions.extractMentionPubkeys(detected.leadingText)
            : [];
        } catch {
          // Ambiguous mentions remain literal; the send flow explains how to resolve them.
          selectedAgentPubkeys = [];
        }
        if (
          selectedAgentPubkeys.length === 0 ||
          selectedAgentPubkeys.some(
            (pubkey) =>
              !providers.some(
                (provider) => provider.pubkey === normalizePubkey(pubkey),
              ),
          )
        ) {
          setActiveQuery(null);
          setSelectedIndex(0);
          return;
        }
      }

      const signature = `${detected.replaceFromOffset}:${detected.leadingText}:${detected.query}`;
      if (dismissedSignatureRef.current === signature) {
        setActiveQuery(null);
        return;
      }
      dismissedSignatureRef.current = null;
      setActiveQuery({ detected, selectedAgentPubkeys, signature });
      setSelectedIndex(0);
    },
    [providers, mentions.getMentionIdentities, mentions.extractMentionPubkeys],
  );

  const insertCommand = React.useCallback(
    (
      suggestion: SlashCommandSuggestion,
      selectionEnd: number,
    ): AutocompleteEdit | null => {
      if (!activeQuery) return null;
      let agentDisplayName = suggestion.agentDisplayName;
      if (activeQuery.selectedAgentPubkeys === null) {
        const label = mentions.registerMentionPubkey(
          agentDisplayName,
          suggestion.agentPubkey,
          { isAgent: true },
        );
        if (!label) return null;
        agentDisplayName = label;
      }
      const edit = {
        replaceFromOffset: activeQuery.detected.replaceFromOffset,
        replaceToOffset: selectionEnd,
        insertText: buildSlashCommandInsertText(
          { ...suggestion, agentDisplayName },
          activeQuery.selectedAgentPubkeys !== null,
        ),
      };
      setActiveQuery(null);
      setSelectedIndex(0);
      return edit;
    },
    [activeQuery, mentions.registerMentionPubkey],
  );

  const handleKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent,
    ): { handled: boolean; suggestion?: SlashCommandSuggestion } => {
      if (!isOpen || !activeQuery || event.nativeEvent?.isComposing)
        return { handled: false };
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedIndex((current) =>
          current < suggestions.length - 1 ? current + 1 : 0,
        );
        return { handled: true };
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelectedIndex((current) =>
          current > 0 ? current - 1 : suggestions.length - 1,
        );
        return { handled: true };
      }
      if (
        (event.key === "Tab" || event.key === "Enter") &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey &&
        !event.shiftKey
      ) {
        event.preventDefault();
        return { handled: true, suggestion: suggestions[selectedIndex] };
      }
      if (event.key === "Escape") {
        event.preventDefault();
        dismissedSignatureRef.current = activeQuery.signature;
        setActiveQuery(null);
        setSelectedIndex(0);
        return { handled: true };
      }
      return { handled: false };
    },
    [activeQuery, isOpen, selectedIndex, suggestions],
  );

  return {
    groups,
    handleKeyDown,
    insertCommand,
    isOpen,
    selectedIndex,
    suggestions,
    updateQuery,
  };
}
