import * as React from "react";

import type { MentionSuggestion } from "@/features/messages/ui/MentionAutocomplete";

export type MentionPickerMode = "first-agent" | "preserve" | null;

export function useMentionSelection(suggestions: MentionSuggestion[]) {
  const [mentionSelectedIndex, setMentionSelectedIndex] = React.useState(0);
  const preferAgentSelectionRef = React.useRef(false);

  React.useEffect(() => {
    setMentionSelectedIndex((current) => {
      if (suggestions.length === 0) return 0;
      if (preferAgentSelectionRef.current) {
        preferAgentSelectionRef.current = false;
        const firstAgentIndex = suggestions.findIndex(
          (suggestion) => suggestion.isAgent && suggestion.pubkey,
        );
        if (firstAgentIndex >= 0) return firstAgentIndex;
      }
      return Math.min(current, suggestions.length - 1);
    });
  }, [suggestions]);

  const clearAgentSelectionPreference = React.useCallback(() => {
    preferAgentSelectionRef.current = false;
  }, []);
  const prepareSelectionPreference = React.useCallback(
    (preference: MentionPickerMode) => {
      preferAgentSelectionRef.current = preference === "first-agent";
    },
    [],
  );
  return {
    clearAgentSelectionPreference,
    mentionSelectedIndex,
    prepareSelectionPreference,
    setMentionSelectedIndex,
  };
}
