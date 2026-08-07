import { normalizePubkey } from "@/shared/lib/pubkey";

import type { BotActivityAgent } from "./BotActivityBar";
import type { ManagedAgent } from "@/shared/api/types";

export type ChannelActivityAgent = BotActivityAgent & {
  status?: ManagedAgent["status"];
};

function workingPubkeySet(
  workingBotPubkeys: readonly string[],
): Set<string> {
  const working = new Set<string>();
  for (const pubkey of workingBotPubkeys) {
    working.add(normalizePubkey(pubkey));
  }
  return working;
}

export function countWorkingChannelAgents(
  agents: ChannelActivityAgent[],
  workingBotPubkeys: readonly string[],
): number {
  if (workingBotPubkeys.length === 0 || agents.length === 0) {
    return 0;
  }
  const working = workingPubkeySet(workingBotPubkeys);
  let count = 0;
  for (const agent of agents) {
    if (working.has(normalizePubkey(agent.pubkey))) {
      count += 1;
    }
  }
  return count;
}

/** View all when two or more agents are actively working in this channel. */
export function shouldShowViewAllAgentActivity({
  agents,
  workingBotPubkeys,
}: {
  agents: ChannelActivityAgent[];
  workingBotPubkeys: readonly string[];
}): boolean {
  // Fast path: fewer than two working signals → no need to scan agents.
  if (workingBotPubkeys.length < 2) {
    return false;
  }
  return countWorkingChannelAgents(agents, workingBotPubkeys) >= 2;
}

/** Only agents currently working — idle harnesses stay out of View all. */
export function agentsForAllActivityPanel({
  agents,
  workingBotPubkeys,
}: {
  agents: ChannelActivityAgent[];
  workingBotPubkeys: readonly string[];
}): ChannelActivityAgent[] {
  if (workingBotPubkeys.length === 0) {
    return [];
  }
  const working = workingPubkeySet(workingBotPubkeys);
  const panelAgents: ChannelActivityAgent[] = [];
  for (const agent of agents) {
    if (working.has(normalizePubkey(agent.pubkey))) {
      panelAgents.push(agent);
    }
  }
  return panelAgents;
}
