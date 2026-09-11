/**
 * Picker candidates for BW writer assignment (P4E follow-up: "mention-style
 * picker instead of manual hex"). The BW issue panel has no composer
 * `channelId` of its own, so candidates come from the same directories
 * `useMentions` already draws on for the channel case: the relay agent
 * directory and the repo-bound channel's members. Profiles resolve a
 * display name/avatar for whichever source didn't already carry one — the
 * same resolution `IssueAssigneesRow.tsx` (`profileForPubkey`/
 * `labelForPubkey`) uses to label assignee pubkeys.
 */
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";

export type BwAssignmentCandidateSource = {
  pubkey: string;
  displayName?: string | null;
  avatarUrl?: string | null;
  isAgent?: boolean;
};

export type BwAssignmentCandidate = {
  pubkey: string;
  displayName: string;
  avatarUrl: string | null;
  isAgent: boolean;
};

/** Merge relay agents and channel members into one deduplicated candidate
 * list. A candidate present in both sources keeps its agent flag and takes
 * whichever side resolves a name/avatar first. A candidate with no name
 * anywhere (neither source, nor a profile) falls back to its truncated
 * pubkey — never hidden from the picker. */
export function buildBwAssignmentCandidates({
  relayAgents,
  members,
  profiles,
}: {
  relayAgents: readonly BwAssignmentCandidateSource[];
  members: readonly BwAssignmentCandidateSource[];
  profiles?: UserProfileLookup;
}): BwAssignmentCandidate[] {
  const byPubkey = new Map<string, BwAssignmentCandidate>();

  const add = (source: BwAssignmentCandidateSource) => {
    const pubkey = normalizePubkey(source.pubkey);
    const profile = profiles?.[pubkey];
    const displayName =
      source.displayName?.trim() ||
      profile?.displayName?.trim() ||
      profile?.nip05Handle?.trim() ||
      truncatePubkey(pubkey);
    const avatarUrl = source.avatarUrl ?? profile?.avatarUrl ?? null;
    const isAgent = source.isAgent === true || profile?.isAgent === true;
    const current = byPubkey.get(pubkey);
    byPubkey.set(pubkey, {
      avatarUrl: current?.avatarUrl ?? avatarUrl,
      displayName: current?.displayName ?? displayName,
      isAgent: current?.isAgent === true || isAgent,
      pubkey,
    });
  };

  for (const agent of relayAgents) add({ ...agent, isAgent: true });
  for (const member of members) add(member);

  return [...byPubkey.values()].sort((a, b) =>
    a.displayName.localeCompare(b.displayName),
  );
}

/** Case-insensitive substring filter over the candidate's resolved display
 * name — the same "type to filter" affordance as the composer's `@`-picker. */
export function filterBwAssignmentCandidates(
  candidates: readonly BwAssignmentCandidate[],
  query: string,
): BwAssignmentCandidate[] {
  const trimmed = query.trim().toLowerCase();
  if (!trimmed) return [...candidates];
  return candidates.filter((candidate) =>
    candidate.displayName.toLowerCase().includes(trimmed),
  );
}

/** The exact `delegate` a picker selection stages for `submitBwAssignment` —
 * normalized the same way the raw-hex fallback input already is
 * (`.trim().toLowerCase()`), so both paths reach Core as the identical wire
 * value regardless of which one picked the writer. */
export function resolveBwAssignmentSelection(suggestion: {
  pubkey?: string;
}): string | null {
  return suggestion.pubkey ? normalizePubkey(suggestion.pubkey) : null;
}
