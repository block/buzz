// What the web Agents page lists when the data comes over the relay bridge: ONLY running agents.
// TODO(direct): a direct-tailnet management connection may show all agents (pass showAll = true).
import type { AgentSummary } from "./protocol";

export function visibleRelayAgents<T extends Pick<AgentSummary, "status">>(agents: readonly T[], showAll = false): T[] {
  return showAll ? [...agents] : agents.filter((a) => a.status === "running");
}

export function hiddenAgentCount(agents: readonly Pick<AgentSummary, "status">[], showAll = false): number {
  return agents.length - visibleRelayAgents(agents, showAll).length;
}

export const NO_RUNNING_AGENTS = "No agents running on your desktop.";
