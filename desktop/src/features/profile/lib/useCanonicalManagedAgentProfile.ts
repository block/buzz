import * as React from "react";

import {
  pickDirectProfileAgent,
  pickProfileAgent,
} from "@/features/agents/lib/pickProfileAgent";
import { useUserProfileQuery } from "@/features/profile/hooks";
import { ownsAuthorAgent } from "@/features/profile/lib/identity";
import { useOwnedManagedAgentPersonaId } from "@/features/profile/lib/useOwnedManagedAgentPersonaId";
import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function useCanonicalManagedAgentProfile(input: {
  currentPubkey: string | undefined;
  managedAgents: readonly ManagedAgent[] | undefined;
  personaId: string | undefined;
  preferDirectManagedAgent?: boolean;
  preserveRequestedInstance?: boolean;
  pubkey: string | undefined;
}) {
  const {
    currentPubkey,
    managedAgents,
    personaId,
    preferDirectManagedAgent = false,
    preserveRequestedInstance = false,
    pubkey,
  } = input;
  const directManagedAgent = React.useMemo(() => {
    if (!pubkey) return undefined;
    const target = normalizePubkey(pubkey);
    return managedAgents?.find(
      (agent) => normalizePubkey(agent.pubkey) === target,
    );
  }, [managedAgents, pubkey]);
  const requestedProfileQuery = useUserProfileQuery(pubkey);
  const historicalPersonaId = useOwnedManagedAgentPersonaId({
    agentPubkey: pubkey,
    enabled: Boolean(
      pubkey &&
        !directManagedAgent &&
        ownsAuthorAgent(requestedProfileQuery.data, currentPubkey),
    ),
    ownerPubkey: currentPubkey,
  });
  const linkedPersonaId =
    personaId ?? directManagedAgent?.personaId ?? historicalPersonaId;
  const personaInstances = React.useMemo(() => {
    if (!linkedPersonaId) {
      return directManagedAgent ? [directManagedAgent] : [];
    }
    return (managedAgents ?? []).filter(
      (agent) => agent.personaId === linkedPersonaId,
    );
  }, [directManagedAgent, linkedPersonaId, managedAgents]);
  const managedAgent = React.useMemo(() => {
    if (directManagedAgent) {
      if (preserveRequestedInstance) return directManagedAgent;
      if (preferDirectManagedAgent) {
        return pickDirectProfileAgent(directManagedAgent, personaInstances);
      }
    }
    return pickProfileAgent(personaInstances) ?? directManagedAgent;
  }, [
    directManagedAgent,
    personaInstances,
    preferDirectManagedAgent,
    preserveRequestedInstance,
  ]);

  return { linkedPersonaId, managedAgent, personaInstances };
}
