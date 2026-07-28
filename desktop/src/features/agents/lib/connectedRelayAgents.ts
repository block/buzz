import type { RelayAgent } from "@/shared/api/types";
import { normalizePubkey } from "@/shared/lib/pubkey";

export function connectedRelayAgents(
  relayAgents: readonly RelayAgent[],
  managedPubkeys: ReadonlySet<string>,
): RelayAgent[] {
  return [...relayAgents]
    .filter((agent) => !managedPubkeys.has(normalizePubkey(agent.pubkey)))
    .sort((left, right) => left.name.localeCompare(right.name));
}
