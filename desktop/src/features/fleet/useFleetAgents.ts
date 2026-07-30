import * as React from "react";

import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { combineFleetAgents, type FleetAgent } from "./fleetAgents";

/** All of the owner's agents (managed + declared-owned relay agents). */
export function useFleetAgents(): {
  agents: FleetAgent[];
  isLoading: boolean;
  error: Error | null;
} {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;

  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery();
  const relayAgents = relayAgentsQuery.data;

  const relayAgentPubkeys = React.useMemo(
    () => (relayAgents ?? []).map((agent) => agent.pubkey),
    [relayAgents],
  );
  const profilesQuery = useUsersBatchQuery(relayAgentPubkeys, {
    enabled: Boolean(currentPubkey) && relayAgentPubkeys.length > 0,
  });
  const profiles = profilesQuery.data?.profiles;

  const managedAgents = managedAgentsQuery.data;
  const agents = React.useMemo(() => {
    const ownerByPubkey = new Map<string, string>();
    const avatarByPubkey = new Map<string, string | null>();
    for (const [pubkey, summary] of Object.entries(profiles ?? {})) {
      const key = normalizePubkey(pubkey);
      if (summary.ownerPubkey) {
        ownerByPubkey.set(key, normalizePubkey(summary.ownerPubkey));
      }
      avatarByPubkey.set(key, summary.avatarUrl ?? null);
    }
    return combineFleetAgents(
      managedAgents ?? [],
      relayAgents ?? [],
      ownerByPubkey,
      avatarByPubkey,
      currentPubkey,
    );
  }, [currentPubkey, managedAgents, profiles, relayAgents]);

  return {
    agents,
    isLoading: managedAgentsQuery.isLoading || relayAgentsQuery.isLoading,
    error:
      managedAgentsQuery.error instanceof Error
        ? managedAgentsQuery.error
        : null,
  };
}
