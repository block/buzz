import type { AgentPersona } from "@/shared/api/types";

function getAvailablePersonaIds(personas: AgentPersona[]): Set<string> {
  return new Set(personas.map((persona) => persona.id));
}

export function copySelectedPersonaIds(personaIds: string[]): string[] {
  return [...personaIds];
}

/**
 * Tells you if the submit button in the team dialog is enabled.
 *
 * A name is necessary. A member is not. A team with no members must be
 * savable, because you must empty a team before you can delete it.
 */
export function canSubmitTeamDialog({
  name,
  isPending,
}: {
  name: string;
  isPending: boolean;
}): boolean {
  return name.trim().length > 0 && !isPending;
}

export function countMissingPersonaIds(
  personaIds: string[],
  personas: AgentPersona[],
): number {
  const availablePersonaIds = getAvailablePersonaIds(personas);
  return personaIds.filter((personaId) => !availablePersonaIds.has(personaId))
    .length;
}

export function filterAvailablePersonaIds(
  personaIds: string[],
  personas: AgentPersona[],
): string[] {
  const availablePersonaIds = getAvailablePersonaIds(personas);
  return personaIds.filter((personaId) => availablePersonaIds.has(personaId));
}

export function orderPersonasByInitiallySelected(
  personas: AgentPersona[],
  initialSelectedPersonaIds: string[],
): AgentPersona[] {
  const selectedIds = new Set(initialSelectedPersonaIds);
  const selected: AgentPersona[] = [];
  const unselected: AgentPersona[] = [];

  for (const persona of personas) {
    if (selectedIds.has(persona.id)) {
      selected.push(persona);
    } else {
      unselected.push(persona);
    }
  }

  return [...selected, ...unselected];
}
