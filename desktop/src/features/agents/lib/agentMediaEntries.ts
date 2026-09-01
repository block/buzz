import type { AgentMediaSession } from "./agentMediaSession";

/** One live agent, as a channel indicator lists it. */
export type AgentMediaEntry = {
  agentPubkey: string;
  label: string;
};

/**
 * One entry per live agent, newest session first.
 *
 * Deduped by pubkey, which the session list is not: nothing stops an agent
 * announcing a second session before its first expires, since the relay
 * enforces no one-at-a-time rule. Two rows for one agent would both open the
 * same panel while the badge beside them claimed two agents were live. The
 * first wins, and sessions arrive newest first, so that is the current one.
 *
 * `labelFor` is called once per agent rather than once per session, and the
 * caller keeps profile resolution — this module stays testable without a query
 * client.
 */
export function agentMediaEntries(
  sessions: readonly Pick<AgentMediaSession, "agentPubkey">[],
  labelFor: (agentPubkey: string) => string,
): AgentMediaEntry[] {
  const entries: AgentMediaEntry[] = [];
  const seen = new Set<string>();
  for (const session of sessions) {
    if (seen.has(session.agentPubkey)) continue;
    seen.add(session.agentPubkey);
    entries.push({
      agentPubkey: session.agentPubkey,
      label: labelFor(session.agentPubkey),
    });
  }
  return entries;
}

/**
 * What the indicator says it is showing.
 *
 * Counts agents, not sessions, because that is what the wording claims. An
 * agent with a stale session still live alongside its current one is one agent.
 */
export function describeAgentMediaEntries(
  entries: readonly AgentMediaEntry[],
): string {
  if (entries.length === 0) return "";
  if (entries.length === 1) return `${entries[0].label} is live`;
  return `${entries.length} agents are live`;
}
