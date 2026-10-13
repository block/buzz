import type { RelayAgent } from "@/shared/api/types";

export function selectCommunityAgents(
  relayAgents: readonly RelayAgent[],
  isLocallyManaged: (pubkey: string) => boolean,
): RelayAgent[] {
  return [...relayAgents]
    .filter((agent) => !isLocallyManaged(agent.pubkey))
    .sort((left, right) => left.name.localeCompare(right.name));
}

export function communityAgentChannelLabels(
  agent: RelayAgent,
  channelIdToName: Readonly<Record<string, string>>,
): string[] {
  const labels: string[] = [];
  const seen = new Set<string>();

  for (const channelId of agent.channelIds) {
    const label = channelIdToName[channelId];
    const normalized = label?.trim();
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    labels.push(normalized);
  }

  if (agent.channelIds.length === 0) {
    for (const channel of agent.channels) {
      const normalized = channel.trim();
      if (!normalized || seen.has(normalized)) continue;
      seen.add(normalized);
      labels.push(normalized);
    }
  }

  return labels;
}

export function communityAgentChannelSummary(
  agent: RelayAgent,
  channelIdToName: Readonly<Record<string, string>>,
): string {
  const labels = communityAgentChannelLabels(agent, channelIdToName);
  if (labels.length === 0) {
    const count = agent.channelIds.length || agent.channels.length;
    if (count === 1) return "1 shared channel";
    if (count > 1) return `${count} shared channels`;
    return "No shared channels";
  }

  const visible = labels.slice(0, 2).map((label) => `#${label}`);
  const remaining = labels.length - visible.length;
  return remaining > 0
    ? `${visible.join(" · ")} · +${remaining}`
    : visible.join(" · ");
}
