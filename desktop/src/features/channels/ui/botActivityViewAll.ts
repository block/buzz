import type { ManagedAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

import type { BotActivityAgent } from "./BotActivityBar";

export type ChannelActivityAgent = BotActivityAgent & {
  status?: ManagedAgent["status"];
};

/** Harness is live in this channel (local running or deployed/remote). */
export function isActiveChannelAgent(agent: ChannelActivityAgent): boolean {
  if (!agent.status) {
    return true;
  }
  return agent.status === "running" || agent.status === "deployed";
}

export function countActiveChannelAgents(agents: ChannelActivityAgent[]): number {
  return agents.filter(isActiveChannelAgent).length;
}

export function countWorkingChannelAgents(
  agents: ChannelActivityAgent[],
  workingBotPubkeys: readonly string[],
): number {
  const working = new Set(
    workingBotPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  return agents.filter((agent) =>
    working.has(normalizePubkey(agent.pubkey)),
  ).length;
}

/** View all when two or more agents are running in channel or actively working. */
export function shouldShowViewAllAgentActivity({
  agents,
  workingBotPubkeys,
}: {
  agents: ChannelActivityAgent[];
  workingBotPubkeys: readonly string[];
}): boolean {
  return (
    countActiveChannelAgents(agents) >= 2 ||
    countWorkingChannelAgents(agents, workingBotPubkeys) >= 2
  );
}

export function agentsForAllActivityPanel({
  agents,
  workingBotPubkeys,
}: {
  agents: ChannelActivityAgent[];
  workingBotPubkeys: readonly string[];
}): ChannelActivityAgent[] {
  const working = new Set(
    workingBotPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  return agents.filter(
    (agent) =>
      working.has(normalizePubkey(agent.pubkey)) ||
      isActiveChannelAgent(agent),
  );
}
