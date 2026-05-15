import * as React from "react";

import type { TimelineMessage } from "@/features/messages/types";
import type {
  Channel,
  ChannelMember,
  ManagedAgent,
  RelayAgent,
} from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

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
  channelMembers?: ChannelMember[];
  handleOpenThread: (message: TimelineMessage) => void;
  managedAgents: ChannelAgentSessionAgent[];
  setExpandedThreadReplyIds: (value: Set<string>) => void;
  setOpenThreadHeadId: (value: string | null) => void;
  setProfilePanelPubkey: (value: string | null) => void;
  setThreadReplyTargetId: (value: string | null) => void;
  setThreadScrollTargetId: (value: string | null) => void;
  targetMessageId: string | null;
  timelineMessages: TimelineMessage[];
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
  channelMembers,
  handleOpenThread,
  managedAgents,
  setExpandedThreadReplyIds,
  setOpenThreadHeadId,
  setProfilePanelPubkey,
  setThreadReplyTargetId,
  setThreadScrollTargetId,
  targetMessageId,
  timelineMessages,
}: UseChannelAgentSessionsOptions) {
  const [openAgentSessionPubkey, setOpenAgentSessionPubkey] = React.useState<
    string | null
  >(null);
  const handledThreadTargetIdRef = React.useRef<string | null>(null);

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
  }, []);

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
      setOpenThreadHeadId,
      setProfilePanelPubkey,
      setThreadReplyTargetId,
      setThreadScrollTargetId,
    ],
  );

  const selectAgentSession = React.useCallback((pubkey: string) => {
    setOpenAgentSessionPubkey(pubkey);
  }, []);

  const openThreadAndCloseAgentSession = React.useCallback(
    (message: TimelineMessage) => {
      setOpenAgentSessionPubkey(null);
      setProfilePanelPubkey(null);
      handleOpenThread(message);
    },
    [handleOpenThread, setProfilePanelPubkey],
  );

  React.useEffect(() => {
    if (!targetMessageId) {
      handledThreadTargetIdRef.current = null;
      return;
    }

    const targetKey = `${activeChannelId ?? "none"}:${targetMessageId}`;
    if (
      handledThreadTargetIdRef.current !== null &&
      handledThreadTargetIdRef.current !== targetKey
    ) {
      handledThreadTargetIdRef.current = null;
    }

    if (
      handledThreadTargetIdRef.current === targetKey ||
      !activeChannel ||
      activeChannel.channelType === "forum"
    ) {
      return;
    }

    const targetMessage =
      timelineMessages.find((message) => message.id === targetMessageId) ??
      null;

    if (!targetMessage?.parentId) {
      return;
    }

    const threadHeadId = targetMessage.rootId ?? targetMessage.parentId;
    const messageById = new Map(
      timelineMessages.map((message) => [message.id, message]),
    );

    if (!messageById.has(threadHeadId)) {
      return;
    }

    const expandedReplyIds = new Set<string>();
    let ancestorId: string | null = targetMessage.parentId;
    let guard = 0;

    while (
      ancestorId &&
      ancestorId !== threadHeadId &&
      guard < timelineMessages.length
    ) {
      expandedReplyIds.add(ancestorId);
      ancestorId = messageById.get(ancestorId)?.parentId ?? null;
      guard += 1;
    }

    setOpenAgentSessionPubkey(null);
    setProfilePanelPubkey(null);
    setOpenThreadHeadId(threadHeadId);
    setThreadReplyTargetId(threadHeadId);
    setThreadScrollTargetId(targetMessageId);
    setExpandedThreadReplyIds(expandedReplyIds);
    handledThreadTargetIdRef.current = targetKey;
  }, [
    activeChannel,
    activeChannelId,
    setExpandedThreadReplyIds,
    setOpenThreadHeadId,
    setProfilePanelPubkey,
    setThreadReplyTargetId,
    setThreadScrollTargetId,
    targetMessageId,
    timelineMessages,
  ]);

  React.useEffect(() => {
    if (
      openAgentSessionPubkey &&
      !channelAgentSessionAgents.some(
        (agent) =>
          normalizePubkey(agent.pubkey) ===
          normalizePubkey(openAgentSessionPubkey),
      )
    ) {
      setOpenAgentSessionPubkey(null);
    }
  }, [channelAgentSessionAgents, openAgentSessionPubkey]);

  return {
    channelAgentSessionAgents,
    closeAgentSession,
    openAgentSession,
    openAgentSessionPubkey,
    openThreadAndCloseAgentSession,
    selectAgentSession,
  };
}
