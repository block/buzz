import type { PresenceLookup, PresenceStatus } from "@/shared/api/types";

/**
 * The status fields this merge reads off an agent, whatever its source.
 *
 * Kept structural rather than importing `RelayAgent` so locally-managed agents
 * projected into the same shape can be merged without a cast.
 */
export type AgentStatusInput = {
  pubkey: string;
  status: PresenceStatus;
};

/**
 * Build the pubkey → status map for an agent roster, preferring live relay
 * presence over the agent's self-declared `kind:10100` status.
 *
 * Why presence wins: the `status` field on a `kind:10100` agent profile has no
 * producer in-tree, so `agents_from_events` falls back to `"offline"` for every
 * relay-discovered agent. Rendering that directly reports agents as offline
 * whether or not they are actually running. Relay presence (kind:20001, backed
 * by a 180s Redis TTL) is the only source that reflects the process's real
 * state, so it takes precedence wherever it has an entry.
 *
 * Agents absent from `presence` keep their incoming status: locally-managed
 * agents already derive theirs from the live process handle, which is
 * authoritative for the machine running them and is not something the relay
 * can contradict.
 */
export function mergeAgentPresenceStatus(
  agents: readonly AgentStatusInput[],
  presence: PresenceLookup | undefined,
): Record<string, PresenceStatus> {
  const map: Record<string, PresenceStatus> = {};
  for (const agent of agents) {
    map[agent.pubkey] = presence?.[agent.pubkey] ?? agent.status;
  }
  return map;
}
