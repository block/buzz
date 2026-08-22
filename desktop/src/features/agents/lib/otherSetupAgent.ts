import type { ManagedAgent, RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

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

  return Boolean(
    ownerPubkey &&
      normalizePubkey(ownerPubkey) === normalizePubkey(currentPubkey),
  );
}
