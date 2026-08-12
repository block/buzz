import {
  filterAdmittedMentionPubkeys,
  getAgentMentionAdmission,
  getMentionableAgentPubkeys,
  type AgentEligibilityScope,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import * as React from "react";
import type { MentionCandidate } from "./mentionCandidates";

type DirectoryResult<T> = {
  data: T | undefined;
  error: Error | null;
};

export async function revalidateAgentMentionPubkeys({
  pubkeys,
  agentPubkeys,
  ownerPubkeysByAgent,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  ownerOnly,
  ownerPolicyError,
  refetchManagedAgents,
  refetchRelayAgents,
}: {
  pubkeys: readonly string[];
  agentPubkeys: ReadonlySet<string>;
  ownerPubkeysByAgent: ReadonlyMap<string, string | null | undefined>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  ownerOnly: boolean | undefined;
  ownerPolicyError: Error | null;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  refetchRelayAgents: () => Promise<DirectoryResult<RelayAgent[]>>;
}) {
  if (!pubkeys.some((pubkey) => agentPubkeys.has(normalizePubkey(pubkey)))) {
    return [...pubkeys];
  }

  const [managedResult, relayResult] = await Promise.all([
    refetchManagedAgents(),
    refetchRelayAgents(),
  ]);
  if (
    managedResult.error !== null ||
    relayResult.error !== null ||
    managedResult.data === undefined ||
    relayResult.data === undefined ||
    ownerOnly === undefined ||
    ownerPolicyError !== null
  ) {
    return filterAdmittedMentionPubkeys(pubkeys, agentPubkeys, new Set());
  }

  const managedPubkeys = new Set(
    managedResult.data.map((agent) => normalizePubkey(agent.pubkey)),
  );
  const mentionablePubkeys = getMentionableAgentPubkeys({
    currentPubkey,
    eligibilityScope,
    managedAgentPubkeys: managedPubkeys,
    relayAgents: relayResult.data,
    sharedChannelIds,
  });
  const admittedPubkeys = new Set(
    [...agentPubkeys].filter(
      (pubkey) =>
        getAgentMentionAdmission({
          isAgent: true,
          isManagedAgent: managedPubkeys.has(pubkey),
          pubkey,
          ownerPubkey: ownerPubkeysByAgent.get(pubkey),
          currentPubkey,
          mentionableAgentPubkeys: mentionablePubkeys,
          directoryReady: true,
          ownerOnly,
        }) === "allow",
    ),
  );
  return filterAdmittedMentionPubkeys(pubkeys, agentPubkeys, admittedPubkeys);
}

export function useAgentMentionRevalidation({
  agentPubkeys,
  getSelectedAgentPubkeys,
  candidates,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  ownerOnly,
  ownerPolicyError,
  refetchManagedAgents,
  refetchRelayAgents,
}: {
  agentPubkeys: ReadonlySet<string>;
  getSelectedAgentPubkeys: () => ReadonlySet<string>;
  candidates: readonly MentionCandidate[];
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  ownerOnly: boolean | undefined;
  ownerPolicyError: Error | null;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  refetchRelayAgents: () => Promise<DirectoryResult<RelayAgent[]>>;
}) {
  return React.useCallback(
    (pubkeys: readonly string[]) =>
      revalidateAgentMentionPubkeys({
        pubkeys,
        agentPubkeys: new Set([...agentPubkeys, ...getSelectedAgentPubkeys()]),
        ownerPubkeysByAgent: new Map(
          candidates.flatMap((candidate) =>
            candidate.pubkey && candidate.isAgent
              ? [[normalizePubkey(candidate.pubkey), candidate.ownerPubkey]]
              : [],
          ),
        ),
        currentPubkey,
        eligibilityScope,
        sharedChannelIds,
        ownerOnly,
        ownerPolicyError,
        refetchManagedAgents,
        refetchRelayAgents,
      }),
    [
      agentPubkeys,
      candidates,
      currentPubkey,
      eligibilityScope,
      getSelectedAgentPubkeys,
      ownerOnly,
      ownerPolicyError,
      refetchManagedAgents,
      refetchRelayAgents,
      sharedChannelIds,
    ],
  );
}
