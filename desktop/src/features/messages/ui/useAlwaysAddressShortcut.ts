import * as React from "react";

import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import type { MentionSuggestion } from "./MentionAutocomplete";

export function useAlwaysAddressShortcut({
  enabled,
  mentions,
  onSelect,
  onToggle,
}: {
  enabled: boolean;
  mentions: UseMentionsResult;
  onSelect: (suggestion: MentionSuggestion) => void;
  onToggle: (suggestion: MentionSuggestion) => void;
}) {
  const {
    getDefaultAgentSuggestion,
    isMentionOpen,
    mentionSelectedIndex,
    suggestions,
  } = mentions;
  return React.useCallback(
    (event: React.KeyboardEvent): boolean => {
      if (
        !enabled ||
        event.key.toLowerCase() !== "m" ||
        !hasPrimaryShortcutModifier(event) ||
        event.altKey ||
        !event.shiftKey
      ) {
        return false;
      }

      event.preventDefault();
      if (event.repeat) return true;
      const suggestion = isMentionOpen
        ? suggestions[mentionSelectedIndex]
        : getDefaultAgentSuggestion();
      if (!suggestion?.isAgent || !suggestion.pubkey) return true;
      if (isMentionOpen) {
        onSelect(suggestion);
      } else {
        onToggle(suggestion);
      }
      return true;
    },
    [
      enabled,
      getDefaultAgentSuggestion,
      isMentionOpen,
      mentionSelectedIndex,
      onSelect,
      onToggle,
      suggestions,
    ],
  );
}
