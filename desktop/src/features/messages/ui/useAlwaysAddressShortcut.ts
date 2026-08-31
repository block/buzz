import * as React from "react";

import type { UseMentionsResult } from "@/features/messages/lib/useMentions";
import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";
import type { MentionSuggestion } from "./MentionAutocomplete";

export function useAlwaysAddressShortcut({
  enabled,
  lockedAgent,
  mentions,
  onOpenPicker,
  onToggle,
}: {
  enabled: boolean;
  lockedAgent?: Pick<MentionSuggestion, "avatarUrl" | "displayName" | "pubkey">;
  mentions: UseMentionsResult;
  onOpenPicker: (insertTrigger?: boolean) => void;
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
        event.code !== "KeyM" ||
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
        : lockedAgent
          ? { ...lockedAgent, isAgent: true }
          : getDefaultAgentSuggestion();
      if (!suggestion?.isAgent || !suggestion.pubkey) {
        if (!isMentionOpen) onOpenPicker(false);
        return true;
      }
      onToggle(suggestion);
      return true;
    },
    [
      enabled,
      getDefaultAgentSuggestion,
      isMentionOpen,
      lockedAgent,
      mentionSelectedIndex,
      onOpenPicker,
      onToggle,
      suggestions,
    ],
  );
}
