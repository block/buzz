import * as React from "react";

import {
  type ActiveChannelTurnSummary,
  useActiveAgentTurnsBridge,
  useActiveAgentTurnsByChannel,
} from "@/features/agents/activeAgentTurnsStore";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
import {
  useManagedAgentObserverBridge,
  useObserverCandidatePubkeys,
} from "@/features/agents/observerRelayStore";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { ManagedAgent } from "@/shared/api/types";

type BridgeAgent = Pick<ManagedAgent, "pubkey" | "status">;

export function useActiveWorkingChannelsById(): ReadonlyMap<
  string,
  ActiveChannelTurnSummary
> {
  const identityQuery = useIdentityQuery();
  const currentPubkey = identityQuery.data?.pubkey;
  const managedAgentsQuery = useManagedAgentsQuery();
  const relayAgentsQuery = useRelayAgentsQuery({ enabled: true });
  const managedAgents = React.useMemo(
    () => managedAgentsQuery.data ?? [],
    [managedAgentsQuery.data],
  );
  const relayAgents = React.useMemo(
    () => relayAgentsQuery.data ?? [],
    [relayAgentsQuery.data],
  );
  const observerCandidatePubkeys = useObserverCandidatePubkeys();
  const relayAgentPubkeys = React.useMemo(
    () => relayAgents.map((agent) => agent.pubkey),
    [relayAgents],
  );
  const ownerCandidatePubkeys = React.useMemo(
    () => [...new Set([...relayAgentPubkeys, ...observerCandidatePubkeys])],
    [observerCandidatePubkeys, relayAgentPubkeys],
  );
  const ownerCandidateProfilesQuery = useUsersBatchQuery(
    ownerCandidatePubkeys,
    {
      enabled: Boolean(currentPubkey) && ownerCandidatePubkeys.length > 0,
    },
  );
  const bridgeAgents = React.useMemo<BridgeAgent[]>(() => {
    const agentsByPubkey = new Map<string, BridgeAgent>();
    for (const agent of managedAgents) {
      agentsByPubkey.set(agent.pubkey.toLowerCase(), {
        pubkey: agent.pubkey,
        status: agent.status,
      });
    }

    if (currentPubkey) {
      const currentPubkeyLower = currentPubkey.toLowerCase();
      const profiles = ownerCandidateProfilesQuery.data?.profiles ?? {};
      for (const pubkey of ownerCandidatePubkeys) {
        const key = pubkey.toLowerCase();
        const ownerPubkey = profiles[key]?.ownerPubkey;
        if (ownerPubkey?.toLowerCase() !== currentPubkeyLower) continue;
        if (agentsByPubkey.has(key)) continue;
        agentsByPubkey.set(key, {
          pubkey,
          status: "deployed",
        });
      }
    }

    return [...agentsByPubkey.values()];
  }, [
    currentPubkey,
    managedAgents,
    ownerCandidateProfilesQuery.data,
    ownerCandidatePubkeys,
  ]);

  useManagedAgentObserverBridge(bridgeAgents);
  useActiveAgentTurnsBridge(bridgeAgents);

  const activeWorkingChannels = useActiveAgentTurnsByChannel();
  return React.useMemo(
    () =>
      new Map(
        activeWorkingChannels.map((summary) => [summary.channelId, summary]),
      ),
    [activeWorkingChannels],
  );
}
