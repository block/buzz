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
