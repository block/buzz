import type { ComponentProps } from "react";
import { usePresenceRuns } from "@/features/presence/usePresenceRuns";
import { MentionAutocomplete as MentionAutocompleteView } from "./MentionAutocomplete";

/** Fetch only visible agent placements, once per picker rather than per row. */
export function MentionAutocomplete(
  props: ComponentProps<typeof MentionAutocompleteView>,
) {
  const presence = usePresenceRuns(
    props.suggestions.flatMap((suggestion) =>
      suggestion.isAgent && suggestion.pubkey ? [suggestion.pubkey] : [],
    ),
  );
  return (
    <MentionAutocompleteView
      {...props}
      presenceRuns={presence.data}
      presenceNow={presence.now}
    />
  );
}
