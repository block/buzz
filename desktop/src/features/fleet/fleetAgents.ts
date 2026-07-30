/**
 * Fleet roster composition. Pure module — the hook that feeds it lives in
 * `useFleetAgents.ts` so these rules stay unit-testable without React.
 */

import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * One row on the fleet board. Managed agents carry full lifecycle metadata;
 * declared-owned relay agents (NIP-OA `ownerPubkey == me`, running on another
 * machine) surface with the reduced fields the relay roster provides — the
 * same ownership rule `useAgentObserverIngestion` uses to decide whose
 * observer frames to decrypt, so every agent shown here also has live data.
 */
export type FleetAgent = {
  pubkey: string;
  name: string;
  avatarUrl: string | null;
  /** Managed-agent process status; owned relay agents map presence → status. */
  status: ManagedAgent["status"];
  isManaged: boolean;
  model: string | null;
  lastError: string | null;
  lastErrorCode: number | null;
  lastStartedAt: string | null;
  lastStoppedAt: string | null;
};

function fromManagedAgent(agent: ManagedAgent): FleetAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    avatarUrl: agent.avatarUrl,
    status: agent.status,
    isManaged: true,
    model: agent.model,
    lastError: agent.lastError,
    lastErrorCode: agent.lastErrorCode,
    lastStartedAt: agent.lastStartedAt,
    lastStoppedAt: agent.lastStoppedAt,
  };
}

function fromOwnedRelayAgent(
  agent: Pick<RelayAgent, "pubkey" | "name" | "status">,
  avatarUrl: string | null,
): FleetAgent {
  return {
    pubkey: agent.pubkey,
    name: agent.name,
    avatarUrl,
    // A relay-roster agent that is not locally managed runs elsewhere; its
    // presence is the only liveness fact available. Offline presence maps to
    // `not_deployed` rather than `stopped` — remotely we cannot distinguish
    // a deliberate stop from an expired presence TTL.
    status: agent.status === "offline" ? "not_deployed" : "deployed",
    isManaged: false,
    model: null,
    lastError: null,
    lastErrorCode: null,
    lastStartedAt: null,
    lastStoppedAt: null,
  };
}

/**
 * Combine locally managed agents with relay agents the current identity
 * declared-owns into one fleet list. Managed agents win pubkey collisions
 * (they carry richer metadata); the result is name-sorted for a stable grid.
 * Mirrors `combineObserverIngestionAgents` — the fleet shows exactly the set
 * of agents whose observer frames this desktop decrypts.
 */
export function combineFleetAgents(
  managedAgents: readonly ManagedAgent[],
  relayAgents: readonly RelayAgent[],
  ownerByPubkey: ReadonlyMap<string, string>,
  avatarByPubkey: ReadonlyMap<string, string | null>,
  currentPubkey: string | null | undefined,
): FleetAgent[] {
  const fleet = managedAgents.map(fromManagedAgent);
  const managedSet = new Set(
    managedAgents.map((agent) => normalizePubkey(agent.pubkey)),
  );

  if (currentPubkey) {
    const me = normalizePubkey(currentPubkey);
    for (const agent of relayAgents) {
      const key = normalizePubkey(agent.pubkey);
      if (managedSet.has(key)) continue;
      const owner = ownerByPubkey.get(key);
      if (!owner || normalizePubkey(owner) !== me) continue;
      fleet.push(fromOwnedRelayAgent(agent, avatarByPubkey.get(key) ?? null));
    }
  }

  return fleet.sort(
    (left, right) =>
      left.name.localeCompare(right.name) ||
      left.pubkey.localeCompare(right.pubkey),
  );
}
