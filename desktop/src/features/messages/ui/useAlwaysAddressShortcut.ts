import * as React from "react";

import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import type { MentionSuggestion } from "./MentionAutocomplete";

export function useAlwaysAddressShortcut({
  enabled,
  mentions,
  onOpenPicker,
  onToggle,
}: {
  enabled: boolean;
  mentions: UseMentionsResult;
  onOpenPicker: (insertTrigger?: boolean) => void;
  onToggle: (suggestion: MentionSuggestion) => void;
}) {
  const { isMentionOpen, mentionSelectedIndex, suggestions } = mentions;
  return React.useCallback(
    (event: React.KeyboardEvent): boolean => {
      if (
        !enabled ||
        event.key !== "Enter" ||
        !hasPrimaryShortcutModifier(event) ||
        event.altKey ||
        !event.shiftKey
      ) {
        return false;
      }

      event.preventDefault();
      if (event.repeat) return true;
      if (!isMentionOpen) {
        onOpenPicker(false);
        return true;
      }

      const suggestion = suggestions[mentionSelectedIndex];
      if (!suggestion?.isAgent || !suggestion.pubkey) return true;
      onToggle(suggestion);
      return true;
    },
    [
      enabled,
      isMentionOpen,
      mentionSelectedIndex,
      onOpenPicker,
      onToggle,
      suggestions,
    ],
  );
}
