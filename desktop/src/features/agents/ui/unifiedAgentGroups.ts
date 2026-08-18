import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

type PersonaGroup = { persona: AgentPersona; agents: ManagedAgent[] };

/**
 * Group managed agents under their personas for the Agents library.
 *
 * Archived instances are dropped from the standalone `ungrouped` (custom
 * agents) and `unknown` buckets so a relay-archived identity never shows as a
 * clickable library card of its own. Matched persona groups keep their full
 * instance list — the persona card resolves its own target through
 * `pickProfileAgent`, which applies the same `isArchived` filter and falls back
 * to persona-only mode when every instance is archived. `isArchived` is
 * fail-open (returns `false` while the relay archive snapshot loads).
 */
export function buildUnifiedGroups(
  personas: AgentPersona[],
  agents: ManagedAgent[],
  isArchived: (pubkey: string) => boolean,
) {
  const byPersonaId = new Map<string, ManagedAgent[]>();
  const ungrouped: ManagedAgent[] = [];

  for (const agent of agents) {
    if (!agent.personaId) {
      if (!isArchived(agent.pubkey)) ungrouped.push(agent);
    } else {
      const list = byPersonaId.get(agent.personaId) ?? [];
      list.push(agent);
      byPersonaId.set(agent.personaId, list);
    }
  }

  const matched = new Set<string>();
  const groups: PersonaGroup[] = personas.map((persona) => {
    matched.add(persona.id);
    return { persona, agents: byPersonaId.get(persona.id) ?? [] };
  });

  const unknown: ManagedAgent[] = [];
  for (const [id, list] of byPersonaId) {
    if (!matched.has(id)) {
      unknown.push(...list.filter((agent) => !isArchived(agent.pubkey)));
    }
  }

  return { groups, ungrouped, unknown };
}
