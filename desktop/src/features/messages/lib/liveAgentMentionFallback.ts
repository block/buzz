import { normalizePubkey } from "@/shared/lib/pubkey";

/** Minimal shape of a relay-directory agent needed for mention fallback. */
export type LiveRelayAgentRef = {
  name: string;
  pubkey: string;
};

/**
 * Resolve a persona mention to an already-live relay agent with the same
 * display name, if one exists.
 *
 * Persona mentions normally instantiate a local agent. On a desktop that is
 * not the agent's host machine that spawn fails (harness definitions are
 * per-machine), the send is blocked, and each attempt mints an orphan
 * keypair. When an agent with the persona's name is already registered on
 * the relay, the mention should tag that agent instead. Prefers an agent
 * that is already a member of the current channel; falls back to the first
 * name match.
 *
 * Returns the normalized pubkey to mention, or null when no live agent
 * matches (the caller proceeds with persona instantiation).
 */
export function resolveLivePersonaMentionPubkey(
  relayAgents: readonly LiveRelayAgentRef[] | undefined,
  displayName: string,
  memberPubkeys: ReadonlySet<string>,
): string | null {
  const needle = displayName.trim().toLowerCase();
  if (!needle) {
    return null;
  }
  const matches = (relayAgents ?? [])
    .filter((agent) => agent.name.trim().toLowerCase() === needle)
    .map((agent) => normalizePubkey(agent.pubkey));
  if (matches.length === 0) {
    return null;
  }
  return matches.find((pubkey) => memberPubkeys.has(pubkey)) ?? matches[0];
}

/**
 * True when a managed-agent mention should skip the local start attempt:
 * the agent is already online on the relay, so it is running somewhere —
 * possibly hosted by another machine whose runtime this desktop cannot
 * (and must not) duplicate. Tag it without touching the local runtime.
 */
export function shouldSkipLocalStartForOnlineAgent(
  presenceStatus: string | undefined,
): boolean {
  return presenceStatus === "online";
}
