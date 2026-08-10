import {
  collapseSeparators,
  WORD_SEPARATORS,
} from "@/shared/lib/identifierMatch";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

export type MentionCandidateForRanking = {
  displayName: string | null;
  isAgent: boolean;
  isMember: boolean;
  kind: "identity" | "persona" | "team";
  personaId?: string | null;
  personaName?: string | null;
  pubkey?: string;
  secondaryLabel?: string | null;
};

export type RankedMentionCandidate<T extends MentionCandidateForRanking> = {
  candidate: T;
  groupRank: number;
  label: string;
  order: number;
  score: number;
};

function getMentionCandidateGroupRank(
  candidate: MentionCandidateForRanking,
  activePersonaIds: ReadonlySet<string>,
) {
  if (candidate.isMember) return 0;

  const isRunnablePersona =
    candidate.kind === "team" ||
    candidate.kind === "persona" ||
    (candidate.personaId ? activePersonaIds.has(candidate.personaId) : false);
  if (isRunnablePersona) return 1;

  if (!candidate.isAgent) return 2;

  return 3;
}

function scoreMentionCandidateLabel(
  label: string,
  lowerQuery: string,
): number | null {
  const lower = label.toLowerCase();
  if (lower === lowerQuery) return 0;
  if (lower.startsWith(lowerQuery)) return 1;

  const words = lower.split(WORD_SEPARATORS).filter(Boolean);
  if (words.some((word) => word === lowerQuery)) return 2;
  if (words.some((word) => word.startsWith(lowerQuery))) return 3;

  // Labels are identifiers: `janedoe` should find `jane-doe` / `Jane Doe`.
  // Ranked below all literal matches so existing ordering is unchanged.
  const collapsedQuery = collapseSeparators(lowerQuery);
  if (
    collapsedQuery.length > 0 &&
    collapseSeparators(lower).startsWith(collapsedQuery)
  ) {
    return 4;
  }

  return null;
}

export function rankMentionCandidates<T extends MentionCandidateForRanking>(
  candidates: readonly T[],
  query: string,
  activePersonaIds: ReadonlySet<string> = new Set(),
): RankedMentionCandidate<T>[] {
  const lowerQuery = query.toLowerCase();

  return candidates
    .map((candidate, order) => {
      const pubkeyLower = candidate.pubkey
        ? normalizePubkey(candidate.pubkey)
        : "";
      const label =
        candidate.displayName ??
        (candidate.pubkey ? truncatePubkey(candidate.pubkey) : "agent");
      const groupRank = getMentionCandidateGroupRank(
        candidate,
        activePersonaIds,
      );

      const labelScores = [
        candidate.displayName,
        candidate.personaName,
        candidate.secondaryLabel,
      ]
        .map((value) =>
          value ? scoreMentionCandidateLabel(value, lowerQuery) : null,
        )
        .filter((score): score is number => score !== null);
      const labelScore =
        labelScores.length > 0 ? Math.min(...labelScores) : null;

      const pubkeyScore = candidate.pubkey
        ? pubkeyLower.startsWith(lowerQuery)
          ? 5
          : pubkeyLower.includes(lowerQuery)
            ? 6
            : null
        : null;
      const score = labelScore !== null ? labelScore : pubkeyScore;

      return { candidate, groupRank, label, order, score };
    })
    .filter((item): item is RankedMentionCandidate<T> => item.score !== null)
    .sort(
      (a, b) =>
        a.groupRank - b.groupRank || a.score - b.score || a.order - b.order,
    );
}
