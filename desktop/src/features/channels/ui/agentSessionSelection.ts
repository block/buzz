import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { BotActivityAgent } from "@/features/channels/ui/BotActivityBar";
import type { ChannelAgentSessionAgent } from "@/features/channels/ui/useChannelAgentSessions";

export function resolveSelectedAgentSession({
  agentSessionAgents,
  openAgentSessionPubkey,
  profilePanelPubkey,
  profiles,
}: {
  agentSessionAgents: ChannelAgentSessionAgent[];
  openAgentSessionPubkey: string | null;
  profilePanelPubkey?: string | null;
  profiles?: UserProfileLookup;
}): ChannelAgentSessionAgent | null {
  if (!openAgentSessionPubkey) {
    return null;
  }

  const listedAgent = agentSessionAgents.find(
    (agent) =>
      agent.pubkey.toLowerCase() === openAgentSessionPubkey.toLowerCase(),
  );
  if (listedAgent) {
    return listedAgent;
  }

  if (
    !profilePanelPubkey ||
    profilePanelPubkey.toLowerCase() !== openAgentSessionPubkey.toLowerCase()
  ) {
    return null;
  }

  const profile = profiles?.[openAgentSessionPubkey.toLowerCase()];
  return {
    pubkey: openAgentSessionPubkey,
    name: profile?.displayName?.trim() || "Agent",
    status: "deployed",
    agentSource: "relay",
    canInterruptTurn: false,
  };
}

export function isAgentInActivityList({
  activityAgents,
  selectedAgent,
}: {
  activityAgents: BotActivityAgent[];
  selectedAgent: ChannelAgentSessionAgent | null;
}) {
  if (!selectedAgent) {
    return false;
  }

  return activityAgents.some(
    (agent) =>
      agent.pubkey.toLowerCase() === selectedAgent.pubkey.toLowerCase(),
  );
}
