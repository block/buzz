import * as React from "react";

import type { AutocompleteEdit } from "@/features/messages/lib/useRichTextEditor";
import { detectSlashQuery, type SlashQuery } from "./lib/detectSlashQuery";
import { filterSlashCommands } from "./lib/filterSlashCommands";
import type { SlashCommandDefinition } from "./lib/types";

const SLASH_QUERY_DEBOUNCE_MS = 120;

export function useSlashCommandAutocomplete(
  commands: readonly SlashCommandDefinition[],
  options: { isDm: boolean },
) {
  const [slashQuery, setSlashQuery] = React.useState<SlashQuery | null>(null);
  const [selectedIndex, setSelectedIndex] = React.useState(0);

  const suggestions = slashQuery
    ? filterSlashCommands(commands, slashQuery.query, {
        isDm: options.isDm,
      })
    : [];
  const isOpen = suggestions.length > 0;
  const activeIndex = Math.min(selectedIndex, suggestions.length - 1);

  const debounceTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const cancelPendingUpdate = React.useCallback(() => {
    if (debounceTimerRef.current !== null) {
      clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
  }, []);

  React.useEffect(() => cancelPendingUpdate, [cancelPendingUpdate]);

  // Enter reaches ProseMirror first and splits the block, which updates the
  // editor before this hook sees the key. Detecting on a short delay keeps the
  // list open until the key handler has picked the suggestion.
  const updateQuery = React.useCallback(
    (text: string, cursor: number) => {
      cancelPendingUpdate();
      if (text.length === 0) {
        setSlashQuery(null);
        return;
      }
      debounceTimerRef.current = setTimeout(() => {
        debounceTimerRef.current = null;
        const next = detectSlashQuery(text, cursor);
        setSlashQuery((current) =>
          current?.query === next?.query &&
          current?.startIndex === next?.startIndex &&
          current?.endIndex === next?.endIndex
            ? current
            : next,
        );
        setSelectedIndex(0);
      }, SLASH_QUERY_DEBOUNCE_MS);
    },
    [cancelPendingUpdate],
  );

  const clear = React.useCallback(() => {
    cancelPendingUpdate();
    setSlashQuery(null);
    setSelectedIndex(0);
  }, [cancelPendingUpdate]);

  const insertCommand = React.useCallback(
    (
      command: SlashCommandDefinition,
      selectionEnd: number,
    ): AutocompleteEdit => {
      const range = slashQuery ?? { startIndex: 0, endIndex: 0 };
      clear();
      return {
        replaceFromOffset: range.startIndex,
        replaceToOffset: Math.max(range.endIndex, selectionEnd),
        insertText: `/${command.name} `,
      };
    },
    [clear, slashQuery],
  );

  const handleKeyDown = React.useCallback(
    (
      event: React.KeyboardEvent,
    ): { handled: boolean; suggestion?: SlashCommandDefinition } => {
      if (!isOpen) return { handled: false };

      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedIndex(
          activeIndex < suggestions.length - 1 ? activeIndex + 1 : 0,
        );
        return { handled: true };
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSelectedIndex(
          activeIndex > 0 ? activeIndex - 1 : suggestions.length - 1,
        );
        return { handled: true };
      }
      // Shift+Tab stays the browser's so the overlay never traps focus.
      if (
        (event.key === "Tab" && !event.shiftKey) ||
        (event.key === "Enter" &&
          !event.ctrlKey &&
          !event.metaKey &&
          !event.altKey &&
          !event.shiftKey)
      ) {
        event.preventDefault();
        return { handled: true, suggestion: suggestions[activeIndex] };
      }
      if (event.key === "Escape") {
        event.preventDefault();
        clear();
        return { handled: true };
      }
      return { handled: false };
    },
    [activeIndex, clear, isOpen, suggestions],
  );

  return {
    clear,
    handleKeyDown,
    insertCommand,
    isOpen,
    selectedIndex: activeIndex,
    suggestions,
    updateQuery,
  };
}

export type UseSlashCommandAutocompleteResult = ReturnType<
  typeof useSlashCommandAutocomplete
>;
