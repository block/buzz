import type { AgentPersona, AgentTeam } from "@/shared/api/types";

export type ResolvedTeamPersonas = {
  hasMissingPersonas: boolean;
  isComplete: boolean;
  isUsable: boolean;
  missingPersonaCount: number;
  missingPersonaIds: string[];
  resolvedPersonaIds: string[];
  resolvedPersonas: AgentPersona[];
};

export function emptyResolvedTeamPersonas(): ResolvedTeamPersonas {
  return {
    hasMissingPersonas: false,
    isComplete: true,
    isUsable: false,
    missingPersonaCount: 0,
    missingPersonaIds: [],
    resolvedPersonaIds: [],
    resolvedPersonas: [],
  };
}

export function isResolvedTeamUsable(
  resolution: Pick<ResolvedTeamPersonas, "isComplete" | "resolvedPersonaIds">,
) {
  return resolution.isComplete && resolution.resolvedPersonaIds.length > 0;
}

export function getUsableTeams(
  teams: readonly AgentTeam[],
  personas: AgentPersona[],
) {
  return teams.filter((team) =>
    isResolvedTeamUsable(resolveTeamPersonas(team, personas)),
  );
}

export function resolveTeamPersonas(
  team: Pick<AgentTeam, "personaIds">,
  personas: AgentPersona[],
): ResolvedTeamPersonas {
  const personasById = new Map(
    personas.map((persona) => [persona.id, persona]),
  );
  const resolvedPersonas: AgentPersona[] = [];
  const resolvedPersonaIds: string[] = [];
  const missingPersonaIds: string[] = [];

  for (const personaId of team.personaIds) {
    const persona = personasById.get(personaId);

    if (persona) {
      resolvedPersonas.push(persona);
      resolvedPersonaIds.push(persona.id);
      continue;
    }

    missingPersonaIds.push(personaId);
  }

  const missingPersonaCount = missingPersonaIds.length;

  return {
    hasMissingPersonas: missingPersonaCount > 0,
    isComplete: missingPersonaCount === 0,
    isUsable: missingPersonaCount === 0 && resolvedPersonaIds.length > 0,
    missingPersonaCount,
    missingPersonaIds,
    resolvedPersonaIds,
    resolvedPersonas,
  };
}
