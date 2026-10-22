import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

/** Owned identity absent from the loaded local inventory; not evidence of hosting location. */
export function isOtherSetupAgent({
  agentDirectoriesReady,
  currentPubkey,
  managedAgents,
  profileOwnerPubkey,
  pubkey,
  relayAgents,
}: {
  agentDirectoriesReady: boolean;
  currentPubkey?: string;
  managedAgents: readonly ManagedAgent[];
  profileOwnerPubkey?: string | null;
  pubkey: string;
  relayAgents: readonly RelayAgent[];
}): boolean {
  if (!agentDirectoriesReady || !currentPubkey) return false;

  const normalizedPubkey = normalizePubkey(pubkey);
  if (
    managedAgents.some(
      (agent) => normalizePubkey(agent.pubkey) === normalizedPubkey,
    )
  ) {
    return false;
  }

  const relayOwnerPubkey = relayAgents.find(
    (agent) => normalizePubkey(agent.pubkey) === normalizedPubkey,
  )?.ownerPubkey;
  const ownerPubkey = profileOwnerPubkey ?? relayOwnerPubkey;

  return isOwnedAgentNotManagedOnDevice({
    currentPubkey,
    ownerPubkey,
    localInventoryReady: agentDirectoriesReady,
    isLocallyManaged: false,
  });
}

/**
 * Relay agents whose verified owner is the viewer and that have no exact-key
 * local record: the Agents page's read-only "Running elsewhere" list. Empty
 * until both directories have loaded, so a pending local inventory never
 * lists a locally managed agent twice. Archived identities are omitted.
 */
export function ownedAgentsNotManagedOnDevice({
  agentDirectoriesReady,
  currentPubkey,
  isArchived,
  managedAgents,
  relayAgents,
}: {
  agentDirectoriesReady: boolean;
  currentPubkey?: string;
  isArchived: (pubkey: string) => boolean;
  managedAgents: readonly ManagedAgent[];
  relayAgents: readonly RelayAgent[];
}): RelayAgent[] {
  if (!agentDirectoriesReady || !currentPubkey) return [];

  const localPubkeys = new Set(
    managedAgents.map((agent) => normalizePubkey(agent.pubkey)),
  );
  const byPubkey = new Map<string, RelayAgent>();
  for (const agent of relayAgents) {
    const key = normalizePubkey(agent.pubkey);
    if (byPubkey.has(key) || isArchived(agent.pubkey)) continue;
    if (
      isOwnedAgentNotManagedOnDevice({
        currentPubkey,
        ownerPubkey: agent.ownerPubkey,
        localInventoryReady: true,
        isLocallyManaged: localPubkeys.has(key),
      })
    ) {
      byPubkey.set(key, agent);
    }
  }
  return [...byPubkey.values()].sort(
    (left, right) =>
      left.name.localeCompare(right.name) ||
      normalizePubkey(left.pubkey).localeCompare(normalizePubkey(right.pubkey)),
  );
}

/** Presentation provenance only; neither hosting location nor availability. */
export function isOwnedAgentNotManagedOnDevice({
  currentPubkey,
  ownerPubkey,
  localInventoryReady,
  isLocallyManaged,
}: {
  currentPubkey?: string;
  ownerPubkey?: string | null;
  localInventoryReady: boolean;
  isLocallyManaged: boolean;
}): boolean {
  return Boolean(
    localInventoryReady &&
      !isLocallyManaged &&
      currentPubkey &&
      ownerPubkey &&
      normalizePubkey(ownerPubkey) === normalizePubkey(currentPubkey),
  );
}
