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
  avatarUrl?: string | null;
  canInterruptTurn: boolean;
  channelIds?: string[];
  channels?: string[];
};

function relayStatusToManagedStatus(
  status: RelayAgent["status"],
): ManagedAgent["status"] {
  return status === "offline" ? "stopped" : "deployed";
}

function isAgentChannelMember(member: ChannelMember) {
  return member.role === "bot" || member.isAgent;
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
      avatarUrl: agent.avatarUrl,
      canInterruptTurn: true,
      channelIds: existing?.channelIds,
      channels: existing?.channels,
    });
  }

  for (const member of channelMembers ?? []) {
    const key = normalizePubkey(member.pubkey);
    if (!isAgentChannelMember(member) || byPubkey.has(key)) {
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
          .filter(isAgentChannelMember)
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

export function resolveOpenAgentSessionAgent({
  allAgentCandidates,
  channelAgentSessionAgents,
  openAgentSessionPubkey,
}: {
  allAgentCandidates: ChannelAgentSessionAgent[];
  channelAgentSessionAgents: ChannelAgentSessionAgent[];
  openAgentSessionPubkey: string | null;
}): ChannelAgentSessionAgent | null {
  if (!openAgentSessionPubkey) {
    return null;
  }

  const normalized = normalizePubkey(openAgentSessionPubkey);
  return (
    channelAgentSessionAgents.find(
      (agent) => normalizePubkey(agent.pubkey) === normalized,
    ) ??
    allAgentCandidates.find(
      (agent) => normalizePubkey(agent.pubkey) === normalized,
    ) ?? {
      pubkey: openAgentSessionPubkey,
      name: openAgentSessionPubkey.slice(0, 8),
      status: "deployed",
      agentSource: "relay",
      canInterruptTurn: false,
    }
  );
}
