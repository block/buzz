import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import { isCommandTeamPersonaId } from "@/features/command-console/domain/commandTeam";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

export type PersonaGroup = {
  persona: AgentPersona;
  agents: ManagedAgent[];
};

export function buildUnifiedGroups(
  personas: AgentPersona[],
  agents: ManagedAgent[],
) {
  const byPersonaId = new Map<string, ManagedAgent[]>();
  const ungrouped: ManagedAgent[] = [];

  for (const agent of agents) {
    if (!agent.personaId) {
      ungrouped.push(agent);
    } else {
      const list = byPersonaId.get(agent.personaId) ?? [];
      list.push(agent);
      byPersonaId.set(agent.personaId, list);
    }
  }

  const matched = new Set<string>();
  const seenPersonas = new Set<string>();
  const personaGroups: PersonaGroup[] = personas.flatMap((persona) => {
    if (seenPersonas.has(persona.id)) return [];
    seenPersonas.add(persona.id);
    matched.add(persona.id);
    return [{ persona, agents: byPersonaId.get(persona.id) ?? [] }];
  });
  const commandTeamGroups = personaGroups.filter(({ persona }) =>
    isCommandTeamPersonaId(persona.id),
  );
  const groups = personaGroups.filter(
    ({ persona }) => !isCommandTeamPersonaId(persona.id),
  );

  const unknown: ManagedAgent[] = [];
  for (const [id, list] of byPersonaId) {
    if (!matched.has(id)) unknown.push(...list);
  }

  return { commandTeamGroups, groups, ungrouped, unknown };
}

export function pickProfileAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const activeDiff =
      Number(isManagedAgentActive(right)) - Number(isManagedAgentActive(left));
    if (activeDiff !== 0) return activeDiff;
    return left.name.localeCompare(right.name);
  })[0];
}
