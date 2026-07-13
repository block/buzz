import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * Pure merge behind `useKnownAgentPubkeys`: managed agents ∪ relay agents,
 * normalised via `normalizePubkey` so membership checks work against
 * normalised pubkeys.
 *
 * Structurally typed on `{ pubkey }` so node unit tests don't need to build
 * full `ManagedAgent`/`RelayAgent` values.
 */
export function mergeKnownAgentPubkeys(
  managedAgents: readonly { pubkey: string }[] | undefined,
  relayAgents: readonly { pubkey: string }[] | undefined,
): ReadonlySet<string> {
  const pubkeys = new Set<string>();
  for (const agent of managedAgents ?? []) {
    pubkeys.add(normalizePubkey(agent.pubkey));
  }
  for (const agent of relayAgents ?? []) {
    pubkeys.add(normalizePubkey(agent.pubkey));
  }
  return pubkeys;
}
