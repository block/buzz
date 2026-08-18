import {
  filterAdmittedMentionPubkeys,
  getAgentMentionAdmission,
  getMentionableAgentPubkeys,
  type AgentEligibilityScope,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import type {
  ManagedAgent,
  RelayAgent,
  UsersBatchResponse,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

type DirectoryResult<T> = {
  data: T | undefined;
  error: Error | null;
};

export async function revalidateAgentMentionPubkeys({
  pubkeys,
  agentPubkeys,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  ownerOnly,
  ownerPolicyError,
  refetchManagedAgents,
  refetchRelayAgents,
  refetchOwnerProfiles,
}: {
  pubkeys: readonly string[];
  agentPubkeys: ReadonlySet<string>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  ownerOnly: boolean | undefined;
  ownerPolicyError: Error | null;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  refetchRelayAgents: () => Promise<DirectoryResult<RelayAgent[]>>;
  refetchOwnerProfiles: (pubkeys: string[]) => Promise<UsersBatchResponse>;
}) {
  const requestedAgentPubkeys = new Set(
    pubkeys.map(normalizePubkey).filter((pubkey) => agentPubkeys.has(pubkey)),
  );
  if (requestedAgentPubkeys.size === 0) {
    return [...pubkeys];
  }

  const [managedResult, relayResult, ownerProfiles] = await Promise.all([
    refetchManagedAgents(),
    refetchRelayAgents(),
    ownerOnly
      ? refetchOwnerProfiles([...requestedAgentPubkeys]).catch(() => null)
      : Promise.resolve(null),
  ]);
  const relayDirectoryReady =
    relayResult.error === null && relayResult.data !== undefined;
  if (
    ownerOnly === undefined ||
    ownerPolicyError !== null ||
    managedResult.error !== null ||
    managedResult.data === undefined
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
    relayAgents: relayDirectoryReady ? relayResult.data : [],
    sharedChannelIds,
  });
  const admittedPubkeys = new Set(
    [...agentPubkeys].filter((pubkey) => {
      const isManagedAgent = managedPubkeys.has(normalizePubkey(pubkey));
      const directoryReady =
        isManagedAgent ||
        (relayDirectoryReady && (!ownerOnly || ownerProfiles !== null));
      return (
        getAgentMentionAdmission({
          isAgent: true,
          isManagedAgent,
          pubkey,
          ownerPubkey: ownerProfiles?.profiles[pubkey]?.ownerPubkey,
          currentPubkey,
          mentionableAgentPubkeys: mentionablePubkeys,
          directoryReady,
          ownerOnly,
        }) === "allow"
      );
    }),
  );
  return filterAdmittedMentionPubkeys(pubkeys, agentPubkeys, admittedPubkeys);
}

export function useAgentMentionRevalidation({
  agentPubkeys,
  getSelectedAgentPubkeys,
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
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  ownerOnly: boolean | undefined;
  ownerPolicyError: Error | null;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  refetchRelayAgents: () => Promise<DirectoryResult<RelayAgent[]>>;
}) {
  const queryClient = useQueryClient();
  const refetchOwnerProfiles = React.useCallback(
    async (pubkeys: string[]) => {
      evictUsersBatchEntries(queryClient, pubkeys);
      return getUsersBatch(pubkeys);
    },
    [queryClient],
  );
  return React.useCallback(
    (pubkeys: readonly string[]) =>
      revalidateAgentMentionPubkeys({
        pubkeys,
        agentPubkeys: new Set([...agentPubkeys, ...getSelectedAgentPubkeys()]),
        currentPubkey,
        eligibilityScope,
        sharedChannelIds,
        ownerOnly,
        ownerPolicyError,
        refetchManagedAgents,
        refetchRelayAgents,
        refetchOwnerProfiles,
      }),
    [
      agentPubkeys,
      currentPubkey,
      eligibilityScope,
      getSelectedAgentPubkeys,
      ownerOnly,
      ownerPolicyError,
      refetchManagedAgents,
      refetchOwnerProfiles,
      refetchRelayAgents,
      sharedChannelIds,
    ],
  );
}
