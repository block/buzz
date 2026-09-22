import {
  getAgentMentionAdmission,
  getMentionableAgentPubkeys,
  type AgentEligibilityScope,
} from "@/features/agents/lib/agentAutocompleteEligibility";
import { revalidateRelayAgents } from "@/shared/api/tauriRelayAgents";
import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import * as React from "react";

export type MentionRevalidationOptions = {
  phase?: "prepare" | "publish";
  intendedAgentPubkeys?: readonly string[];
};

export class AgentMentionAuthorizationError extends Error {
  constructor() {
    super(
      "Could not authorize a mentioned agent. Check its access and channel membership, then retry or remove the mention.",
    );
    this.name = "AgentMentionAuthorizationError";
  }
}

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
  channelMemberPubkeys,
  refetchManagedAgents,
  fetchRelayAgents,
  phase = "publish",
}: {
  phase?: "prepare" | "publish";
  pubkeys: readonly string[];
  agentPubkeys: ReadonlySet<string>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  /** Destination-channel members. Membership alone authorizes @mention notify. */
  channelMemberPubkeys?: ReadonlySet<string>;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  fetchRelayAgents: (pubkeys: string[]) => Promise<RelayAgent[]>;
}) {
  const requestedAgentPubkeys = new Set(
    pubkeys.map(normalizePubkey).filter((pubkey) => agentPubkeys.has(pubkey)),
  );
  if (requestedAgentPubkeys.size === 0) {
    return [...pubkeys];
  }

  const memberPubkeys = new Set(
    [...(channelMemberPubkeys ?? [])].map((pubkey) => normalizePubkey(pubkey)),
  );
  const destinationChannelId =
    eligibilityScope.type === "channel" || eligibilityScope.type === "owned"
      ? eligibilityScope.channelId
      : null;
  const membershipAuthorizesMentions = destinationChannelId !== null;

  const [managedResult, relayAgents] = await Promise.all([
    refetchManagedAgents().catch(() => null),
    fetchRelayAgents([...requestedAgentPubkeys]).catch(() => null),
  ]);
  const relayDirectoryReady = relayAgents !== null;
  // Each directory proves only its own identities. A failed local runtime
  // query must neither veto fresh relay evidence nor admit stale local data.
  const managedPubkeys = new Set(
    (managedResult?.error === null ? (managedResult.data ?? []) : []).map(
      (agent) => normalizePubkey(agent.pubkey),
    ),
  );
  const mentionablePubkeys = getMentionableAgentPubkeys({
    currentPubkey,
    eligibilityScope,
    phase,
    managedAgentPubkeys: managedPubkeys,
    relayAgents: relayDirectoryReady ? relayAgents : [],
    sharedChannelIds,
  });
  const admittedPubkeys = new Set(
    [...agentPubkeys].filter((pubkey) => {
      const normalized = normalizePubkey(pubkey);
      // Channel membership is sufficient to @mention (matches CLI / notify).
      // Community bots often lack kind:10100 + NIP-OA directory rows; without
      // this, Desktop rejects every in-channel bot mention while CLI succeeds.
      if (membershipAuthorizesMentions && memberPubkeys.has(normalized)) {
        return true;
      }
      const isManagedAgent = managedPubkeys.has(normalized);
      const directoryReady = isManagedAgent || relayDirectoryReady;
      return (
        getAgentMentionAdmission({
          isAgent: true,
          pubkey,
          mentionableAgentPubkeys: mentionablePubkeys,
          directoryReady,
        }) === "allow"
      );
    }),
  );
  if (
    [...requestedAgentPubkeys].some((pubkey) => !admittedPubkeys.has(pubkey))
  ) {
    throw new AgentMentionAuthorizationError();
  }
  return [...pubkeys];
}

export function useAgentMentionRevalidation({
  agentPubkeys,
  getSelectedAgentPubkeys,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  channelMemberPubkeys,
  refetchManagedAgents,
}: {
  agentPubkeys: ReadonlySet<string>;
  getSelectedAgentPubkeys: () => ReadonlySet<string>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
  channelMemberPubkeys?: ReadonlySet<string>;
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
}) {
  return React.useCallback(
    (
      pubkeys: readonly string[],
      destinationChannelId?: string | null,
      options: MentionRevalidationOptions = {},
    ) => {
      // A new DM can acquire its channel during preparation. Validate the
      // actual destination at publication, not the composer's original null id.
      const scope: AgentEligibilityScope = destinationChannelId
        ? {
            type: eligibilityScope.type === "owned" ? "owned" : "channel",
            channelId: destinationChannelId,
          }
        : eligibilityScope;
      return revalidateAgentMentionPubkeys({
        pubkeys,
        agentPubkeys: new Set([
          ...agentPubkeys,
          ...getSelectedAgentPubkeys(),
          ...(options.intendedAgentPubkeys ?? []).map(normalizePubkey),
        ]),
        phase: options.phase,
        currentPubkey,
        eligibilityScope: scope,
        sharedChannelIds,
        channelMemberPubkeys,
        refetchManagedAgents,
        fetchRelayAgents: (requestedPubkeys) =>
          revalidateRelayAgents(
            requestedPubkeys,
            "channelId" in scope ? (scope.channelId ?? undefined) : undefined,
          ),
      });
    },
    [
      agentPubkeys,
      channelMemberPubkeys,
      currentPubkey,
      eligibilityScope,
      getSelectedAgentPubkeys,
      refetchManagedAgents,
      sharedChannelIds,
    ],
  );
}
