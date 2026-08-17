import {
  foldAgentDisplayName,
  groupAgentsForDisplay,
} from "@/features/agents/lib/agentIdentity";
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
  /**
   * Stable card key, also used for the card's React key and `data-testid`.
   *
   * Derived only from the persona and the display group, never from runtime
   * status or array position: a key that moves when an agent starts remounts
   * the card and refires its avatar query.
   */
  key: string;
  label: string;
  /**
   * Persona name, rendered as the card's second line, and only when the
   * persona has split into several cards — otherwise `label` already is the
   * persona name. Without it a split persona's own name appears nowhere in
   * the library, which is the same disappearing act this module exists to
   * prevent.
   */
  personaLabel: string | null;
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
    personaLabel: null,
    persona,
    agent: pickProfileAgent(members, isArchived),
    agents: members,
    ownsPersonaActions: true,
  });

  if (displayGroups.length <= 1) {
    return [personaOnlyCard(displayGroups[0]?.agents ?? [])];
  }

  const liveGroups = displayGroups.filter(
    (group) => pickProfileAgent(group.agents, isArchived) !== undefined,
  );
  if (liveGroups.length === 0) return [personaOnlyCard(agents)];

  const ownerIndex = pickPersonaActionsIndex(persona, liveGroups);
  return liveGroups.map((group, index) => ({
    // Plain `::` join rather than the length-prefixed form `agentDisplayGroupKey`
    // uses, because this key is also the card's `data-testid` and e2e specs read
    // it. That is safe only because the left segment cannot contain the
    // separator: a persona id is either a `slugify()` output (every
    // non-alphanumeric becomes `-`), a v4 UUID, or a `builtin:<name>` literal
    // with a single colon — never `::`. The right segment is free text, but a
    // forged separator there cannot shift the boundary when the left one is
    // constrained. Revisit if persona ids ever become user-supplied.
    key: `${persona.id}::${group.foldedName}`,
    label: group.name || persona.displayName,
    personaLabel: persona.displayName,
    persona,
    agent: pickProfileAgent(group.agents, isArchived),
    agents: group.agents,
    ownsPersonaActions: index === ownerIndex,
  }));
}

/**
 * Which split card carries the persona menu — Edit / Duplicate / Share /
 * Deactivate / Delete persona.
 *
 * Deliberately independent of runtime status. Deriving it from
 * `pickProfileAgent` (active-first) relocated the only route to editing or
 * deleting a persona whenever an instance started, leaving the card the owner
 * was looking at with no menu and no hint where it went. The card that still
 * carries the persona's own name is the natural home; if the owner renamed
 * every instance — or archiving retired the card that held the persona's own
 * name — the first surviving card gets it.
 */
function pickPersonaActionsIndex(
  persona: AgentPersona,
  displayGroups: readonly { foldedName: string }[],
): number {
  const personaName = foldAgentDisplayName(persona.displayName);
  const named = displayGroups.findIndex(
    (group) => group.foldedName === personaName,
  );
  return named === -1 ? 0 : named;
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
