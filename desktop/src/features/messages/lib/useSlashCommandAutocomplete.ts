import * as React from "react";

import {
  getAvailableAgentCommands,
  subscribeAgentObserverStore,
} from "@/features/agents/observerRelayStore";
import type { AgentAvailableCommand } from "@/features/agents/lib/agentAvailableCommands";
import type { UseMentionsResult } from "./useMentions";
import type { AutocompleteEdit } from "./useRichTextEditor";

const MAX_SUGGESTIONS = 12;

type MentionRouting = Pick<
  UseMentionsResult,
  "extractMentionPubkeys" | "isAgentPubkey"
>;

export function detectSlashCommandQuery(
  value: string,
  cursorPosition: number,
): { query: string; startIndex: number } | null {
  const beforeCursor = value.slice(0, cursorPosition);
  const match = beforeCursor.match(/(?:^|\s)(\/([^\s/]*))$/);
  if (!match) return null;
  return {
    query: match[2] ?? "",
    startIndex: beforeCursor.length - match[1].length,
  };
}

export function rankSlashCommands(
  commands: readonly AgentAvailableCommand[],
  query: string,
): AgentAvailableCommand[] {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) return commands.slice(0, MAX_SUGGESTIONS);

  return commands
    .map((command, index) => {
      const name = command.name.toLowerCase();
      const description = command.description?.toLowerCase() ?? "";
      const score =
        name === normalizedQuery
          ? 0
          : name.startsWith(normalizedQuery)
            ? 1
            : name.includes(normalizedQuery)
              ? 2
              : description.includes(normalizedQuery)
                ? 3
                : Number.POSITIVE_INFINITY;
      return { command, index, score };
    })
    .filter((entry) => Number.isFinite(entry.score))
    .sort((left, right) => left.score - right.score || left.index - right.index)
    .slice(0, MAX_SUGGESTIONS)
    .map((entry) => entry.command);
}

export function useSlashCommandAutocomplete(mentions: MentionRouting) {
  const [query, setQuery] = React.useState<string | null>(null);
  const [startIndex, setStartIndex] = React.useState(0);
  const [targetAgentPubkey, setTargetAgentPubkey] = React.useState<
    string | null
  >(null);
  const [selectedIndex, setSelectedIndex] = React.useState(0);

  const getSnapshot = React.useCallback(
    () => getAvailableAgentCommands(targetAgentPubkey),
    [targetAgentPubkey],
  );
  const commands = React.useSyncExternalStore(
    subscribeAgentObserverStore,
    getSnapshot,
    getSnapshot,
  );
  const suggestions = React.useMemo(
    () => rankSlashCommands(commands, query ?? ""),
    [commands, query],
  );
  const isOpen = query !== null && suggestions.length > 0;

  React.useEffect(() => {
    setSelectedIndex((current) => (current < suggestions.length ? current : 0));
  }, [suggestions.length]);

  const clear = React.useCallback(() => {
    setQuery(null);
    setTargetAgentPubkey(null);
    setSelectedIndex(0);
  }, []);

  const updateQuery = React.useCallback(
    (value: string, cursorPosition: number) => {
      const detected = detectSlashCommandQuery(value, cursorPosition);
      if (!detected) {
        clear();
        return;
      }

      const beforeSlash = value.slice(0, detected.startIndex);
      const target = mentions
        .extractMentionPubkeys(beforeSlash)
        .find(mentions.isAgentPubkey);
      if (!target) {
        clear();
        return;
      }

      setQuery(detected.query);
      setStartIndex(detected.startIndex);
      setTargetAgentPubkey(target);
      setSelectedIndex(0);
    },
    [clear, mentions.extractMentionPubkeys, mentions.isAgentPubkey],
  );

  const insert = React.useCallback(
    (
      suggestion: AgentAvailableCommand,
      selectionEnd: number,
    ): AutocompleteEdit => {
      clear();
      return {
        replaceFromOffset: startIndex,
        replaceToOffset: selectionEnd,
        insertText: `/${suggestion.name} `,
      };
    },
    [clear, startIndex],
  );

  const handleKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent,
    ): { handled: boolean; suggestion?: AgentAvailableCommand } => {
      if (!isOpen) return { handled: false };

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
        event.key === "Tab" ||
        (event.key === "Enter" &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey &&
          !event.shiftKey)
      ) {
        event.preventDefault();
        return { handled: true, suggestion: suggestions[selectedIndex] };
      }
      if (event.key === "Escape") {
        event.preventDefault();
        clear();
        return { handled: true };
      }
      return { handled: false };
    },
    [clear, isOpen, selectedIndex, suggestions],
  );

  return {
    clear,
    handleKeyDown,
    insert,
    isOpen,
    selectedIndex,
    suggestions,
    updateQuery,
  };
}
