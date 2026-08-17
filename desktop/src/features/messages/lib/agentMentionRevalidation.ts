import {
  filterAdmittedMentionPubkeys,
  getAgentMentionAdmission,
  getMentionableAgentPubkeys,
  type AgentEligibilityScope,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { evictUsersBatchEntries } from "@/features/profile/hooks";
import { getUsersBatch } from "@/shared/api/tauriProfiles";
import type {
  ChannelMember,
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

type AgentMentionRevalidationOptions = {
  requireChannelMembership?: boolean;
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
  refetchChannelMembers,
  refetchOwnerProfiles,
  requireChannelMembership = true,
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
  refetchChannelMembers?: () => Promise<DirectoryResult<ChannelMember[]>>;
  refetchOwnerProfiles: (pubkeys: string[]) => Promise<UsersBatchResponse>;
  requireChannelMembership?: boolean;
}) {
  const requestedAgentPubkeys = new Set(
    pubkeys.map(normalizePubkey).filter((pubkey) => agentPubkeys.has(pubkey)),
  );
  if (requestedAgentPubkeys.size === 0) {
    return [...pubkeys];
  }

  const [managedResult, relayResult, ownerProfiles, channelMembersResult] =
    await Promise.all([
      refetchManagedAgents(),
      refetchRelayAgents(),
      ownerOnly
        ? refetchOwnerProfiles([...requestedAgentPubkeys]).catch(() => null)
        : Promise.resolve(null),
      eligibilityScope.type === "channel" &&
      requireChannelMembership &&
      refetchChannelMembers
        ? refetchChannelMembers()
        : Promise.resolve(null),
    ]);
  if (
    managedResult.error !== null ||
    relayResult.error !== null ||
    managedResult.data === undefined ||
    relayResult.data === undefined ||
    ownerOnly === undefined ||
    ownerPolicyError !== null ||
    (ownerOnly && ownerProfiles === null) ||
    (eligibilityScope.type === "channel" &&
      requireChannelMembership &&
      (channelMembersResult === null ||
        channelMembersResult.error !== null ||
        channelMembersResult.data === undefined))
  ) {
    return filterAdmittedMentionPubkeys(pubkeys, agentPubkeys, new Set());
  }

  const managedPubkeys = new Set(
    managedResult.data.map((agent) => normalizePubkey(agent.pubkey)),
  );
  const freshMemberPubkeys = new Set(
    (channelMembersResult?.data ?? []).map((member) =>
      normalizePubkey(member.pubkey),
    ),
  );
  const scopedManagedPubkeys =
    eligibilityScope.type === "channel" && requireChannelMembership
      ? new Set(
          [...managedPubkeys].filter((pubkey) =>
            freshMemberPubkeys.has(pubkey),
          ),
        )
      : managedPubkeys;
  const scopedRelayAgents =
    eligibilityScope.type === "channel" && requireChannelMembership
      ? relayResult.data.filter((agent) =>
          freshMemberPubkeys.has(normalizePubkey(agent.pubkey)),
        )
      : relayResult.data;
  const freshEligibilityScope =
    eligibilityScope.type === "channel" && requireChannelMembership
      ? { ...eligibilityScope, memberPubkeys: freshMemberPubkeys }
      : eligibilityScope;
  const mentionablePubkeys = getMentionableAgentPubkeys({
    currentPubkey,
    eligibilityScope: freshEligibilityScope,
    managedAgentPubkeys: scopedManagedPubkeys,
    relayAgents: scopedRelayAgents,
    sharedChannelIds,
  });
  const admittedPubkeys = new Set(
    [...agentPubkeys].filter(
      (pubkey) =>
        getAgentMentionAdmission({
          isAgent: true,
          isManagedAgent: scopedManagedPubkeys.has(pubkey),
          pubkey,
          ownerPubkey: ownerProfiles?.profiles[pubkey]?.ownerPubkey,
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
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  ownerOnly,
  ownerPolicyError,
  refetchManagedAgents,
  refetchRelayAgents,
  refetchChannelMembers,
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
  refetchChannelMembers?: () => Promise<DirectoryResult<ChannelMember[]>>;
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
    (
      pubkeys: readonly string[],
      options: AgentMentionRevalidationOptions = {},
    ) =>
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
        refetchChannelMembers,
        refetchOwnerProfiles,
        requireChannelMembership: options.requireChannelMembership,
      }),
    [
      agentPubkeys,
      currentPubkey,
      eligibilityScope,
      getSelectedAgentPubkeys,
      ownerOnly,
      ownerPolicyError,
      refetchManagedAgents,
      refetchChannelMembers,
      refetchOwnerProfiles,
      refetchRelayAgents,
      sharedChannelIds,
    ],
  );
}
