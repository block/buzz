import { normalizePubkey } from "@/shared/lib/pubkey";

/**
 * THE definition of agent identity, shared by every surface that answers
 * "which agents exist" (@-mention autocomplete, the Agents library, the
 * profile panel). Two surfaces that hand-roll this drift apart, and the drift
 * is invisible until an agent becomes unreachable on one of them.
 *
 * Pubkeys—not persona metadata or a display name—are agent identities. A
 * persona may be installed more than once, and an owner may intentionally
 * create multiple same-named agents. Collapsing either case makes one agent
 * impossible to choose from autocomplete, and impossible to manage from the
 * Agents library.
 */
export type AgentIdentityInput = { pubkey?: string | null };

export function agentIdentityKey(candidate: AgentIdentityInput): string | null {
  const pubkey = candidate.pubkey?.trim();
  return pubkey ? `pubkey:${normalizePubkey(pubkey)}` : null;
}

export type AgentDisplayInput = AgentIdentityInput & {
  name?: string | null;
  personaId?: string | null;
};

/**
 * Presentation-only key: which agents may legitimately share ONE card in the
 * Agents library. This is NOT an identity — it is a statement about what a
 * single label can truthfully stand for. Instances of one persona that all
 * carry the same name are interchangeable on a card (the card's profile panel
 * lists every instance behind it); an instance the owner renamed is not, and
 * must get a card of its own or it disappears from the library.
 *
 * Every caller must keep the full identity list of a display group reachable —
 * grouping may never drop an `agentIdentityKey`.
 */
export function agentDisplayGroupKey(agent: AgentDisplayInput): string {
  const personaId = agent.personaId?.trim() ?? "";
  return `persona:${personaId}|name:${foldAgentDisplayName(agent.name)}`;
}

/**
 * The one place a display name is folded for comparison. Callers that need a
 * name-scoped key of their own (React keys, `data-testid`s) must fold through
 * this rather than lowercasing inline, or their key and the group's disagree.
 */
export function foldAgentDisplayName(name: string | null | undefined): string {
  return name?.trim().toLowerCase() ?? "";
}

export type AgentDisplayGroup<T> = {
  key: string;
  /** Trimmed display name shared by every member, empty when unnamed. */
  name: string;
  /** `name` folded for comparison — the group's identity within a persona. */
  foldedName: string;
  agents: T[];
};

/**
 * Split agents into display groups, in first-seen order, dropping repeated
 * records of the same identity (the same pubkey read from two sources) but
 * never dropping a distinct identity.
 */
export function groupAgentsForDisplay<T extends AgentDisplayInput>(
  agents: readonly T[],
): AgentDisplayGroup<T>[] {
  const groups: AgentDisplayGroup<T>[] = [];
  const groupsByKey = new Map<string, AgentDisplayGroup<T>>();
  const seenIdentities = new Set<string>();

  for (const agent of agents) {
    const identity = agentIdentityKey(agent);
    if (identity) {
      if (seenIdentities.has(identity)) continue;
      seenIdentities.add(identity);
    }

    const key = agentDisplayGroupKey(agent);
    const existing = groupsByKey.get(key);
    if (existing) {
      existing.agents.push(agent);
      continue;
    }

    const group = {
      key,
      name: agent.name?.trim() ?? "",
      foldedName: foldAgentDisplayName(agent.name),
      agents: [agent],
    };
    groupsByKey.set(key, group);
    groups.push(group);
  }

  return groups;
}
