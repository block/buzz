/**
 * Candidates for the BW writer dropdown. The BW issue panel has no composer
 * `channelId` of its own, so candidates come from the same directories used
 * for channel mentions: the relay agent directory and the repo-bound
 * channel's members. Profiles resolve a display name/avatar for whichever
 * source did not already carry one.
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
 * whichever side resolves a name/avatar first. The writer selector is a
 * people-facing control, so identities with no resolved name are omitted
 * instead of exposing a partial public key as presentation text. */
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
      profile?.nip05Handle?.trim();
    if (
      !displayName ||
      displayName.toLowerCase() === pubkey ||
      displayName.toLowerCase() === truncatePubkey(pubkey) ||
      /^(?:nostr:)?npub1/i.test(displayName)
    ) {
      return;
    }
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
