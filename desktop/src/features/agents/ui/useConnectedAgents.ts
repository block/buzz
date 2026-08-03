import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  disconnectRemoteAgent,
  listConnectedAgents,
} from "@/shared/api/remoteAgentApi";
import type { ConnectedAgent } from "@/shared/api/remoteAgentTypes";

export const connectedAgentsQueryKey = ["connected-agents"] as const;

/**
 * Connected self-hosted agents.
 *
 * No `refetchInterval`. The managed-agents query polls because a local process
 * can die with no relay event to signal it; a connected agent's record is a
 * local pointer that changes only when the user connects or disconnects one, so
 * polling it would be pure noise. Liveness of the agent itself comes from relay
 * presence, which the agent publishes and Buzz already subscribes to.
 */
export function useConnectedAgentsQuery() {
  return useQuery({
    queryKey: connectedAgentsQueryKey,
    queryFn: listConnectedAgents,
    staleTime: 30_000,
  });
}

/**
 * State and actions for the Connected-agents section.
 *
 * There is deliberately no start/stop/restart action here to match: the
 * surface offers only what Buzz can actually do to an agent it does not own.
 */
export function useConnectedAgents() {
  const queryClient = useQueryClient();
  const query = useConnectedAgentsQuery();
  const [isDialogOpen, setIsDialogOpen] = React.useState(false);
  const [noticeMessage, setNoticeMessage] = React.useState<string | null>(null);

  const disconnectMutation = useMutation({
    mutationFn: (pubkey: string) => disconnectRemoteAgent(pubkey),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: connectedAgentsQueryKey });
    },
  });

  const handleDisconnect = React.useCallback(
    async (agent: ConnectedAgent) => {
      await disconnectMutation.mutateAsync(agent.pubkey);
      // Say what did NOT happen. A user who just clicked "Disconnect" has good
      // reason to wonder whether they killed their agent; they did not, and
      // silence would leave them guessing.
      setNoticeMessage(
        `${agent.name} is no longer listed here. It is still running on ${agent.host} — Buzz never controlled it.`,
      );
    },
    [disconnectMutation],
  );

  const handleConnected = React.useCallback(
    (agent: ConnectedAgent) => {
      void queryClient.invalidateQueries({ queryKey: connectedAgentsQueryKey });
      setNoticeMessage(`Connected ${agent.name} on ${agent.host}.`);
    },
    [queryClient],
  );

  return {
    agents: query.data ?? [],
    error: query.error instanceof Error ? query.error : null,
    isLoading: query.isLoading,
    isPending: disconnectMutation.isPending,
    isDialogOpen,
    noticeMessage,
    openConnectDialog: () => setIsDialogOpen(true),
    setIsDialogOpen,
    setNoticeMessage,
    handleConnected,
    handleDisconnect,
  };
}
