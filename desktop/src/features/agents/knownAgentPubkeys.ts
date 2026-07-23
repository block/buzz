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

/**
 * Channel-scoped variant: the managed ∪ relay baseline plus this channel's
 * bot members (role `bot` or `isAgent`), so member-only agents are included.
 */
export function mergeChannelKnownAgentPubkeys(
  channelMembers:
    | readonly { pubkey: string; role: string; isAgent: boolean }[]
    | undefined,
  managedAgents: readonly { pubkey: string }[] | undefined,
  relayAgents: readonly { pubkey: string }[] | undefined,
): ReadonlySet<string> {
  const pubkeys = new Set(mergeKnownAgentPubkeys(managedAgents, relayAgents));
  for (const member of channelMembers ?? []) {
    if (member.role === "bot" || member.isAgent) {
      pubkeys.add(normalizePubkey(member.pubkey));
    }
  }
  return pubkeys;
}
