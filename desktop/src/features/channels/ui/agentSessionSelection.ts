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

/**
 * Where the Activity panel should return to when its back arrow fires.
 *
 * Captured when the panel opens (see useChannelAgentSessions) and consumed
 * exactly once on back — an explicit breadcrumb instead of popping the
 * app/browser history stack.
 */
export type AgentSessionReturnTarget =
  | { kind: "profile"; pubkey: string }
  | { kind: "thread"; threadHeadId: string };

/**
 * Resolve the pane the Activity panel is opening over. Threads win over the
 * profile panel because that's the render priority of the right pane — a
 * lingering `profile` URL param never shows while a thread is open.
 * Returns null when Activity opens over no pane (composer/activity bar from
 * the main timeline, or a direct/restored `agentSession` URL).
 */
export function resolveAgentSessionReturnTarget({
  openThreadHeadId,
  profilePanelPubkey,
}: {
  openThreadHeadId: string | null;
  profilePanelPubkey: string | null;
}): AgentSessionReturnTarget | null {
  if (openThreadHeadId) {
    return { kind: "thread", threadHeadId: openThreadHeadId };
  }

  if (profilePanelPubkey) {
    return { kind: "profile", pubkey: profilePanelPubkey };
  }

  return null;
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
