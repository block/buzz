import * as React from "react";
import type { MentionSuggestion } from "../ui/MentionAutocomplete";
import type { MentionRequest } from "./useMentionQuery";

export type MentionPickerMode = "first-agent" | null;

/** Install once per request; indexes refer to these displayed rows, never live ranking. */
export function useMentionSelection(
  request: MentionRequest | null,
  candidates: MentionSuggestion[],
  ready: boolean,
) {
  const [snapshot, setSnapshot] = React.useState<{
    request: MentionRequest;
    rows: MentionSuggestion[];
    index: number;
  } | null>(null);
  React.useEffect(() => {
    if (!request) {
      setSnapshot(null);
      return;
    }
    if (!ready) return;
    setSnapshot((old) =>
      old?.request === request
        ? old
        : {
            request,
            rows: candidates.map((row) => ({ ...row })),
            index: request.firstAgent
              ? Math.max(
                  0,
                  candidates.findIndex((s) => s.isAgent && s.pubkey),
                )
              : 0,
          },
    );
  }, [request, candidates, ready]);
  const installed = snapshot?.request === request ? snapshot : null;
  return {
    suggestions: installed?.rows ?? [],
    mentionSelectedIndex: installed?.index ?? 0,
    isLoading: !!request && !installed,
    move: (direction: number) =>
      setSnapshot((old) =>
        !old || old.request !== request || !old.rows.length
          ? old
          : {
              ...old,
              index:
                (old.index + direction + old.rows.length) % old.rows.length,
            },
      ),
  };
}
