import { normalizePubkey } from "@/shared/lib/pubkey";

import type { BotActivityAgent } from "./BotActivityBar";
import type { ManagedAgent } from "@/shared/api/types";

export type ChannelActivityAgent = BotActivityAgent & {
  status?: ManagedAgent["status"];
};

export function countWorkingChannelAgents(
  agents: ChannelActivityAgent[],
  workingBotPubkeys: readonly string[],
): number {
  const working = new Set(
    workingBotPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  return agents.filter((agent) => working.has(normalizePubkey(agent.pubkey)))
    .length;
}

/** View all when two or more agents are actively working in this channel. */
export function shouldShowViewAllAgentActivity({
  agents,
  workingBotPubkeys,
}: {
  agents: ChannelActivityAgent[];
  workingBotPubkeys: readonly string[];
}): boolean {
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
  const working = new Set(
    workingBotPubkeys.map((pubkey) => normalizePubkey(pubkey)),
  );
  return agents.filter((agent) => working.has(normalizePubkey(agent.pubkey)));
}
