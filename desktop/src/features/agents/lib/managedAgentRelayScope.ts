import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";

/**
 * Scope a managed-agent list to one community's relay.
 *
 * `managed-agents.json` is one file per install and every record keeps the
 * relay URL its keypair was minted against. Changing a community's relay URL
 * mints new keypairs and appends them; the old records stay, so surfaces that
 * read the whole file see one entry per historical relay — the same agent name
 * repeated under keys that are not reachable on the relay the user is
 * currently connected to.
 *
 * Comparison is canonical (`canonicalRelayUrl`), so `localhost` vs `127.0.0.1`,
 * a default port, or a trailing slash still match the same relay.
 *
 * Both arguments are treated as unknown rather than empty when absent: a
 * `null`/unparsable relay URL returns the list untouched. A surface that
 * cannot say which relay it is on must not silently show nothing.
 */
export function managedAgentsForRelay<T extends { relayUrl: string }>(
  agents: readonly T[] | undefined,
  relayUrl: string | null | undefined,
): readonly T[] {
  if (!agents) return [];
  const canonical = relayUrl ? canonicalRelayUrl(relayUrl) : null;
  if (canonical === null) return agents;
  return agents.filter((agent) => {
    const agentCanonical = canonicalRelayUrl(agent.relayUrl);
    // An unparsable record is kept: dropping it would hide a real agent on the
    // strength of a URL we failed to read.
    return agentCanonical === null || agentCanonical === canonical;
  });
}
