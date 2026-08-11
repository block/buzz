import { isManagedAgentActive } from "@/features/agents/lib/managedAgentControlActions";
import type { AgentPersona, ManagedAgent } from "@/shared/api/types";

type PersonaGroup = { persona: AgentPersona; agents: ManagedAgent[] };

const LOCAL_STARTER_PERSONA_IDS = new Set([
  "builtin:fizz",
  "builtin:honey",
  "builtin:bumble",
]);
const LOCAL_STARTER_NAMES = new Set(["Fizz", "Honey", "Bumble"]);

export function filterLocalStartersWhenCommunityHasAgents(
  personas: AgentPersona[],
  agents: ManagedAgent[],
) {
  const communityHasAgents = agents.some(
    (agent) => agent.pubkey.trim() && agent.relayUrl?.trim(),
  );
  if (!communityHasAgents) return { personas, agents };

  return {
    personas: personas.filter(
      (persona) => !LOCAL_STARTER_PERSONA_IDS.has(persona.id),
    ),
    agents: agents.filter(
      (agent) =>
        agent.pubkey.trim() ||
        (agent.personaId
          ? !LOCAL_STARTER_PERSONA_IDS.has(agent.personaId)
          : !LOCAL_STARTER_NAMES.has(agent.name)),
    ),
  };
}

export function buildUnifiedGroups(
  personas: AgentPersona[],
  agents: ManagedAgent[],
) {
  ({ personas, agents } = filterLocalStartersWhenCommunityHasAgents(
    personas,
    agents,
  ));
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
  const groups: PersonaGroup[] = personas.map((persona) => {
    matched.add(persona.id);
    return { persona, agents: byPersonaId.get(persona.id) ?? [] };
  });

  const unknown: ManagedAgent[] = [];
  for (const [id, list] of byPersonaId) {
    if (!matched.has(id)) unknown.push(...list);
  }

  return { groups, ungrouped, unknown };
}

export function pickProfileAgent(agents: ManagedAgent[]) {
  return [...agents].sort((left, right) => {
    const activeDiff =
      Number(isManagedAgentActive(right)) - Number(isManagedAgentActive(left));
    if (activeDiff !== 0) return activeDiff;
    return left.name.localeCompare(right.name);
  })[0];
}
