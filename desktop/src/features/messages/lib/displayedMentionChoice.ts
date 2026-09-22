import type { MentionSuggestion } from "../ui/MentionAutocomplete";

type ChoiceKey = {
  key: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
};

/** Enter/Tab own only unmodified keys and may select only an already displayed row. */
export function displayedMentionChoice(
  event: ChoiceKey,
  suggestions: readonly MentionSuggestion[],
  selectedIndex: number,
  pending: boolean,
): { handled: boolean; suggestion?: MentionSuggestion } {
  if (
    (event.key !== "Enter" && event.key !== "Tab") ||
    event.shiftKey ||
    event.ctrlKey ||
    event.metaKey ||
    event.altKey
  )
    return { handled: false };
  return {
    handled: true,
    suggestion: pending ? undefined : suggestions[selectedIndex],
  };
}
