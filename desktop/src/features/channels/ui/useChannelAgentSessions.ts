import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import type {
  Channel,
  ChannelMember,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";
import type { PanelValueSetter } from "./useChannelPanelHistoryState";

export type ChannelAgentSessionAgent = Pick<
  ManagedAgent,
  "pubkey" | "name" | "status"
> & {
  agentSource: "managed" | "member-bot" | "relay";
  canInterruptTurn: boolean;
  channelIds?: string[];
  channels?: string[];
};

type UseChannelAgentSessionsOptions = {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  agentsLoaded: boolean;
  channelMembers?: ChannelMember[];
  handleOpenThread: (message: TimelineMessage) => void;
  managedAgents: ChannelAgentSessionAgent[];
  openAgentSessionPubkey: string | null;
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenAgentSessionPubkey: PanelValueSetter;
  setOpenThreadHeadId: (value: string | null) => void;
  setProfilePanelPubkey: (value: string | null) => void;
  setThreadReplyTargetId: (value: string | null) => void;
  setThreadScrollTargetId: (value: string | null) => void;
};

function relayStatusToManagedStatus(
  status: RelayAgent["status"],
): ManagedAgent["status"] {
  return status === "offline" ? "stopped" : "deployed";
}

export function buildChannelAgentSessionCandidates({
  channelMembers,
  managedAgents,
  relayAgents,
}: {
  channelMembers?: ChannelMember[];
  managedAgents: ManagedAgent[];
  relayAgents: RelayAgent[];
}): ChannelAgentSessionAgent[] {
  const byPubkey = new Map<string, ChannelAgentSessionAgent>();

  for (const agent of relayAgents) {
    byPubkey.set(normalizePubkey(agent.pubkey), {
      pubkey: agent.pubkey,
      name: agent.name,
      status: relayStatusToManagedStatus(agent.status),
      agentSource: "relay",
      canInterruptTurn: false,
      channelIds: agent.channelIds,
      channels: agent.channels,
    });
  }

  for (const agent of managedAgents) {
    const key = normalizePubkey(agent.pubkey);
    const existing = byPubkey.get(key);
    byPubkey.set(key, {
      pubkey: agent.pubkey,
      name: agent.name,
      status: agent.status,
      agentSource: "managed",
      canInterruptTurn: true,
      channelIds: existing?.channelIds,
      channels: existing?.channels,
    });
  }

  for (const member of channelMembers ?? []) {
    const key = normalizePubkey(member.pubkey);
    if (member.role !== "bot" || byPubkey.has(key)) {
      continue;
    }

    byPubkey.set(key, {
      pubkey: member.pubkey,
      name: member.displayName ?? member.pubkey.slice(0, 8),
      status: "deployed",
      agentSource: "member-bot",
      canInterruptTurn: false,
    });
  }

  return [...byPubkey.values()];
}

export function getChannelAgentSessionAgents({
  activeChannel,
  activeChannelId,
  agents,
  channelMembers,
}: {
  activeChannel: Channel | null;
  activeChannelId: string | null;
  agents: ChannelAgentSessionAgent[];
  channelMembers?: ChannelMember[];
}): ChannelAgentSessionAgent[] {
  if (!activeChannelId || !activeChannel) {
    return [];
  }

  const memberPubkeys = channelMembers
    ? new Set(channelMembers.map((member) => normalizePubkey(member.pubkey)))
    : null;
  const botMemberPubkeys = channelMembers
    ? new Set(
        channelMembers
          .filter((member) => member.role === "bot")
          .map((member) => normalizePubkey(member.pubkey)),
      )
    : null;

  return agents.filter((agent) => {
    const normalizedPubkey = normalizePubkey(agent.pubkey);
    const channelIds = agent.channelIds ?? [];
    const channels = agent.channels ?? [];
    const hasDeclaredChannelScope =
      channelIds.length > 0 || channels.length > 0;
    const matchesDeclaredChannel =
      channelIds.includes(activeChannelId) ||
      channels.includes(activeChannel.name);

    if (agent.agentSource === "member-bot") {
      return botMemberPubkeys?.has(normalizedPubkey) ?? matchesDeclaredChannel;
    }

    if (agent.agentSource === "managed") {
      return memberPubkeys?.has(normalizedPubkey) ?? matchesDeclaredChannel;
    }

    if (matchesDeclaredChannel) {
      return true;
    }

    return (
      !hasDeclaredChannelScope && Boolean(memberPubkeys?.has(normalizedPubkey))
    );
  });
}

export function useChannelAgentSessions({
  activeChannel,
  activeChannelId,
  agentsLoaded,
  channelMembers,
  handleOpenThread,
  managedAgents,
  openAgentSessionPubkey,
  setExpandedThreadReplyIds,
  setOpenAgentSessionPubkey,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
}: UseChannelAgentSessionsOptions) {
  const channelAgentSessionAgents = React.useMemo(
    () =>
      getChannelAgentSessionAgents({
        activeChannel,
        activeChannelId,
        agents: managedAgents,
        channelMembers,
      }),
    [activeChannel, activeChannelId, channelMembers, managedAgents],
  );

  const closeAgentSession = React.useCallback(() => {
    setOpenAgentSessionPubkey(null);
  }, [setOpenAgentSessionPubkey]);

  const openAgentSession = React.useCallback(
    (pubkey: string) => {
      setOpenThreadHeadId(null);
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      setProfilePanelPubkey(null);
      setOpenAgentSessionPubkey(pubkey);
    },
    [
      setExpandedThreadReplyIds,
      setOpenAgentSessionPubkey,
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  const selectAgentSession = React.useCallback(
    (pubkey: string) => {
      setOpenAgentSessionPubkey(pubkey);
    },
    [setOpenAgentSessionPubkey],
  );

  const openThreadAndCloseAgentSession = React.useCallback(
    (message: TimelineMessage) => {
      setOpenAgentSessionPubkey(null);
      setProfilePanelPubkey(null);
      handleOpenThread(message);
    },
    [handleOpenThread, setOpenAgentSessionPubkey, setProfilePanelPubkey],
  );

  React.useEffect(() => {
    // An empty agent list can mean the queries behind it are still loading
    // (e.g. a reload restoring the agentSession URL param), so wait until the
    // agent queries have settled. Once loaded, a channel that legitimately has
    // zero agents will still auto-close a stale param.
    if (
      openAgentSessionPubkey &&
      agentsLoaded &&
      !channelAgentSessionAgents.some(
        (agent) =>
          normalizePubkey(agent.pubkey) ===
          normalizePubkey(openAgentSessionPubkey),
      )
    ) {
      setOpenAgentSessionPubkey(null, { replace: true });
    }
  }, [
    agentsLoaded,
    channelAgentSessionAgents,
    openAgentSessionPubkey,
    setOpenAgentSessionPubkey,
  ]);

  return {
    channelAgentSessionAgents,
    closeAgentSession,
    openAgentSession,
    openAgentSessionPubkey,
    openThreadAndCloseAgentSession,
    selectAgentSession,
  };
}
