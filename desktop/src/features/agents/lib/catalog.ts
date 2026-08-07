import type { AgentPersona } from "@/shared/api/types";

export function isPersonaActive(persona: AgentPersona) {
  return persona.isActive;
}

export function getActivePersonas(personas: readonly AgentPersona[]) {
  return personas.filter(isPersonaActive);
}

export function getLibraryPersonas(personas: readonly AgentPersona[]) {
  return getActivePersonas(personas);
}

export function isCatalogPersonaSelected(persona: AgentPersona) {
  return persona.isActive;
}

export function getPersonaLabelsById(personas: readonly AgentPersona[]) {
  return Object.fromEntries(
    personas.map((persona) => [persona.id, persona.displayName]),
  );
}
