import * as React from "react";

import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { useWorkingChannels } from "@/features/agents/agentWorkingSignal";
import {
  useManagedAgentsQuery,
  useRelayAgentsQuery,
} from "@/features/agents/hooks";
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
  const relayAgentsQuery = useRelayAgentsQuery();
  const managedAgents = React.useMemo(
    () => managedAgentsQuery.data ?? [],
    [managedAgentsQuery.data],
  );
  const namedAgents = React.useMemo(
    () => [...(relayAgentsQuery.data ?? []), ...managedAgents],
    [managedAgents, relayAgentsQuery.data],
  );

  // Unified working signal: observer-derived turns primary, bot typing as
  // fallback — so the sidebar badge appears even for agents whose observer
  // stream is absent for this build/scope.
  const activeWorkingChannels = useWorkingChannels();
  return React.useMemo(
    () =>
      new Map(
        activeWorkingChannels.map((summary) => {
          const resolvedSummary = resolveActiveWorkingChannelNames(
            summary,
            namedAgents,
          );
          return [resolvedSummary.channelId, resolvedSummary];
        }),
      ),
    [activeWorkingChannels, namedAgents],
  );
}
