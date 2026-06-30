import * as React from "react";

import {
  type ActiveChannelTurnSummary,
  useActiveAgentTurnsBridge,
  useActiveAgentTurnsByChannel,
} from "@/features/agents/activeAgentTurnsStore";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function resolveActiveWorkingChannelNames(
  summary: ActiveChannelTurnSummary,
  managedAgents: readonly { pubkey: string; name: string }[],
): ActiveChannelTurnSummary {
  const namesByPubkey = new Map(
    managedAgents.map((agent) => [normalizePubkey(agent.pubkey), agent.name]),
  );

  return {
    ...summary,
    agentNames: summary.agentPubkeys.flatMap((pubkey) => {
      const name = namesByPubkey.get(normalizePubkey(pubkey));
      return name ? [name] : [];
    }),
  };
}

export function useActiveWorkingChannelsById(): ReadonlyMap<
  string,
  ActiveChannelTurnSummary
> {
  const managedAgentsQuery = useManagedAgentsQuery();
  const managedAgents = React.useMemo(
    () => managedAgentsQuery.data ?? [],
    [managedAgentsQuery.data],
  );

  useManagedAgentObserverBridge(managedAgents);
  useActiveAgentTurnsBridge(managedAgents);

  const activeWorkingChannels = useActiveAgentTurnsByChannel();
  return React.useMemo(
    () =>
      new Map(
        activeWorkingChannels.map((summary) => {
          const resolvedSummary = resolveActiveWorkingChannelNames(
            summary,
            managedAgents,
          );
          return [resolvedSummary.channelId, resolvedSummary];
        }),
      ),
    [activeWorkingChannels, managedAgents],
  );
}
