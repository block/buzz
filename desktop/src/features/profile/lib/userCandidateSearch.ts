import type { UserSearchResult } from "@/shared/api/types";
import {
  collapseSeparators,
  WORD_SEPARATORS,
} from "@/shared/lib/identifierMatch";
import { normalizePubkey } from "@/shared/lib/pubkey";

type ScoreUserCandidateInput = {
  allowEmptyQuery?: boolean;
  label: string;
  query: string;
  user: UserSearchResult;
};

type RankUserCandidatesInput = {
  allowEmptyQuery?: boolean;
  candidates: UserSearchResult[];
  getLabel: (user: UserSearchResult) => string;
  limit: number;
  query: string;
};

type KeyboardSearchSelectionInput = {
  currentQuery: string;
  rankedQuery: string;
  results: UserSearchResult[];
};

export function scoreUserCandidate({
  allowEmptyQuery = false,
  label,
  query,
  user,
}: ScoreUserCandidateInput) {
  const normalizedQuery = query.trim().toLowerCase();

  if (normalizedQuery.length === 0) {
    return allowEmptyQuery ? 0 : null;
  }

  const labels = [
    label,
    user.nip05Handle?.trim() ?? "",
    user.isAgent ? "agent" : "",
  ];

  for (const candidateLabel of labels) {
    const lower = candidateLabel.toLowerCase();
    if (lower.startsWith(normalizedQuery)) return 0;
    if (
      lower
        .split(WORD_SEPARATORS)
        .some((word) => word.startsWith(normalizedQuery))
    ) {
      return 1;
    }
    if (lower.includes(normalizedQuery)) return 2;
  }

  // Labels are identifiers: `janedoe` should find `jane-doe` / `Jane Doe`.
  // Ranked below all literal label matches so existing ordering is unchanged.
  const collapsedQuery = collapseSeparators(normalizedQuery);
  if (collapsedQuery.length > 0) {
    for (const candidateLabel of labels) {
      if (
        collapseSeparators(candidateLabel.toLowerCase()).includes(
          collapsedQuery,
        )
      ) {
        return 3;
      }
    }
  }

  const pubkey = normalizePubkey(user.pubkey);
  if (pubkey.startsWith(normalizedQuery)) return 4;
  if (pubkey.includes(normalizedQuery)) return 5;

  return null;
}

export function rankUserCandidatesBySearch({
  allowEmptyQuery = false,
  candidates,
  getLabel,
  limit,
  query,
}: RankUserCandidatesInput) {
  return candidates
    .map((candidate, order) => {
      const label = getLabel(candidate);

      return {
        candidate,
        label,
        order,
        score: scoreUserCandidate({
          allowEmptyQuery,
          label,
          query,
          user: candidate,
        }),
      };
    })
    .filter(
      (item): item is typeof item & { score: number } => item.score !== null,
    )
    .sort(
      (left, right) =>
        left.score - right.score ||
        left.label.localeCompare(right.label) ||
        left.order - right.order,
    )
    .slice(0, limit)
    .map(({ candidate }) => candidate);
}

export function getKeyboardSearchSelection({
  currentQuery,
  rankedQuery,
  results,
}: KeyboardSearchSelectionInput) {
  const trimmedCurrentQuery = currentQuery.trim();
  if (trimmedCurrentQuery.length === 0) {
    return null;
  }

  if (rankedQuery.trim() !== trimmedCurrentQuery) {
    return null;
  }

  return results[0] ?? null;
}
