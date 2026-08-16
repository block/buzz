import type { RelayAgent, UserProfileSummary } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export type RelayAgentPickerCandidate = {
  agent: RelayAgent;
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

type RelayAgentPickerEligibilityInput = {
  currentPubkey: string | null | undefined;
  managedAgentPubkeys: Iterable<string>;
  memberPubkeys: Iterable<string>;
  profiles: Record<string, UserProfileSummary>;
  relayAgents: readonly RelayAgent[];
};

/**
 * Returns relay-native agents owned by the current identity that can be added
 * directly to the channel without creating a duplicate local managed runtime.
 */
export function getRelayAgentPickerCandidates({
  currentPubkey,
  managedAgentPubkeys,
  memberPubkeys,
  profiles,
  relayAgents,
}: RelayAgentPickerEligibilityInput): RelayAgentPickerCandidate[] {
  const normalizedCurrentPubkey = currentPubkey
    ? normalizePubkey(currentPubkey)
    : null;
  if (!normalizedCurrentPubkey) return [];

  const managed = new Set(
    [...managedAgentPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );
  const members = new Set(
    [...memberPubkeys].map((pubkey) => normalizePubkey(pubkey)),
  );
  const seen = new Set<string>();
  const candidates: RelayAgentPickerCandidate[] = [];

  for (const agent of relayAgents) {
    const pubkey = normalizePubkey(agent.pubkey);
    const profile = profiles[pubkey];
    const ownerPubkey = profile?.ownerPubkey
      ? normalizePubkey(profile.ownerPubkey)
      : null;

    if (
      seen.has(pubkey) ||
      managed.has(pubkey) ||
      members.has(pubkey) ||
      agent.channelAddPolicy?.replaceAll("-", "_").toLowerCase() === "nobody" ||
      ownerPubkey !== normalizedCurrentPubkey
    ) {
      continue;
    }

    seen.add(pubkey);
    candidates.push({
      agent,
      avatarUrl: profile.avatarUrl,
      displayName: profile.displayName?.trim() || agent.name,
      pubkey,
    });
  }

  return candidates.sort((left, right) =>
    left.displayName.localeCompare(right.displayName, undefined, {
      sensitivity: "base",
    }),
  );
}
