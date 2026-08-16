import { groupAgentsForDisplay } from "@/features/agents/lib/agentIdentity";
import { pickProfileAgent } from "@/features/agents/lib/pickProfileAgent";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

/**
 * One card in the Agents library.
 *
 * A card stands for every instance in `agents` and opens `agent` — never for
 * an instance it does not list. Persona grouping decides which cards exist;
 * it may never decide which agents exist (see `agentDisplayGroupKey`).
 */
export type UnifiedAgentCard = {
  /** Stable card key, also used for the card's `data-testid`. */
  key: string;
  label: string;
  persona: AgentPersona;
  /** Instance the card opens; `undefined` for a persona with no instances. */
  agent: ManagedAgent | undefined;
  /** Every instance the card stands for. */
  agents: ManagedAgent[];
  /** Persona-level actions (edit/share/delete) live on exactly one card. */
  ownsPersonaActions: boolean;
};

export type PersonaGroup = {
  persona: AgentPersona;
  agents: ManagedAgent[];
  cards: UnifiedAgentCard[];
};

/**
 * Cards for one persona.
 *
 * All of a persona's instances carrying the same name stay collapsed onto the
 * persona's single card — that is the established gallery behaviour, and the
 * card's profile panel lists every instance behind it. Once the owner renames
 * an instance, the persona's name can no longer truthfully stand for all of
 * them, so each surviving name gets its own card. Without this, a rename made
 * an agent vanish from the library entirely.
 *
 * A split card only exists for a name that still has a live instance, so an
 * archived identity never becomes a clickable card of its own. When archiving
 * leaves no live instance under any name, the persona collapses back to a
 * single persona-only card rather than disappearing from the library.
 */
function buildPersonaCards(
  persona: AgentPersona,
  agents: ManagedAgent[],
  isArchived: (pubkey: string) => boolean,
): UnifiedAgentCard[] {
  const displayGroups = groupAgentsForDisplay(agents);
  const personaOnlyCard = (members: ManagedAgent[]): UnifiedAgentCard => ({
    key: persona.id,
    label: persona.displayName,
    persona,
    agent: pickProfileAgent(members, isArchived),
    agents: members,
    ownsPersonaActions: true,
  });

  if (displayGroups.length <= 1) {
    return [personaOnlyCard(displayGroups[0]?.agents ?? [])];
  }

  const liveGroups = displayGroups
    .map((group) => ({
      group,
      primary: pickProfileAgent(group.agents, isArchived),
    }))
    .filter(
      (entry): entry is { group: (typeof displayGroups)[number]; primary: ManagedAgent } =>
        entry.primary !== undefined,
    );

  if (liveGroups.length === 0) return [personaOnlyCard(agents)];

  const personaPrimary = pickProfileAgent(agents, isArchived);
  const cards = liveGroups.map(({ group, primary }) => ({
    key: `${persona.id}-${primary.pubkey}`,
    label: group.name || persona.displayName,
    persona,
    agent: primary,
    agents: group.agents,
    ownsPersonaActions: personaPrimary
      ? group.agents.includes(personaPrimary)
      : false,
  }));

  // Persona actions must exist on exactly one card. `personaPrimary` is live so
  // its own group survived the filter, but instance de-duplication can drop the
  // very record it points at; fall back to the first card rather than stranding
  // edit/share/delete on none of them.
  if (!cards.some((card) => card.ownsPersonaActions)) {
    cards[0].ownsPersonaActions = true;
  }

  return cards;
}

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
    const personaAgents = byPersonaId.get(persona.id) ?? [];
    return {
      persona,
      agents: personaAgents,
      cards: buildPersonaCards(persona, personaAgents, isArchived),
    };
  });

  const unknown: ManagedAgent[] = [];
  for (const [id, list] of byPersonaId) {
    if (!matched.has(id)) {
      unknown.push(...list.filter((agent) => !isArchived(agent.pubkey)));
    }
  }

  return { groups, ungrouped, unknown };
}
