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

export class AgentMentionRelayError extends Error {
  constructor() {
    super("Couldn't reach the relay to verify this agent. Retry?");
    this.name = "AgentMentionRelayError";
  }
}

export function agentMentionFailureMessage(error: unknown): string | null {
  if (
    error instanceof AgentMentionAuthorizationError ||
    error instanceof AgentMentionRelayError
  ) {
    return error.message;
  }
  return null;
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
  refetchManagedAgents: () => Promise<DirectoryResult<ManagedAgent[]>>;
  fetchRelayAgents: (pubkeys: string[]) => Promise<RelayAgent[]>;
}) {
  const requestedAgentPubkeys = new Set(
    pubkeys.map(normalizePubkey).filter((pubkey) => agentPubkeys.has(pubkey)),
  );
  if (requestedAgentPubkeys.size === 0) {
    return [...pubkeys];
  }

  const managedQuery = refetchManagedAgents().catch(() => null);
  const relayRead = await readRelayAgents(fetchRelayAgents, [
    ...requestedAgentPubkeys,
  ]);
  const managedResult = await managedQuery;
  const relayAgents = relayRead.agents;
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
      const isManagedAgent = managedPubkeys.has(normalizePubkey(pubkey));
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
  const deniedPubkeys = [...requestedAgentPubkeys].filter(
    (pubkey) => !admittedPubkeys.has(pubkey),
  );
  if (deniedPubkeys.length > 0) {
    const deniedBecauseRelayWasUnreachable =
      relayRead.transportFailed &&
      deniedPubkeys.some((pubkey) => !managedPubkeys.has(normalizePubkey(pubkey)));
    if (deniedBecauseRelayWasUnreachable) {
      throw new AgentMentionRelayError();
    }
    throw new AgentMentionAuthorizationError();
  }
  return [...pubkeys];
}

const relayReadAttempts = 3;

async function readRelayAgents(
  fetchRelayAgents: (pubkeys: string[]) => Promise<RelayAgent[]>,
  pubkeys: string[],
): Promise<{ agents: RelayAgent[] | null; transportFailed: boolean }> {
  for (let attempt = 0; attempt < relayReadAttempts; attempt += 1) {
    try {
      return { agents: await fetchRelayAgents(pubkeys), transportFailed: false };
    } catch {
      if (attempt < relayReadAttempts - 1) {
        await new Promise((resolve) => setTimeout(resolve, 25 * (attempt + 1)));
      }
    }
  }
  return { agents: null, transportFailed: true };
}

export function useAgentMentionRevalidation({
  agentPubkeys,
  getSelectedAgentPubkeys,
  currentPubkey,
  eligibilityScope,
  sharedChannelIds,
  refetchManagedAgents,
}: {
  agentPubkeys: ReadonlySet<string>;
  getSelectedAgentPubkeys: () => ReadonlySet<string>;
  currentPubkey: string | null;
  eligibilityScope: AgentEligibilityScope;
  sharedChannelIds: ReadonlySet<string>;
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
      currentPubkey,
      eligibilityScope,
      getSelectedAgentPubkeys,
      refetchManagedAgents,
      sharedChannelIds,
    ],
  );
}
