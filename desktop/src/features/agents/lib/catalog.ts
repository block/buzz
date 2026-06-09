import type { AgentPersona } from "@/shared/api/types";

export type CatalogSelectionState = {
  catalogPersonas: AgentPersona[];
  selectedCatalogPersonas: AgentPersona[];
  unselectedCatalogPersonas: AgentPersona[];
};

export type PersonaLibraryState = {
  catalogPersonas: AgentPersona[];
  libraryPersonas: AgentPersona[];
  personaLabelsById: Record<string, string>;
};

export function isPersonaActive(persona: AgentPersona) {
  return persona.isActive;
}

export function getActivePersonas(personas: readonly AgentPersona[]) {
  return personas.filter(isPersonaActive);
}

export function getCatalogPersonas(personas: readonly AgentPersona[]) {
  return personas
    .filter((persona) => persona.isBuiltIn)
    .sort((left, right) => left.displayName.localeCompare(right.displayName));
}

export function isCatalogPersonaSelected(persona: AgentPersona) {
  return persona.isBuiltIn && persona.isActive;
}

export function getCatalogSelectionState(
  personas: readonly AgentPersona[],
): CatalogSelectionState {
  const catalogPersonas = getCatalogPersonas(personas);

  return {
    catalogPersonas,
    selectedCatalogPersonas: catalogPersonas.filter(isCatalogPersonaSelected),
    unselectedCatalogPersonas: catalogPersonas.filter(
      (persona) => !isCatalogPersonaSelected(persona),
    ),
  };
}

export function getPersonaLabelsById(personas: readonly AgentPersona[]) {
  return Object.fromEntries(
    personas.map((persona) => [persona.id, persona.displayName]),
  );
}

export function getPersonaLibraryState(
  personas: readonly AgentPersona[],
): PersonaLibraryState {
  const libraryPersonas = getActivePersonas(personas);
  const { catalogPersonas } = getCatalogSelectionState(personas);

  return {
    catalogPersonas,
    libraryPersonas,
    personaLabelsById: getPersonaLabelsById(personas),
  };
}
