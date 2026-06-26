import * as React from "react";

import {
  type ActiveChannelTurnSummary,
  useActiveAgentTurnsBridge,
  useActiveAgentTurnsByChannel,
} from "@/features/agents/activeAgentTurnsStore";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import { useManagedAgentObserverBridge } from "@/features/agents/observerRelayStore";

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
        activeWorkingChannels.map((summary) => [summary.channelId, summary]),
      ),
    [activeWorkingChannels],
  );
}
