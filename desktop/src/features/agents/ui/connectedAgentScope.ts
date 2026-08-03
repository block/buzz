import type { ConnectedAgent } from "@/shared/api/remoteAgentTypes";

/** Normalize a relay URL so equivalent community spellings compare equally. */
export function normalizeCommunityUrl(url: string): string {
  return url.trim().replace(/\/+$/, "").toLowerCase();
}

/** Return the connected agents relevant to the currently active community. */
export function connectedAgentsForCommunity(
  agents: ConnectedAgent[],
  activeRelayUrl: string | null | undefined,
): ConnectedAgent[] {
  if (!activeRelayUrl) return agents;
  const active = normalizeCommunityUrl(activeRelayUrl);
  return agents.filter(
    (agent) =>
      !agent.community || normalizeCommunityUrl(agent.community) === active,
  );
}
