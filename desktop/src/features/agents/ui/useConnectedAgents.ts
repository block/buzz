import * as React from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import {
  disconnectRemoteAgent,
  listConnectedAgents,
} from "@/shared/api/remoteAgentApi";
import type { ConnectedAgent } from "@/shared/api/remoteAgentTypes";
import { addChannelMembers } from "@/shared/api/tauri";
import type { Channel, ChannelRole } from "@/shared/api/types";
import { channelsQueryKey } from "@/features/channels/hooks";
import { relayAgentsQueryKey } from "@/features/agents/hooks";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { connectedAgentMembershipAdded } from "./connectedAgentChannelIntent";

export const connectedAgentsQueryKey = ["connected-agents"] as const;

export type AttachConnectedAgentToChannelInput = {
  agent: ConnectedAgent;
  channelId: string;
  role?: Exclude<ChannelRole, "owner">;
};

export type AttachConnectedAgentToChannelResult = {
  agent: ConnectedAgent;
  membershipAdded: boolean;
};

/**
 * Add a self-hosted agent to a relay channel without crossing the custody
 * boundary. This writes owner-signed membership only: it never starts,
 * deploys, restarts, or otherwise acts on the remote process.
 */
export function useAttachConnectedAgentToChannelMutation() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      agent,
      channelId,
      role = "bot",
    }: AttachConnectedAgentToChannelInput): Promise<AttachConnectedAgentToChannelResult> => {
      const normalizedPubkey = normalizePubkey(agent.pubkey);
      const result = await addChannelMembers({
        channelId,
        pubkeys: [normalizedPubkey],
        role,
      });

      return {
        agent,
        membershipAdded: connectedAgentMembershipAdded(
          normalizedPubkey,
          result,
        ),
      };
    },
    onSettled: async (_data, _error, variables) => {
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: channelsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
        ...(variables
          ? [
              queryClient.invalidateQueries({
                queryKey: ["channels", variables.channelId, "members"],
              }),
            ]
          : []),
      ]);
    },
  });
}

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
  const [agentToAddToChannel, setAgentToAddToChannel] =
    React.useState<ConnectedAgent | null>(null);
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

  const handleAddedToChannel = React.useCallback(
    (channel: Channel, result: AttachConnectedAgentToChannelResult) => {
      setAgentToAddToChannel(null);
      setNoticeMessage(
        result.membershipAdded
          ? `Added ${result.agent.name} to ${channel.name} as a bot. The agent remains self-supervised on ${result.agent.host}.`
          : `${result.agent.name} is already available in ${channel.name}.`,
      );
    },
    [],
  );

  return {
    agents: query.data ?? [],
    agentToAddToChannel,
    error: query.error instanceof Error ? query.error : null,
    isLoading: query.isLoading,
    isPending: disconnectMutation.isPending,
    isDialogOpen,
    noticeMessage,
    openConnectDialog: () => setIsDialogOpen(true),
    setAgentToAddToChannel,
    setIsDialogOpen,
    setNoticeMessage,
    handleAddedToChannel,
    handleConnected,
    handleDisconnect,
  };
}
