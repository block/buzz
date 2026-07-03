import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import type {
  Channel,
  ChannelMember,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
import { usePanelReturnTarget } from "@/shared/hooks/usePanelReturnTarget";
import { normalizePubkey } from "@/shared/lib/pubkey";
import {
  type AgentSessionReturnTarget,
  resolveAgentSessionReturnTarget,
} from "./agentSessionSelection";
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
  openThreadHeadId: string | null;
  profilePanelPubkey?: string | null;
  setChannelManagementOpen: (open: boolean) => void;
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenAgentSessionChannelId: PanelValueSetter;
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
  openThreadHeadId,
  profilePanelPubkey = null,
  setChannelManagementOpen,
  setExpandedThreadReplyIds,
  setOpenAgentSessionChannelId,
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
  const agentSessionAgents = managedAgents;

  // Breadcrumb for the Activity panel back arrow: captured on the
  // closed→open transition, consumed exactly once on back, cleared on any
  // other close so a stale target can't resurface later. Channel switches
  // drop it via the reset key.
  const { hasTarget: hasAgentSessionReturnTarget, store: returnTarget } =
    usePanelReturnTarget<AgentSessionReturnTarget>(activeChannelId);
  const isAgentSessionOpen = openAgentSessionPubkey != null;

  const closeAgentSession = React.useCallback(() => {
    returnTarget.clear();
    setOpenAgentSessionPubkey(null);
  }, [returnTarget, setOpenAgentSessionPubkey]);

  const openAgentSession = React.useCallback(
    (pubkey: string, channelId?: string | null) => {
      if (!isAgentSessionOpen) {
        returnTarget.capture(
          resolveAgentSessionReturnTarget({
            openThreadHeadId,
            profilePanelPubkey,
          }),
        );
      }
      setOpenThreadHeadId(null);
      setExpandedThreadReplyIds(new Set());
      setThreadScrollTargetId(null);
      setThreadReplyTargetId(null);
      setChannelManagementOpen(false);
      setOpenAgentSessionPubkey(pubkey);
      setOpenAgentSessionChannelId(channelId ?? null);
    },
    [
      isAgentSessionOpen,
      openThreadHeadId,
      profilePanelPubkey,
      returnTarget,
      setChannelManagementOpen,
      setExpandedThreadReplyIds,
      setOpenAgentSessionChannelId,
      setOpenAgentSessionPubkey,
      setOpenThreadHeadId,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  // Back restores the pane the Activity panel replaced; with no recorded
  // target (opened from the composer with no pane, or a direct/restored
  // `agentSession` URL) it simply closes — never a blind history pop.
  const backFromAgentSession = React.useCallback(() => {
    const target = returnTarget.consume();
    setOpenAgentSessionPubkey(null);
    if (target?.kind === "thread") {
      setOpenThreadHeadId(target.threadHeadId);
      return;
    }
    if (target?.kind === "profile") {
      setProfilePanelPubkey(target.pubkey);
    }
  }, [
    returnTarget,
    setOpenAgentSessionPubkey,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
  ]);

  const selectAgentSession = React.useCallback(
    (pubkey: string, channelId?: string | null) => {
      setOpenAgentSessionPubkey(pubkey);
      setOpenAgentSessionChannelId(channelId ?? null);
    },
    [setOpenAgentSessionChannelId, setOpenAgentSessionPubkey],
  );

  const openThreadAndCloseAgentSession = React.useCallback(
    (message: TimelineMessage) => {
      returnTarget.clear();
      setOpenAgentSessionPubkey(null);
      setProfilePanelPubkey(null);
      setChannelManagementOpen(false);
      handleOpenThread(message);
    },
    [
      handleOpenThread,
      returnTarget,
      setChannelManagementOpen,
      setOpenAgentSessionPubkey,
      setProfilePanelPubkey,
    ],
  );

  React.useEffect(() => {
    // An empty agent list can mean the queries behind it are still loading
    // (e.g. a reload restoring the agentSession URL param), so wait until the
    // agent queries have settled. Once loaded, a channel that legitimately has
    // zero agents will still auto-close a stale param.
    if (
      openAgentSessionPubkey &&
      agentsLoaded &&
      normalizePubkey(profilePanelPubkey ?? "") !==
        normalizePubkey(openAgentSessionPubkey) &&
      !agentSessionAgents.some(
        (agent) =>
          normalizePubkey(agent.pubkey) ===
          normalizePubkey(openAgentSessionPubkey),
      )
    ) {
      returnTarget.clear();
      setOpenAgentSessionPubkey(null, { replace: true });
    }
  }, [
    agentSessionAgents,
    agentsLoaded,
    openAgentSessionPubkey,
    profilePanelPubkey,
    returnTarget,
    setOpenAgentSessionPubkey,
  ]);

  return {
    agentSessionAgents,
    backFromAgentSession,
    channelAgentSessionAgents,
    closeAgentSession,
    hasAgentSessionReturnTarget,
    openAgentSession,
    openAgentSessionPubkey,
    openThreadAndCloseAgentSession,
    selectAgentSession,
  };
}
